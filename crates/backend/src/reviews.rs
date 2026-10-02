use std::sync::Arc;

use axum::{
    Extension, Json as ResponseJson, Router,
    body::Body,
    extract::DefaultBodyLimit,
    http::{HeaderValue, StatusCode, header},
    middleware,
    response::Response,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    access,
    auth::{AuthState, CurrentUser, Role, origin_guard},
    config::Config,
    error::{ApiError, ApiResult, Json, Path},
    media::{FieldKind, FieldRule, MULTIPART_LIMIT, MediaField, Multipart, receive},
    models::{Page, Pagination, id, time},
};

#[derive(Clone)]
pub struct ReviewState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
}

pub fn router(pool: SqlitePool, config: Arc<Config>) -> Router {
    let state = ReviewState {
        pool: pool.clone(),
        config: config.clone(),
    };
    Router::new()
        .merge(routes())
        .merge(crate::summary::routes())
        .layer(Extension(AuthState {
            pool,
            config: config.clone(),
        }))
        .layer(Extension(state))
        .layer(DefaultBodyLimit::max(MULTIPART_LIMIT))
        .layer(middleware::from_fn_with_state(config, origin_guard))
}

pub(crate) fn routes() -> Router {
    Router::new()
        .route(
            "/api/attempts/{attempt_id}/review",
            post(create_attempt_review),
        )
        .route("/api/lessons/{lesson_id}/leave", post(create_leave_review))
        .route("/api/lessons/{lesson_id}/reviews", get(list_lesson_reviews))
        .route(
            "/api/reviews/{review_id}/evidence",
            get(get_review_evidence),
        )
        .route("/api/reviews/{review_id}/decision", post(decide_review))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    Partial,
    Late,
    Leave,
}

impl ReviewKind {
    fn as_db(self) -> &'static str {
        match self {
            Self::Partial => "partial",
            Self::Late => "late",
            Self::Leave => "leave",
        }
    }

    fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "partial" => Ok(Self::Partial),
            "late" => Ok(Self::Late),
            "leave" => Ok(Self::Leave),
            _ => Err(ApiError::internal()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    Approved,
    Rejected,
}

impl ReviewStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            _ => Err(ApiError::internal()),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Review {
    pub id: Uuid,
    pub lesson_id: Uuid,
    pub user_id: Uuid,
    pub kind: ReviewKind,
    pub attempt_id: Option<Uuid>,
    pub reason: String,
    pub status: ReviewStatus,
    pub reviewer_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub has_evidence: bool,
}

#[derive(FromRow)]
struct ReviewRow {
    id: String,
    lesson_id: String,
    user_id: String,
    kind: String,
    attempt_id: Option<String>,
    reason: String,
    status: String,
    reviewer_id: Option<String>,
    reviewed_at: Option<i64>,
    decision_note: Option<String>,
    created_at: i64,
    has_evidence: i64,
}

impl ReviewRow {
    fn dto(self) -> ApiResult<Review> {
        Ok(Review {
            id: id(&self.id)?,
            lesson_id: id(&self.lesson_id)?,
            user_id: id(&self.user_id)?,
            kind: ReviewKind::parse(&self.kind)?,
            attempt_id: self.attempt_id.as_deref().map(id).transpose()?,
            reason: self.reason,
            status: ReviewStatus::parse(&self.status)?,
            reviewer_id: self.reviewer_id.as_deref().map(id).transpose()?,
            reviewed_at: self.reviewed_at.map(time).transpose()?,
            decision_note: self.decision_note,
            created_at: time(self.created_at)?,
            has_evidence: self.has_evidence != 0,
        })
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateReviewRequest {
    #[schema(min_length = 1, max_length = 1000)]
    reason: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionRequest {
    decision: ReviewDecision,
    #[schema(max_length = 1000)]
    note: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReviewDecision {
    Approve,
    Reject,
}

impl ReviewDecision {
    fn status(self) -> ReviewStatus {
        match self {
            Self::Approve => ReviewStatus::Approved,
            Self::Reject => ReviewStatus::Rejected,
        }
    }
}

#[derive(ToSchema)]
#[schema(as = LeaveMultipart)]
pub(crate) struct LeaveMultipartSchema {
    #[schema(min_length = 1, max_length = 1000)]
    reason: String,
    /// JPEG, PNG, or PDF attachment, limited to 5 MiB.
    #[schema(value_type = String, format = Binary)]
    evidence: MediaField,
}

#[derive(FromRow)]
struct AttemptReviewRow {
    id: String,
    stage_id: String,
    lesson_id: String,
    user_id: String,
    outcome: String,
    is_late: i64,
}

#[utoipa::path(
    post,
    path = "/api/attempts/{attempt_id}/review",
    operation_id = "reviews_create_attempt_review",
    params(
        ("attempt_id" = Uuid, Path, description = "Reviewable attendance attempt"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    request_body = CreateReviewRequest,
    responses(
        (status = 201, description = "Review request created", body = Review),
        (status = 200, description = "Existing review request for this attempt", body = Review),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Student role required or Origin rejected", body = ApiError),
        (status = 404, description = "Attempt not found or not owned", body = ApiError),
        (status = 409, description = "Attempt is not reviewable, its stage passed, or another review is pending", body = ApiError),
        (status = 422, description = "Invalid reason or path", body = ApiError)
    ),
    security(("session" = [])),
    tag = "reviews"
)]
pub(crate) async fn create_attempt_review(
    Extension(state): Extension<ReviewState>,
    Path(attempt_id): Path<Uuid>,
    user: CurrentUser,
    Json(input): Json<CreateReviewRequest>,
) -> ApiResult<(StatusCode, ResponseJson<Review>)> {
    user.student()?;
    let reason = normalized_reason(input.reason)?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let attempt = sqlx::query_as::<_, AttemptReviewRow>(
        "SELECT a.id, a.stage_id, l.id AS lesson_id, a.user_id, a.outcome, a.is_late \
         FROM attempts a JOIN stages s ON s.id = a.stage_id JOIN lessons l ON l.id = s.lesson_id \
         WHERE a.id = ? AND a.user_id = ? AND (\
            EXISTS (SELECT 1 FROM lesson_students ls WHERE ls.lesson_id = l.id AND ls.user_id = a.user_id) \
            OR EXISTS (SELECT 1 FROM enrollments e WHERE e.course_id = l.course_id AND e.user_id = a.user_id))",
    )
    .bind(attempt_id.to_string())
    .bind(user.user.id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if let Some(row) =
        sqlx::query_as::<_, ReviewRow>(&format!("{REVIEW_SELECT} WHERE attempt_id = ?"))
            .bind(&attempt.id)
            .fetch_optional(&mut *tx)
            .await?
    {
        tx.commit().await?;
        return Ok((StatusCode::OK, ResponseJson(row.dto()?)));
    }
    if attempt.outcome != "reviewable" {
        return Err(ApiError::conflict("Attempt is not eligible for review"));
    }
    let stage_result = sqlx::query_scalar::<_, String>(
        "SELECT status FROM stage_results WHERE stage_id = ? AND user_id = ?",
    )
    .bind(&attempt.stage_id)
    .bind(&attempt.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if stage_result.as_deref() == Some("passed") {
        return Err(ApiError::conflict("Stage has already passed"));
    }
    let other_pending: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM reviews r JOIN attempts a ON a.id = r.attempt_id \
         WHERE r.lesson_id = ? AND r.user_id = ? AND a.stage_id = ? AND r.status = 'pending')",
    )
    .bind(&attempt.lesson_id)
    .bind(&attempt.user_id)
    .bind(&attempt.stage_id)
    .fetch_one(&mut *tx)
    .await?;
    if other_pending != 0 {
        return Err(ApiError::conflict(
            "A review is already pending for this stage",
        ));
    }

    let now = Utc::now().timestamp_millis();
    let review_id = Uuid::new_v4();
    let kind = if attempt.is_late != 0 {
        ReviewKind::Late
    } else {
        ReviewKind::Partial
    };
    sqlx::query(
        "INSERT INTO reviews (id, lesson_id, user_id, kind, attempt_id, reason, evidence_bytes, evidence_mime, \
         status, reviewer_id, reviewed_at, decision_note, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, 'pending', NULL, NULL, NULL, ?)",
    )
    .bind(review_id.to_string())
    .bind(&attempt.lesson_id)
    .bind(&attempt.user_id)
    .bind(kind.as_db())
    .bind(&attempt.id)
    .bind(reason)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let changed = sqlx::query(
        "INSERT INTO stage_results (stage_id, user_id, status, attempt_id, review_id, updated_at) \
         VALUES (?, ?, 'pending', ?, ?, ?) \
         ON CONFLICT(stage_id, user_id) DO UPDATE SET status = 'pending', attempt_id = excluded.attempt_id, \
         review_id = excluded.review_id, updated_at = excluded.updated_at \
         WHERE stage_results.status <> 'passed'",
    )
    .bind(&attempt.stage_id)
    .bind(&attempt.user_id)
    .bind(&attempt.id)
    .bind(review_id.to_string())
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(ApiError::conflict("Stage has already passed"));
    }
    let row = sqlx::query_as::<_, ReviewRow>(&format!("{REVIEW_SELECT} WHERE id = ?"))
        .bind(review_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, ResponseJson(row.dto()?)))
}

#[utoipa::path(
    post,
    path = "/api/lessons/{lesson_id}/leave",
    operation_id = "reviews_create_leave_request",
    params(
        ("lesson_id" = Uuid, Path, description = "Lesson to request leave for"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    request_body(content = LeaveMultipartSchema, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Leave request created; attachment is retained privately", body = Review),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Student role required or Origin rejected", body = ApiError),
        (status = 404, description = "Lesson not found or not visible", body = ApiError),
        (status = 409, description = "A leave request is already pending or approved", body = ApiError),
        (status = 413, description = "Evidence exceeds the upload limit", body = ApiError),
        (status = 422, description = "Invalid multipart fields, reason, MIME type, or evidence magic", body = ApiError)
    ),
    security(("session" = [])),
    tag = "reviews"
)]
pub(crate) async fn create_leave_review(
    Extension(state): Extension<ReviewState>,
    Path(lesson_id): Path<Uuid>,
    user: CurrentUser,
    multipart: Multipart,
) -> ApiResult<(StatusCode, ResponseJson<Review>)> {
    user.student()?;
    let mut fields = receive(
        multipart,
        &[
            FieldRule {
                name: "reason",
                kind: FieldKind::Text,
            },
            FieldRule {
                name: "evidence",
                kind: FieldKind::Evidence,
            },
        ],
    )
    .await?;
    let reason_text = fields
        .remove("reason")
        .ok_or_else(|| ApiError::invalid("reason is required"))?
        .text()?
        .to_owned();
    let reason = normalized_reason(reason_text)?;
    let evidence = fields
        .remove("evidence")
        .ok_or_else(|| ApiError::invalid("evidence is required"))?;
    let input = LeaveMultipartSchema { reason, evidence };
    let evidence_mime = input
        .evidence
        .mime
        .ok_or_else(|| ApiError::invalid("Evidence MIME type is required"))?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query_scalar::<_, String>(
        "SELECT l.id FROM lessons l JOIN courses c ON c.id = l.course_id \
         WHERE l.id = ? AND (EXISTS (SELECT 1 FROM enrollments e WHERE e.course_id = l.course_id AND e.user_id = ?) \
         OR EXISTS (SELECT 1 FROM lesson_students ls WHERE ls.lesson_id = l.id AND ls.user_id = ?))",
    )
    .bind(lesson_id.to_string())
    .bind(user.user.id.to_string())
    .bind(user.user.id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let active_leave: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM reviews WHERE lesson_id = ? AND user_id = ? AND kind = 'leave' \
         AND status IN ('pending', 'approved'))",
    )
    .bind(lesson_id.to_string())
    .bind(user.user.id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if active_leave != 0 {
        return Err(ApiError::conflict(
            "A leave request is already pending or approved",
        ));
    }
    let now = Utc::now().timestamp_millis();
    let review_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reviews (id, lesson_id, user_id, kind, attempt_id, reason, evidence_bytes, evidence_mime, \
         status, reviewer_id, reviewed_at, decision_note, created_at) \
         VALUES (?, ?, ?, 'leave', NULL, ?, ?, ?, 'pending', NULL, NULL, NULL, ?)",
    )
    .bind(review_id.to_string())
    .bind(lesson_id.to_string())
    .bind(user.user.id.to_string())
    .bind(input.reason)
    .bind(input.evidence.bytes)
    .bind(evidence_mime)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, ReviewRow>(&format!("{REVIEW_SELECT} WHERE id = ?"))
        .bind(review_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, ResponseJson(row.dto()?)))
}

#[utoipa::path(
    get,
    path = "/api/lessons/{lesson_id}/reviews",
    operation_id = "reviews_list_lesson_reviews",
    params(
        ("lesson_id" = Uuid, Path, description = "Lesson identifier"),
        ("limit" = Option<i64>, Query, description = "Page size from 1 to 100; defaults to 50"),
        ("offset" = Option<i64>, Query, description = "Zero-based offset; defaults to 0")
    ),
    responses(
        (status = 200, description = "Reviews visible to the current user; evidence bytes are not included", body = Page<Review>),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Lesson not found or not visible", body = ApiError),
        (status = 422, description = "Invalid lesson identifier or pagination", body = ApiError)
    ),
    security(("session" = [])),
    tag = "reviews"
)]
pub(crate) async fn list_lesson_reviews(
    Extension(state): Extension<ReviewState>,
    Path(lesson_id): Path<Uuid>,
    user: CurrentUser,
    pagination: Pagination,
) -> ApiResult<ResponseJson<Page<Review>>> {
    access::lesson(&state.pool, &user, lesson_id).await?;
    let (count_sql, list_sql) = match user.user.role {
        Role::Teacher => (
            "SELECT COUNT(*) FROM reviews WHERE lesson_id = ?",
            "SELECT id, lesson_id, user_id, kind, attempt_id, reason, status, reviewer_id, reviewed_at, \
             decision_note, created_at, evidence_bytes IS NOT NULL AS has_evidence FROM reviews \
             WHERE lesson_id = ? ORDER BY created_at, id LIMIT ? OFFSET ?",
        ),
        Role::Student => (
            "SELECT COUNT(*) FROM reviews WHERE lesson_id = ? AND user_id = ?",
            "SELECT id, lesson_id, user_id, kind, attempt_id, reason, status, reviewer_id, reviewed_at, \
             decision_note, created_at, evidence_bytes IS NOT NULL AS has_evidence FROM reviews \
             WHERE lesson_id = ? AND user_id = ? ORDER BY created_at, id LIMIT ? OFFSET ?",
        ),
    };
    let total = match user.user.role {
        Role::Teacher => {
            sqlx::query_scalar::<_, i64>(count_sql)
                .bind(lesson_id.to_string())
                .fetch_one(&state.pool)
                .await?
        }
        Role::Student => {
            sqlx::query_scalar::<_, i64>(count_sql)
                .bind(lesson_id.to_string())
                .bind(user.user.id.to_string())
                .fetch_one(&state.pool)
                .await?
        }
    };
    let rows = match user.user.role {
        Role::Teacher => {
            sqlx::query_as::<_, ReviewRow>(list_sql)
                .bind(lesson_id.to_string())
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(&state.pool)
                .await?
        }
        Role::Student => {
            sqlx::query_as::<_, ReviewRow>(list_sql)
                .bind(lesson_id.to_string())
                .bind(user.user.id.to_string())
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(&state.pool)
                .await?
        }
    };
    let items = rows
        .into_iter()
        .map(ReviewRow::dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(ResponseJson(Page { items, total }))
}

#[derive(FromRow)]
struct ReviewOwnerRow {
    lesson_id: String,
    user_id: String,
}

#[derive(FromRow)]
struct ReviewEvidenceRow {
    evidence_bytes: Option<Vec<u8>>,
    evidence_mime: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/reviews/{review_id}/evidence",
    operation_id = "reviews_get_evidence",
    params(("review_id" = Uuid, Path, description = "Review identifier")),
    responses(
        (status = 200, description = "Original validated PDF, PNG, or JPEG evidence bytes with private download headers", content_type = "application/octet-stream", body = String, headers(("Content-Disposition" = String, description = "attachment; filename=\"evidence\""), ("X-Content-Type-Options" = String, description = "nosniff"), ("Cache-Control" = String, description = "no-store"))),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Review not found or not visible", body = ApiError),
        (status = 422, description = "Review does not contain evidence", body = ApiError)
    ),
    security(("session" = [])),
    tag = "reviews"
)]
pub(crate) async fn get_review_evidence(
    Extension(state): Extension<ReviewState>,
    Path(review_id): Path<Uuid>,
    user: CurrentUser,
) -> ApiResult<Response> {
    let owner = sqlx::query_as::<_, ReviewOwnerRow>(
        "SELECT r.lesson_id, r.user_id FROM reviews r WHERE r.id = ?",
    )
    .bind(review_id.to_string())
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(ApiError::not_found)?;
    authorize_review(&state, &user, &owner.lesson_id, &owner.user_id).await?;
    let row = sqlx::query_as::<_, ReviewEvidenceRow>(
        "SELECT evidence_bytes, evidence_mime FROM reviews WHERE id = ?",
    )
    .bind(review_id.to_string())
    .fetch_one(&state.pool)
    .await?;
    let bytes = row
        .evidence_bytes
        .ok_or_else(|| ApiError::invalid("Review has no evidence"))?;
    let mime = row.evidence_mime.ok_or_else(ApiError::internal)?;
    if !matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "application/pdf"
    ) {
        return Err(ApiError::internal());
    }
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime).map_err(|_| ApiError::internal())?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"evidence\""),
    );
    response.headers_mut().insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/reviews/{review_id}/decision",
    operation_id = "reviews_decide_review",
    params(
        ("review_id" = Uuid, Path, description = "Review identifier"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    request_body = DecisionRequest,
    responses(
        (status = 200, description = "Review decision applied or repeated idempotently", body = Review),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Owning teacher role required or Origin rejected", body = ApiError),
        (status = 404, description = "Review not found or not owned", body = ApiError),
        (status = 409, description = "A different decision already exists", body = ApiError),
        (status = 422, description = "Invalid path, decision, or note", body = ApiError)
    ),
    security(("session" = [])),
    tag = "reviews"
)]
pub(crate) async fn decide_review(
    Extension(state): Extension<ReviewState>,
    Path(review_id): Path<Uuid>,
    user: CurrentUser,
    Json(input): Json<DecisionRequest>,
) -> ApiResult<ResponseJson<Review>> {
    user.teacher()?;
    let note = input.note.map(|value| value.trim().to_owned());
    if note
        .as_ref()
        .is_some_and(|value| value.chars().count() > 1000)
    {
        return Err(ApiError::invalid(
            "Decision note must be at most 1000 characters",
        ));
    }
    let note = note.filter(|value| !value.is_empty());
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query_as::<_, ReviewRow>(
        &format!("{REVIEW_SELECT} JOIN lessons ON lessons.id = reviews.lesson_id JOIN courses c ON c.id = lessons.course_id WHERE reviews.id = ? AND c.teacher_id = ?"),
    )
    .bind(review_id.to_string())
    .bind(user.user.id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let desired = input.decision.status();
    let current = ReviewStatus::parse(&row.status)?;
    if current != ReviewStatus::Pending {
        if current != desired {
            return Err(ApiError::conflict(
                "Review already has a different decision",
            ));
        }
        tx.commit().await?;
        return Ok(ResponseJson(row.dto()?));
    }

    let reviewed_at = Utc::now().timestamp_millis();
    let changed = sqlx::query(
        "UPDATE reviews SET status = ?, reviewer_id = ?, reviewed_at = ?, decision_note = ? \
         WHERE id = ? AND status = 'pending'",
    )
    .bind(desired.as_db())
    .bind(user.user.id.to_string())
    .bind(reviewed_at)
    .bind(note)
    .bind(review_id.to_string())
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Review decision was changed concurrently",
        ));
    }
    if row.kind == "partial" || row.kind == "late" {
        let attempt_id = row.attempt_id.as_deref().ok_or_else(ApiError::internal)?;
        let stage_id: String = sqlx::query_scalar("SELECT stage_id FROM attempts WHERE id = ?")
            .bind(attempt_id)
            .fetch_one(&mut *tx)
            .await?;
        match input.decision {
            ReviewDecision::Approve => {
                sqlx::query(
                    "UPDATE stage_results SET status = 'passed', updated_at = ? \
                     WHERE stage_id = ? AND user_id = ? AND status = 'pending' AND review_id = ?",
                )
                .bind(reviewed_at)
                .bind(stage_id)
                .bind(&row.user_id)
                .bind(review_id.to_string())
                .execute(&mut *tx)
                .await?;
            }
            ReviewDecision::Reject => {
                sqlx::query(
                    "UPDATE stage_results SET status = 'failed', updated_at = ? \
                     WHERE stage_id = ? AND user_id = ? AND status = 'pending' AND review_id = ?",
                )
                .bind(reviewed_at)
                .bind(stage_id)
                .bind(&row.user_id)
                .bind(review_id.to_string())
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    let result = sqlx::query_as::<_, ReviewRow>(&format!("{REVIEW_SELECT} WHERE id = ?"))
        .bind(review_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ResponseJson(result.dto()?))
}

const REVIEW_SELECT: &str = "SELECT reviews.id, reviews.lesson_id, reviews.user_id, reviews.kind, reviews.attempt_id, \
    reviews.reason, reviews.status, reviews.reviewer_id, reviews.reviewed_at, reviews.decision_note, reviews.created_at, \
    reviews.evidence_bytes IS NOT NULL AS has_evidence FROM reviews";

async fn authorize_review(
    state: &ReviewState,
    user: &CurrentUser,
    lesson_id: &str,
    review_user_id: &str,
) -> ApiResult<()> {
    match user.user.role {
        Role::Teacher => {
            let owner: Option<String> = sqlx::query_scalar(
                "SELECT c.teacher_id FROM lessons l JOIN courses c ON c.id = l.course_id WHERE l.id = ?",
            )
            .bind(lesson_id)
            .fetch_optional(&state.pool)
            .await?;
            let user_id = user.user.id.to_string();
            if owner.as_deref() != Some(user_id.as_str()) {
                return Err(ApiError::not_found());
            }
        }
        Role::Student => {
            if review_user_id != user.user.id.to_string() {
                return Err(ApiError::not_found());
            }
            access::lesson(&state.pool, user, id(lesson_id)?).await?;
        }
    }
    Ok(())
}

fn normalized_reason(value: String) -> ApiResult<String> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.chars().count() > 1000 {
        return Err(ApiError::invalid("Reason must contain 1–1000 characters"));
    }
    Ok(value)
}
