use std::{collections::HashMap, sync::Arc};

use aegis_core::face::{FaceError, FaceTemplate, MODEL_ID};
use axum::{
    Extension, Json as ResponseJson, Router, extract::DefaultBodyLimit, http::StatusCode,
    middleware, routing::post,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::{AuthState, CurrentUser, origin_guard},
    config::Config,
    error::{ApiError, ApiResult, Json, Path},
    media::{self, FieldKind, FieldRule, MediaField, Multipart},
    models::{StageKind, time},
    policy::{
        self, AttemptOutcome, AttemptPersistence, LocationInput, ReasonCode, SubmissionWindow,
    },
    workers::{FaceWorkers, WorkerError},
};

const FACE_CHALLENGE_LIFETIME_MS: i64 = 60_000;

#[derive(Clone)]
pub struct AttendanceState {
    pub pool: SqlitePool,
    pub workers: FaceWorkers,
}

pub fn router(pool: SqlitePool, config: Arc<Config>, workers: FaceWorkers) -> Router {
    Router::new()
        .route("/api/face/challenges", post(issue_face_challenge))
        .route("/api/face/enroll", post(enroll_face))
        .route("/api/stages/{id}/attempts", post(submit_attempt))
        .layer(DefaultBodyLimit::max(media::MULTIPART_LIMIT))
        .layer(Extension(AttendanceState {
            pool: pool.clone(),
            workers,
        }))
        .layer(Extension(AuthState {
            pool,
            config: config.clone(),
        }))
        .layer(middleware::from_fn_with_state(config, origin_guard))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChallengePurpose {
    Enroll,
    Attendance,
}

impl ChallengePurpose {
    fn as_db(self) -> &'static str {
        match self {
            Self::Enroll => "enroll",
            Self::Attendance => "attendance",
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FaceChallengeRequest {
    pub purpose: ChallengePurpose,
    #[serde(default)]
    pub stage_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FaceChallenge {
    pub id: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FaceEnrollmentReceipt {
    pub enrolled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmittedLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_m: f64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptPayload {
    pub face_challenge_id: Uuid,
    #[serde(default)]
    pub location: Option<SubmittedLocation>,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct AttemptFactors {
    pub qr: Option<bool>,
    pub location: bool,
    pub face: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AttendanceAttempt {
    pub id: Uuid,
    pub stage_id: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub factors: AttemptFactors,
    pub is_late: bool,
    pub outcome: AttemptOutcome,
    pub reason_codes: Vec<ReasonCode>,
    pub review_id: Option<Uuid>,
}

#[derive(ToSchema)]
pub(crate) struct FaceEnrollmentMultipart {
    /// Face challenge UUID returned for this student.
    pub challenge_id: Uuid,
    /// JPEG or PNG image; maximum 2 MiB.
    #[schema(value_type = String, format = Binary)]
    pub frame_0: MediaField,
    /// JPEG or PNG image; maximum 2 MiB.
    #[schema(value_type = String, format = Binary)]
    pub frame_1: MediaField,
    /// JPEG or PNG image; maximum 2 MiB.
    #[schema(value_type = String, format = Binary)]
    pub frame_2: MediaField,
}

#[derive(ToSchema)]
pub(crate) struct AttendanceAttemptMultipart {
    /// JSON string containing face_challenge_id and optional location.
    #[schema(value_type = String)]
    pub payload: AttemptPayload,
    /// Optional JPEG or PNG face frame; maximum 2 MiB.
    #[schema(value_type = Option<String>, format = Binary)]
    pub frame_0: Option<MediaField>,
    /// Optional JPEG or PNG face frame; maximum 2 MiB.
    #[schema(value_type = Option<String>, format = Binary)]
    pub frame_1: Option<MediaField>,
    /// Optional JPEG or PNG face frame; maximum 2 MiB.
    #[schema(value_type = Option<String>, format = Binary)]
    pub frame_2: Option<MediaField>,
    /// Optional JPEG or PNG QR image; maximum 2 MiB; regular check_in only.
    #[schema(value_type = Option<String>, format = Binary)]
    pub qr_image: Option<MediaField>,
}

#[utoipa::path(
    post,
    path = "/api/face/challenges",
    operation_id = "attendance_issue_face_challenge",
    params(("Origin" = String, Header, description = "Must equal the configured application origin")),
    request_body = FaceChallengeRequest,
    responses(
        (status = 201, description = "A 60-second, single-use student face challenge", body = FaceChallenge),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Student role required or Origin rejected", body = ApiError),
        (status = 404, description = "Stage not found or not visible", body = ApiError),
        (status = 409, description = "Challenge purpose or submission window conflicts", body = ApiError),
        (status = 422, description = "Invalid request", body = ApiError)
    ),
    security(("session" = [])),
    tag = "attendance"
)]
pub(crate) async fn issue_face_challenge(
    Extension(state): Extension<AttendanceState>,
    user: CurrentUser,
    Json(input): Json<FaceChallengeRequest>,
) -> ApiResult<(StatusCode, ResponseJson<FaceChallenge>)> {
    user.student()?;
    match (input.purpose, input.stage_id) {
        (ChallengePurpose::Enroll, Some(_)) => {
            return Err(ApiError::invalid(
                "Enrollment challenges cannot name a stage",
            ));
        }
        (ChallengePurpose::Attendance, None) => {
            return Err(ApiError::invalid(
                "Attendance challenges require a stage_id",
            ));
        }
        _ => {}
    }

    let now = Utc::now().timestamp_millis();
    let expires_at = now
        .checked_add(FACE_CHALLENGE_LIFETIME_MS)
        .ok_or_else(ApiError::internal)?;
    let challenge_id = Uuid::new_v4();
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    match input.purpose {
        ChallengePurpose::Enroll => {}
        ChallengePurpose::Attendance => {
            let stage_id = input
                .stage_id
                .ok_or_else(|| ApiError::invalid("Attendance challenges require a stage_id"))?;
            let stage = stage_context(&mut tx, stage_id, &user).await?;
            let kind = stage.kind_value()?;
            if policy::submission_window(
                kind,
                stage.opens_at,
                stage.closes_at,
                stage.closed_at,
                stage.lesson_starts_at,
                stage.lesson_ends_at,
                stage.lesson_closed_at.is_some(),
                now,
            )
            .is_none()
            {
                return Err(ApiError::window_closed());
            }
        }
    }
    sqlx::query(
        "INSERT INTO face_challenges (id, user_id, stage_id, purpose, expires_at, consumed_at) VALUES (?, ?, ?, ?, ?, NULL)",
    )
    .bind(challenge_id.to_string())
    .bind(user.user.id.to_string())
    .bind(input.stage_id.map(|id| id.to_string()))
    .bind(input.purpose.as_db())
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok((
        StatusCode::CREATED,
        ResponseJson(FaceChallenge {
            id: challenge_id,
            expires_at: time(expires_at)?,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/api/face/enroll",
    operation_id = "attendance_enroll_face",
    params(("Origin" = String, Header, description = "Must equal the configured application origin")),
    request_body(content = FaceEnrollmentMultipart, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Face template enrolled; biometric data is never returned", body = FaceEnrollmentReceipt),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Student role required or Origin rejected", body = ApiError),
        (status = 409, description = "Challenge expired, consumed, mismatched, or face already enrolled", body = ApiError),
        (status = 413, description = "Multipart body exceeds limit", body = ApiError),
        (status = 422, description = "Invalid multipart media or face enrollment factors", body = ApiError),
        (status = 503, description = "Local face inference unavailable", body = ApiError)
    ),
    security(("session" = [])),
    tag = "attendance"
)]
pub(crate) async fn enroll_face(
    Extension(state): Extension<AttendanceState>,
    user: CurrentUser,
    multipart: Multipart,
) -> ApiResult<(StatusCode, ResponseJson<FaceEnrollmentReceipt>)> {
    user.student()?;
    let mut fields = media::receive(
        multipart,
        &[
            FieldRule {
                name: "challenge_id",
                kind: FieldKind::Text,
            },
            FieldRule {
                name: "frame_0",
                kind: FieldKind::Image,
            },
            FieldRule {
                name: "frame_1",
                kind: FieldKind::Image,
            },
            FieldRule {
                name: "frame_2",
                kind: FieldKind::Image,
            },
        ],
    )
    .await?;
    let received_at = Utc::now().timestamp_millis();
    let challenge_id = parse_challenge_field(&mut fields, "challenge_id")?;
    let [frame_0, frame_1, frame_2] = take_required_frames(&mut fields)?;
    let input = FaceEnrollmentMultipart {
        challenge_id,
        frame_0,
        frame_1,
        frame_2,
    };
    consume_enrollment_challenge(&state.pool, &user, input.challenge_id, received_at).await?;
    let frames = decode_frames([input.frame_0, input.frame_1, input.frame_2]).await?;
    let template = state.workers.enroll(frames).await.map_err(ApiError::from)?;
    if template.model_id != MODEL_ID || !template.embedding.iter().all(|value| value.is_finite()) {
        return Err(ApiError::factor_unavailable());
    }
    let mut embedding = [0_u8; 128 * 4];
    for (chunk, value) in embedding.chunks_exact_mut(4).zip(template.embedding) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    sqlx::query(
        "INSERT INTO face_templates (user_id, embedding, model_id, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(user.user.id.to_string())
    .bind(embedding.as_slice())
    .bind(template.model_id)
    .bind(Utc::now().timestamp_millis())
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if let sqlx::Error::Database(database) = &error {
            if database.is_unique_violation() {
                return ApiError::conflict("Face is already enrolled");
            }
        }
        ApiError::from(error)
    })?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok((
        StatusCode::CREATED,
        ResponseJson(FaceEnrollmentReceipt { enrolled: true }),
    ))
}

#[utoipa::path(
    post,
    path = "/api/stages/{id}/attempts",
    operation_id = "attendance_submit_attempt",
    params(
        ("id" = Uuid, Path, description = "Attendance stage identifier"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    request_body(content = AttendanceAttemptMultipart, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Persisted factor attempt", body = AttendanceAttempt),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Student role required or Origin rejected", body = ApiError),
        (status = 404, description = "Stage not found or student not in its frozen roster", body = ApiError),
        (status = 409, description = "Challenge already consumed, expired, mismatched, or window closed", body = ApiError),
        (status = 413, description = "Multipart body exceeds limit", body = ApiError),
        (status = 422, description = "Invalid multipart protocol or media", body = ApiError),
        (status = 503, description = "Factor inference unavailable or worker capacity exhausted", body = ApiError)
    ),
    security(("session" = [])),
    tag = "attendance"
)]
pub(crate) async fn submit_attempt(
    Extension(state): Extension<AttendanceState>,
    user: CurrentUser,
    Path(stage_id): Path<Uuid>,
    multipart: Multipart,
) -> ApiResult<(StatusCode, ResponseJson<AttendanceAttempt>)> {
    user.student()?;
    let mut fields = media::receive(
        multipart,
        &[
            FieldRule {
                name: "payload",
                kind: FieldKind::Text,
            },
            FieldRule {
                name: "frame_0",
                kind: FieldKind::Image,
            },
            FieldRule {
                name: "frame_1",
                kind: FieldKind::Image,
            },
            FieldRule {
                name: "frame_2",
                kind: FieldKind::Image,
            },
            FieldRule {
                name: "qr_image",
                kind: FieldKind::Image,
            },
        ],
    )
    .await?;
    let received_at = Utc::now().timestamp_millis();
    let input = AttendanceAttemptMultipart {
        payload: parse_attempt_payload(&mut fields)?,
        qr_image: fields.remove("qr_image"),
        frame_0: fields.remove("frame_0"),
        frame_1: fields.remove("frame_1"),
        frame_2: fields.remove("frame_2"),
    };
    let payload = input.payload;
    let qr_media = input.qr_image;
    let media_frames = [input.frame_0, input.frame_1, input.frame_2];
    let (initial_stage, window) = consume_attendance_challenge(
        &state.pool,
        &user,
        stage_id,
        payload.face_challenge_id,
        qr_media.is_some(),
        received_at,
    )
    .await?;
    let initial_kind = initial_stage.kind_value()?;
    let mut reason_codes = Vec::with_capacity(4);
    let qr_image = match qr_media {
        Some(field) => Some(media::decode_image(field).await?),
        None => None,
    };
    let media_frames = decode_optional_frames(media_frames).await?;
    let qr_pass = if initial_kind == StageKind::CheckIn && window == SubmissionWindow::Regular {
        match qr_image {
            None => {
                reason_codes.push(ReasonCode::QrMissing);
                Some(false)
            }
            Some(image) => match state
                .workers
                .decode_qr(image)
                .await
                .map_err(ApiError::from)?
            {
                Ok(token) => match qr_factor(&state.pool, stage_id, &token, received_at).await? {
                    Ok(()) => Some(true),
                    Err(reason) => {
                        reason_codes.push(reason);
                        Some(false)
                    }
                },
                Err(_) => {
                    reason_codes.push(ReasonCode::QrUnreadable);
                    Some(false)
                }
            },
        }
    } else {
        None
    };

    let location_failure = policy::location_failure(
        payload.location.map(|location| LocationInput {
            latitude: location.latitude,
            longitude: location.longitude,
            accuracy_m: location.accuracy_m,
        }),
        initial_stage.latitude,
        initial_stage.longitude,
        initial_stage.radius_m,
    );
    if let Some(reason) = location_failure {
        reason_codes.push(reason);
    }
    let location_pass = location_failure.is_none();

    let face_pass = verify_face(
        &state.pool,
        &state.workers,
        &user,
        media_frames,
        &mut reason_codes,
    )
    .await?;
    let outcome = policy::attempt_outcome(initial_kind, window, qr_pass, location_pass, face_pass);
    let attempt_id = Uuid::new_v4();
    let is_late = window == SubmissionWindow::Late;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    let final_stage = stage_context(&mut tx, stage_id, &user).await?;
    if Utc::now().timestamp_millis() >= final_stage.lesson_ends_at {
        return Err(ApiError::window_closed());
    }
    let final_kind = final_stage.kind_value()?;
    let final_window = policy::submission_window(
        final_kind,
        final_stage.opens_at,
        final_stage.closes_at,
        final_stage.closed_at,
        final_stage.lesson_starts_at,
        final_stage.lesson_ends_at,
        final_stage.lesson_closed_at.is_some(),
        received_at,
    )
    .ok_or_else(ApiError::window_closed)?;
    if final_window != window || final_kind != initial_kind {
        return Err(ApiError::window_closed());
    }
    let attempt_id_text = attempt_id.to_string();
    let stage_id_text = stage_id.to_string();
    let user_id_text = user.user.id.to_string();
    policy::apply_attempt_result(
        &mut tx,
        AttemptPersistence {
            attempt_id: &attempt_id_text,
            stage_id: &stage_id_text,
            user_id: &user_id_text,
            submitted_at: received_at,
            qr_pass,
            location_pass,
            face_pass,
            is_late,
            outcome,
            reason_codes: &reason_codes,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok((
        StatusCode::CREATED,
        ResponseJson(AttendanceAttempt {
            id: attempt_id,
            stage_id,
            submitted_at: time(received_at)?,
            factors: AttemptFactors {
                qr: qr_pass,
                location: location_pass,
                face: face_pass,
            },
            is_late,
            outcome,
            reason_codes,
            review_id: None,
        }),
    ))
}

#[derive(Clone, FromRow)]
struct StageContext {
    kind: String,
    opens_at: i64,
    closes_at: i64,
    closed_at: Option<i64>,
    latitude: f64,
    longitude: f64,
    radius_m: f64,
    lesson_starts_at: i64,
    lesson_ends_at: i64,
    lesson_closed_at: Option<i64>,
}

impl StageContext {
    fn kind_value(&self) -> ApiResult<StageKind> {
        StageKind::parse(&self.kind)
    }
}

async fn stage_context(
    transaction: &mut Transaction<'_, Sqlite>,
    stage_id: Uuid,
    user: &CurrentUser,
) -> ApiResult<StageContext> {
    let row = sqlx::query_as::<_, StageContext>(
        "SELECT s.kind, s.opens_at, s.closes_at, s.closed_at, s.latitude, s.longitude, s.radius_m, \
                l.starts_at AS lesson_starts_at, l.ends_at AS lesson_ends_at, l.closed_at AS lesson_closed_at \
         FROM stages AS s JOIN lessons AS l ON l.id = s.lesson_id \
         WHERE s.id = ? AND EXISTS (SELECT 1 FROM lesson_students AS ls WHERE ls.lesson_id = l.id AND ls.user_id = ?)",
    )
    .bind(stage_id.to_string())
    .bind(user.user.id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::from)?
    .ok_or_else(ApiError::not_found)?;
    Ok(row)
}

async fn consume_attendance_challenge(
    pool: &SqlitePool,
    user: &CurrentUser,
    stage_id: Uuid,
    challenge_id: Uuid,
    has_qr_image: bool,
    received_at: i64,
) -> ApiResult<(StageContext, SubmissionWindow)> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    let stage = stage_context(&mut tx, stage_id, user).await?;
    let kind = stage.kind_value()?;
    let window = policy::submission_window(
        kind,
        stage.opens_at,
        stage.closes_at,
        stage.closed_at,
        stage.lesson_starts_at,
        stage.lesson_ends_at,
        stage.lesson_closed_at.is_some(),
        received_at,
    )
    .ok_or_else(ApiError::window_closed)?;
    if has_qr_image && (kind != StageKind::CheckIn || window != SubmissionWindow::Regular) {
        return Err(ApiError::invalid(
            "qr_image is allowed only for a regular check_in attempt",
        ));
    }
    consume_challenge(
        &mut tx,
        user.user.id,
        challenge_id,
        "attendance",
        Some(stage_id),
        received_at,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok((stage, window))
}

async fn consume_enrollment_challenge(
    pool: &SqlitePool,
    user: &CurrentUser,
    challenge_id: Uuid,
    received_at: i64,
) -> ApiResult<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    let enrolled = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM face_templates WHERE user_id = ?)",
    )
    .bind(user.user.id.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from)?
        != 0;
    if enrolled {
        return Err(ApiError::conflict("Face is already enrolled"));
    }
    consume_challenge(
        &mut tx,
        user.user.id,
        challenge_id,
        "enroll",
        None,
        received_at,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)
}

#[derive(FromRow)]
struct FaceChallengeRow {
    user_id: String,
    stage_id: Option<String>,
    purpose: String,
    expires_at: i64,
    consumed_at: Option<i64>,
}

pub async fn consume_challenge(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: Uuid,
    challenge_id: Uuid,
    purpose: &'static str,
    stage_id: Option<Uuid>,
    received_at: i64,
) -> ApiResult<()> {
    let row = sqlx::query_as::<_, FaceChallengeRow>(
        "SELECT user_id, stage_id, purpose, expires_at, consumed_at FROM face_challenges WHERE id = ?",
    )
    .bind(challenge_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::from)?
    .ok_or_else(|| ApiError::conflict("Face challenge is invalid, expired, or already consumed"))?;
    let expected_stage_id = stage_id.map(|id| id.to_string());
    if row.user_id != user_id.to_string()
        || row.purpose != purpose
        || row.stage_id != expected_stage_id
        || row.consumed_at.is_some()
        || received_at >= row.expires_at
    {
        return Err(ApiError::conflict(
            "Face challenge is invalid, expired, or already consumed",
        ));
    }
    let result = sqlx::query(
        "UPDATE face_challenges SET consumed_at = ? WHERE id = ? AND consumed_at IS NULL AND expires_at > ?",
    )
    .bind(received_at)
    .bind(challenge_id.to_string())
    .bind(received_at)
    .execute(&mut **transaction)
    .await
    .map_err(ApiError::from)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Face challenge is invalid, expired, or already consumed",
        ));
    }
    Ok(())
}

async fn qr_factor(
    pool: &SqlitePool,
    stage_id: Uuid,
    token: &str,
    received_at: i64,
) -> ApiResult<Result<(), ReasonCode>> {
    let token_hash = Sha256::digest(token.as_bytes());
    let row = sqlx::query_as::<_, (String, i64)>(
        "SELECT stage_id, expires_at FROM qr_challenges WHERE token_hash = ?",
    )
    .bind(token_hash.as_slice())
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)?;
    let Some((stored_stage, expires_at)) = row else {
        return Ok(Err(ReasonCode::QrWrongStage));
    };
    if stored_stage != stage_id.to_string() {
        return Ok(Err(ReasonCode::QrWrongStage));
    }
    if !policy::qr_token_valid_at(expires_at, received_at) {
        return Ok(Err(ReasonCode::QrExpired));
    }
    Ok(Ok(()))
}

async fn verify_face(
    pool: &SqlitePool,
    workers: &FaceWorkers,
    user: &CurrentUser,
    frames: [Option<image::RgbImage>; 3],
    reasons: &mut Vec<ReasonCode>,
) -> ApiResult<bool> {
    let has_all_frames = frames.iter().all(Option::is_some);
    let row = sqlx::query_as::<_, (Vec<u8>, String)>(
        "SELECT embedding, model_id FROM face_templates WHERE user_id = ?",
    )
    .bind(user.user.id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)?;
    let Some((embedding_bytes, model_id)) = row else {
        reasons.push(ReasonCode::FaceUnenrolled);
        if !has_all_frames {
            reasons.push(ReasonCode::FaceMissing);
        }
        return Ok(false);
    };
    if model_id != MODEL_ID {
        return Err(ApiError::factor_unavailable());
    }
    if embedding_bytes.len() != 512 {
        return Err(ApiError::factor_unavailable());
    }
    let mut embedding = [0_f32; 128];
    for (value, bytes) in embedding.iter_mut().zip(embedding_bytes.chunks_exact(4)) {
        *value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    let norm_squared = embedding
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    if embedding.iter().any(|value| !value.is_finite()) || (norm_squared - 1.0).abs() > 1.0e-3 {
        return Err(ApiError::factor_unavailable());
    }
    let [Some(frame_0), Some(frame_1), Some(frame_2)] = frames else {
        reasons.push(ReasonCode::FaceMissing);
        return Ok(false);
    };
    let template = FaceTemplate {
        embedding,
        model_id,
    };
    match workers.verify([frame_0, frame_1, frame_2], template).await {
        Ok(()) => Ok(true),
        Err(WorkerError::Face(FaceError::NotSingleFace)) => {
            reasons.push(ReasonCode::FaceNotSingle);
            Ok(false)
        }
        Err(WorkerError::Face(FaceError::Spoof)) => {
            reasons.push(ReasonCode::FaceSpoof);
            Ok(false)
        }
        Err(WorkerError::Face(FaceError::IdentityMismatch)) => {
            reasons.push(ReasonCode::FaceMismatch);
            Ok(false)
        }
        Err(error) => Err(ApiError::from(error)),
    }
}

fn parse_challenge_field(fields: &mut HashMap<String, MediaField>, name: &str) -> ApiResult<Uuid> {
    let field = fields
        .remove(name)
        .ok_or_else(|| ApiError::invalid("Missing challenge_id"))?;
    Uuid::parse_str(field.text()?).map_err(|_| ApiError::invalid("Invalid challenge_id"))
}

fn parse_attempt_payload(fields: &mut HashMap<String, MediaField>) -> ApiResult<AttemptPayload> {
    let field = fields
        .remove("payload")
        .ok_or_else(|| ApiError::invalid("Missing payload"))?;
    serde_json::from_str(field.text()?).map_err(|_| ApiError::invalid("Invalid attendance payload"))
}

fn take_required_frames(fields: &mut HashMap<String, MediaField>) -> ApiResult<[MediaField; 3]> {
    let frames = ["frame_0", "frame_1", "frame_2"].map(|name| fields.remove(name));
    match frames {
        [Some(first), Some(second), Some(third)] => Ok([first, second, third]),
        _ => Err(ApiError::invalid(
            "Enrollment requires frame_0, frame_1, and frame_2",
        )),
    }
}

async fn decode_frames(media_frames: [MediaField; 3]) -> ApiResult<[image::RgbImage; 3]> {
    let [frame_0, frame_1, frame_2] = media_frames;
    Ok([
        media::decode_image(frame_0).await?,
        media::decode_image(frame_1).await?,
        media::decode_image(frame_2).await?,
    ])
}

async fn decode_optional_frames(
    media_frames: [Option<MediaField>; 3],
) -> ApiResult<[Option<image::RgbImage>; 3]> {
    let [frame_0, frame_1, frame_2] = media_frames;
    Ok([
        match frame_0 {
            Some(field) => Some(media::decode_image(field).await?),
            None => None,
        },
        match frame_1 {
            Some(field) => Some(media::decode_image(field).await?),
            None => None,
        },
        match frame_2 {
            Some(field) => Some(media::decode_image(field).await?),
            None => None,
        },
    ])
}
