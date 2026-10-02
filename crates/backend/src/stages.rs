use std::io::Cursor;

use axum::{
    Router,
    body::Body,
    extract::Extension,
    http::{HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{SecondsFormat, Utc};
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Sqlite, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    courses::CoursesState,
    error::{ApiError, ApiResult, Json, Path},
    models::{Lesson, LessonRow, Stage, StageKind, StageRow},
};

const QR_SLOT_MS: i64 = 5_000;

pub(crate) fn routes() -> Router {
    Router::new()
        .route("/api/lessons/{id}/stages", post(create_stage))
        .route("/api/stages/{id}/close", post(close_stage))
        .route("/api/lessons/{id}/close", post(close_lesson))
        .route("/api/stages/{id}/qr", get(get_stage_qr))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateStageRequest {
    pub kind: StageKind,
    #[schema(minimum = 1, maximum = 900)]
    pub duration_seconds: i64,
    #[schema(minimum = -90.0, maximum = 90.0)]
    pub latitude: f64,
    #[schema(minimum = -180.0, maximum = 180.0)]
    pub longitude: f64,
    #[schema(minimum = 1.0, maximum = 1000.0)]
    pub radius_m: f64,
}

#[derive(FromRow)]
struct LessonWindowRow {
    closed_at: Option<i64>,
    ends_at: i64,
}

#[utoipa::path(
    post,
    path = "/api/lessons/{id}/stages",
    operation_id = "stages_create_lesson_stage",
    params(("id" = Uuid, Path, description = "Lesson identifier"), ("Origin" = String, Header, description = "Must match AEGIS_ORIGIN")),
    request_body = CreateStageRequest,
    responses(
        (status = 201, description = "Opened attendance stage", body = Stage),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 404, description = "Lesson not found", body = ApiError),
        (status = 409, description = "Stage ordering or lesson window conflict", body = ApiError),
        (status = 422, description = "Invalid stage parameters", body = ApiError)
    ),
    security(("session" = [])),
    tag = "stages"
)]
pub(crate) async fn create_stage(
    Extension(state): Extension<CoursesState>,
    user: CurrentUser,
    Path(lesson_id): Path<Uuid>,
    Json(input): Json<CreateStageRequest>,
) -> ApiResult<(StatusCode, axum::Json<Stage>)> {
    user.teacher()?;
    validate_stage_request(&input)?;

    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    let lesson = owned_lesson(&mut transaction, &user, lesson_id).await?;
    let now = Utc::now().timestamp_millis();
    if lesson.closed_at.is_some() || now < lesson.starts_at || now >= lesson.ends_at {
        return Err(ApiError::window_closed());
    }

    let existing_check_in = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM stages WHERE lesson_id = ? AND kind = 'check_in')",
    )
    .bind(lesson_id.to_string())
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::from)?
        != 0;

    if !existing_check_in {
        if input.kind != StageKind::CheckIn {
            return Err(ApiError::conflict("The first stage must be check_in"));
        }
        let roster_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM enrollments AS e JOIN users AS u ON u.id = e.user_id \
             WHERE e.course_id = ? AND u.role = 'student'",
        )
        .bind(&lesson.course_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
        if roster_count == 0 {
            return Err(ApiError::conflict(
                "A lesson needs a nonempty student roster",
            ));
        }
    } else if input.kind == StageKind::CheckIn {
        return Err(ApiError::conflict(
            "A lesson can have only one check_in stage",
        ));
    }

    let stage_after_check_in = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM stages WHERE lesson_id = ? AND kind = 'check_out')",
    )
    .bind(lesson_id.to_string())
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::from)?
        != 0;
    if stage_after_check_in {
        return Err(ApiError::conflict("No stages may follow check_out"));
    }

    let active_stage = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM stages WHERE lesson_id = ? AND closed_at IS NULL AND closes_at > ?)",
    )
    .bind(lesson_id.to_string())
    .bind(now)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::from)? != 0;
    if active_stage {
        return Err(ApiError::conflict(
            "The current stage window must end before opening another",
        ));
    }

    let ordinal =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(ordinal) FROM stages WHERE lesson_id = ?")
            .bind(lesson_id.to_string())
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::from)?
            .map_or(Ok(0), |value| {
                value.checked_add(1).ok_or_else(ApiError::internal)
            })?;
    let requested_end = now
        .checked_add(
            input
                .duration_seconds
                .checked_mul(1_000)
                .ok_or_else(ApiError::internal)?,
        )
        .ok_or_else(ApiError::internal)?;
    let closes_at = requested_end.min(lesson.ends_at);
    let stage_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO stages (id, lesson_id, ordinal, kind, opens_at, closes_at, closed_at, latitude, longitude, radius_m) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
    )
    .bind(stage_id.to_string())
    .bind(lesson_id.to_string())
    .bind(ordinal)
    .bind(input.kind.as_db())
    .bind(now)
    .bind(closes_at)
    .bind(input.latitude)
    .bind(input.longitude)
    .bind(input.radius_m)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::from)?;

    if !existing_check_in {
        sqlx::query(
            "INSERT INTO lesson_students (lesson_id, user_id) \
             SELECT ?, e.user_id FROM enrollments AS e JOIN users AS u ON u.id = e.user_id \
             WHERE e.course_id = ? AND u.role = 'student'",
        )
        .bind(lesson_id.to_string())
        .bind(&lesson.course_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
    }

    let row = stage_by_id(&mut transaction, stage_id).await?;
    transaction.commit().await.map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, axum::Json(row.dto()?)))
}

#[utoipa::path(
    post,
    path = "/api/stages/{id}/close",
    operation_id = "stages_close_stage",
    params(("id" = Uuid, Path, description = "Stage identifier"), ("Origin" = String, Header, description = "Must match AEGIS_ORIGIN")),
    responses(
        (status = 200, description = "Current stage, closed early when still open", body = Stage),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 404, description = "Stage not found", body = ApiError)
    ),
    security(("session" = [])),
    tag = "stages"
)]
pub(crate) async fn close_stage(
    Extension(state): Extension<CoursesState>,
    user: CurrentUser,
    Path(stage_id): Path<Uuid>,
) -> ApiResult<axum::Json<Stage>> {
    user.teacher()?;
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    let stage = owned_stage(&mut transaction, &user, stage_id).await?;
    let now = Utc::now().timestamp_millis();
    if stage.closed_at.is_none() && now <= stage.closes_at && now >= stage.opens_at {
        sqlx::query(
            "UPDATE stages SET closes_at = MIN(closes_at, ?), closed_at = MIN(closes_at, ?) \
             WHERE id = ? AND closed_at IS NULL AND closes_at >= ? AND opens_at <= ?",
        )
        .bind(now)
        .bind(now)
        .bind(stage_id.to_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
    }
    let row = stage_by_id(&mut transaction, stage_id).await?;
    transaction.commit().await.map_err(ApiError::from)?;
    Ok(axum::Json(row.dto()?))
}

#[utoipa::path(
    post,
    path = "/api/lessons/{id}/close",
    operation_id = "stages_close_lesson",
    params(("id" = Uuid, Path, description = "Lesson identifier"), ("Origin" = String, Header, description = "Must match AEGIS_ORIGIN")),
    responses(
        (status = 200, description = "Closed lesson", body = Lesson),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 404, description = "Lesson not found", body = ApiError),
        (status = 409, description = "A check_in stage is required", body = ApiError)
    ),
    security(("session" = [])),
    tag = "stages"
)]
pub(crate) async fn close_lesson(
    Extension(state): Extension<CoursesState>,
    user: CurrentUser,
    Path(lesson_id): Path<Uuid>,
) -> ApiResult<axum::Json<Lesson>> {
    user.teacher()?;
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    let lesson = owned_lesson(&mut transaction, &user, lesson_id).await?;
    let has_check_in = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM stages WHERE lesson_id = ? AND kind = 'check_in')",
    )
    .bind(lesson_id.to_string())
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::from)?
        != 0;
    if !has_check_in {
        return Err(ApiError::conflict(
            "A check_in stage is required before closing the lesson",
        ));
    }

    let now = Utc::now().timestamp_millis();
    if lesson.closed_at.is_none() {
        sqlx::query("UPDATE lessons SET closed_at = ? WHERE id = ? AND closed_at IS NULL")
            .bind(now)
            .bind(lesson_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::from)?;
        sqlx::query(
            "UPDATE stages SET closes_at = MIN(closes_at, ?), closed_at = MIN(closes_at, ?) \
             WHERE lesson_id = ? AND closed_at IS NULL AND opens_at <= ? AND closes_at > ?",
        )
        .bind(now)
        .bind(now)
        .bind(lesson_id.to_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
    }

    let row = sqlx::query_as::<_, LessonRow>("SELECT * FROM lessons WHERE id = ?")
        .bind(lesson_id.to_string())
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
    transaction.commit().await.map_err(ApiError::from)?;
    Ok(axum::Json(row.dto()?))
}

#[utoipa::path(
    get,
    path = "/api/stages/{id}/qr",
    operation_id = "stages_get_stage_qr",
    params(("id" = Uuid, Path, description = "Stage identifier")),
    responses(
        (status = 200, description = "Current check_in QR image; never cached", body = [u8], content_type = "image/png", headers(("Cache-Control" = String, description = "no-store"), ("X-QR-Expires-At" = String, description = "RFC3339 expiration time"))),
        (status = 403, description = "Only the owning teacher can retrieve a QR image", body = ApiError),
        (status = 404, description = "Stage not found", body = ApiError),
        (status = 409, description = "QR is unavailable outside an open check_in window", body = ApiError)
    ),
    security(("session" = [])),
    tag = "stages"
)]
pub(crate) async fn get_stage_qr(
    Extension(state): Extension<CoursesState>,
    user: CurrentUser,
    Path(stage_id): Path<Uuid>,
) -> ApiResult<Response> {
    user.teacher()?;
    let (slot, token, expires_at) = {
        let mut transaction = state
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(ApiError::from)?;
        let stage = owned_stage(&mut transaction, &user, stage_id).await?;
        if stage.kind != StageKind::CheckIn.as_db() {
            return Err(ApiError::conflict(
                "QR images are available only for check_in stages",
            ));
        }
        let lesson_window = lesson_window(&mut transaction, &stage.lesson_id).await?;
        let now = Utc::now().timestamp_millis();
        if stage.closed_at.is_some()
            || lesson_window.closed_at.is_some()
            || now < stage.opens_at
            || now >= stage.closes_at
            || now >= lesson_window.ends_at
        {
            return Err(ApiError::window_closed());
        }

        let slot = now
            .checked_sub(stage.opens_at)
            .ok_or_else(ApiError::internal)?
            / QR_SLOT_MS;
        let slot_end = stage
            .opens_at
            .checked_add(
                slot.checked_add(1)
                    .and_then(|n| n.checked_mul(QR_SLOT_MS))
                    .ok_or_else(ApiError::internal)?,
            )
            .ok_or_else(ApiError::internal)?;
        let expires_at = slot_end.min(stage.closes_at);
        let token = random_qr_token()?;
        let token_hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query(
            "INSERT INTO qr_challenges (stage_id, slot, token_hash, token, expires_at) \
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(stage_id, slot) DO NOTHING",
        )
        .bind(stage_id.to_string())
        .bind(slot)
        .bind(token_hash)
        .bind(token)
        .bind(expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
        let (stored_token, stored_expiry) = sqlx::query_as::<_, (String, i64)>(
            "SELECT token, expires_at FROM qr_challenges WHERE stage_id = ? AND slot = ?",
        )
        .bind(stage_id.to_string())
        .bind(slot)
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
        transaction.commit().await.map_err(ApiError::from)?;
        (slot, stored_token, stored_expiry)
    };

    let encode_token = token.clone();
    let png = tokio::task::spawn_blocking(move || -> ApiResult<Vec<u8>> {
        let carrier =
            aegis_core::qr::encode_chroma(&encode_token).map_err(|_| ApiError::internal())?;
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(carrier)
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|_| ApiError::internal())?;
        Ok(output.into_inner())
    })
    .await
    .map_err(|_| ApiError::internal())??;

    let revalidation_now = Utc::now().timestamp_millis();
    let still_valid = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS( \
            SELECT 1 FROM qr_challenges AS q \
            JOIN stages AS s ON s.id = q.stage_id \
            JOIN lessons AS l ON l.id = s.lesson_id \
            JOIN courses AS c ON c.id = l.course_id \
            WHERE q.stage_id = ? AND q.slot = ? AND q.token = ? AND q.expires_at = ? \
              AND s.kind = 'check_in' AND s.closed_at IS NULL AND l.closed_at IS NULL \
              AND c.teacher_id = ? AND s.opens_at <= ? AND ? < s.closes_at \
              AND ? < l.ends_at AND ? < q.expires_at \
         )",
    )
    .bind(stage_id.to_string())
    .bind(slot)
    .bind(&token)
    .bind(expires_at)
    .bind(user.user.id.to_string())
    .bind(revalidation_now)
    .bind(revalidation_now)
    .bind(revalidation_now)
    .bind(revalidation_now)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::from)?
        != 0;
    if !still_valid {
        return Err(ApiError::window_closed());
    }

    let expiry = chrono::DateTime::from_timestamp_millis(expires_at)
        .ok_or_else(ApiError::internal)?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut response = Response::new(Body::from(png));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "X-QR-Expires-At",
        HeaderValue::from_str(&expiry).map_err(|_| ApiError::internal())?,
    );
    Ok(response)
}

fn validate_stage_request(input: &CreateStageRequest) -> ApiResult<()> {
    if !(1..=900).contains(&input.duration_seconds) {
        return Err(ApiError::invalid(
            "duration_seconds must be between 1 and 900",
        ));
    }
    if !input.latitude.is_finite() || !(-90.0..=90.0).contains(&input.latitude) {
        return Err(ApiError::invalid(
            "latitude must be finite and between -90 and 90",
        ));
    }
    if !input.longitude.is_finite() || !(-180.0..=180.0).contains(&input.longitude) {
        return Err(ApiError::invalid(
            "longitude must be finite and between -180 and 180",
        ));
    }
    if !input.radius_m.is_finite() || !(1.0..=1_000.0).contains(&input.radius_m) {
        return Err(ApiError::invalid(
            "radius_m must be finite and between 1 and 1000",
        ));
    }
    Ok(())
}

async fn owned_lesson(
    transaction: &mut Transaction<'_, Sqlite>,
    user: &CurrentUser,
    lesson_id: Uuid,
) -> ApiResult<LessonRow> {
    sqlx::query_as::<_, LessonRow>(
        "SELECT l.* FROM lessons AS l JOIN courses AS c ON c.id = l.course_id \
         WHERE l.id = ? AND c.teacher_id = ?",
    )
    .bind(lesson_id.to_string())
    .bind(user.user.id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::from)?
    .ok_or_else(ApiError::not_found)
}

async fn owned_stage(
    transaction: &mut Transaction<'_, Sqlite>,
    user: &CurrentUser,
    stage_id: Uuid,
) -> ApiResult<StageRow> {
    sqlx::query_as::<_, StageRow>(
        "SELECT s.* FROM stages AS s \
         JOIN lessons AS l ON l.id = s.lesson_id \
         JOIN courses AS c ON c.id = l.course_id \
         WHERE s.id = ? AND c.teacher_id = ?",
    )
    .bind(stage_id.to_string())
    .bind(user.user.id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::from)?
    .ok_or_else(ApiError::not_found)
}

async fn stage_by_id(
    transaction: &mut Transaction<'_, Sqlite>,
    stage_id: Uuid,
) -> ApiResult<StageRow> {
    sqlx::query_as::<_, StageRow>("SELECT * FROM stages WHERE id = ?")
        .bind(stage_id.to_string())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)
}

async fn lesson_window(
    transaction: &mut Transaction<'_, Sqlite>,
    lesson_id: &str,
) -> ApiResult<LessonWindowRow> {
    sqlx::query_as::<_, LessonWindowRow>("SELECT closed_at, ends_at FROM lessons WHERE id = ?")
        .bind(lesson_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)
}

fn random_qr_token() -> ApiResult<String> {
    let mut bytes = [0_u8; 24];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ApiError::internal())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
