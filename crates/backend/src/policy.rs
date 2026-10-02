use crate::{
    error::{ApiError, ApiResult},
    models::StageKind,
};
use serde::Serialize;
use sqlx::{Sqlite, Transaction};
use utoipa::ToSchema;

pub const LATE_WINDOW_MS: i64 = 15 * 60 * 1_000;
const EARTH_RADIUS_METERS: f64 = 6_371_008.8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionWindow {
    Regular,
    Late,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Passed,
    Reviewable,
    Failed,
}

impl AttemptOutcome {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Reviewable => "reviewable",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    QrMissing,
    QrUnreadable,
    QrExpired,
    QrWrongStage,
    LocationMissing,
    LocationInvalid,
    LocationInaccurate,
    LocationOutside,
    FaceMissing,
    FaceUnenrolled,
    FaceNotSingle,
    FaceSpoof,
    FaceMismatch,
}

#[derive(Clone, Copy, Debug)]
pub struct LocationInput {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_m: f64,
}

/// Selects the server-side submission window using only persisted timestamps and the
/// timestamp captured immediately after the bounded multipart body was received.
pub fn submission_window(
    kind: StageKind,
    stage_opens_at: i64,
    stage_closes_at: i64,
    stage_closed_at: Option<i64>,
    lesson_starts_at: i64,
    lesson_ends_at: i64,
    lesson_closed: bool,
    received_at: i64,
) -> Option<SubmissionWindow> {
    if lesson_closed
        || received_at >= lesson_ends_at
        || stage_closed_at.is_some_and(|closed| closed > received_at)
    {
        return None;
    }
    if received_at >= stage_opens_at && received_at < stage_closes_at {
        return Some(SubmissionWindow::Regular);
    }
    if kind == StageKind::CheckIn
        && received_at >= stage_closes_at
        && received_at >= lesson_starts_at
        && lesson_starts_at
            .checked_add(LATE_WINDOW_MS)
            .is_some_and(|deadline| received_at <= deadline)
    {
        return Some(SubmissionWindow::Late);
    }
    None
}

pub fn qr_token_valid_at(expires_at: i64, received_at: i64) -> bool {
    received_at < expires_at
}

/// Returns the fixed failure reason for an absent or insufficiently reliable location.
pub fn location_failure(
    location: Option<LocationInput>,
    target_latitude: f64,
    target_longitude: f64,
    radius_m: f64,
) -> Option<ReasonCode> {
    let Some(location) = location else {
        return Some(ReasonCode::LocationMissing);
    };
    if !location.latitude.is_finite()
        || !location.longitude.is_finite()
        || !(-90.0..=90.0).contains(&location.latitude)
        || !(-180.0..=180.0).contains(&location.longitude)
    {
        return Some(ReasonCode::LocationInvalid);
    }
    if !location.accuracy_m.is_finite()
        || location.accuracy_m <= 0.0
        || location.accuracy_m > radius_m
    {
        return Some(ReasonCode::LocationInaccurate);
    }
    let distance = haversine_meters(
        location.latitude,
        location.longitude,
        target_latitude,
        target_longitude,
    );
    if distance + location.accuracy_m > radius_m {
        return Some(ReasonCode::LocationOutside);
    }
    None
}

pub fn haversine_meters(
    latitude: f64,
    longitude: f64,
    target_latitude: f64,
    target_longitude: f64,
) -> f64 {
    let lat1 = latitude.to_radians();
    let lat2 = target_latitude.to_radians();
    let delta_lat = (target_latitude - latitude).to_radians();
    let delta_lon = (target_longitude - longitude).to_radians();
    let a =
        (delta_lat * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (delta_lon * 0.5).sin().powi(2);
    2.0 * EARTH_RADIUS_METERS * a.clamp(0.0, 1.0).sqrt().asin()
}

/// Implements the sole policy exception for human review: exactly two normally-required
/// check-in factors, or both late face/location factors, are reviewable.
pub fn attempt_outcome(
    kind: StageKind,
    window: SubmissionWindow,
    qr_pass: Option<bool>,
    location_pass: bool,
    face_pass: bool,
) -> AttemptOutcome {
    match (kind, window) {
        (StageKind::CheckIn, SubmissionWindow::Regular) => {
            let Some(qr_pass) = qr_pass else {
                return AttemptOutcome::Failed;
            };
            let count = (qr_pass as usize) + (location_pass as usize) + (face_pass as usize);
            match count {
                3 => AttemptOutcome::Passed,
                2 => AttemptOutcome::Reviewable,
                _ => AttemptOutcome::Failed,
            }
        }
        (StageKind::CheckIn, SubmissionWindow::Late) => {
            if location_pass && face_pass {
                AttemptOutcome::Reviewable
            } else {
                AttemptOutcome::Failed
            }
        }
        (StageKind::Renew | StageKind::CheckOut, SubmissionWindow::Regular) => {
            if location_pass && face_pass {
                AttemptOutcome::Passed
            } else {
                AttemptOutcome::Failed
            }
        }
        (StageKind::Renew | StageKind::CheckOut, SubmissionWindow::Late) => AttemptOutcome::Failed,
    }
}

pub struct AttemptPersistence<'a> {
    pub attempt_id: &'a str,
    pub stage_id: &'a str,
    pub user_id: &'a str,
    pub submitted_at: i64,
    pub qr_pass: Option<bool>,
    pub location_pass: bool,
    pub face_pass: bool,
    pub is_late: bool,
    pub outcome: AttemptOutcome,
    pub reason_codes: &'a [ReasonCode],
}

/// Stores one factor result and advances the stage result monotonically. Call inside the
/// same short `BEGIN IMMEDIATE` transaction as the final lesson/stage/membership recheck.
pub async fn apply_attempt_result(
    transaction: &mut Transaction<'_, Sqlite>,
    attempt: AttemptPersistence<'_>,
) -> ApiResult<()> {
    let reason_codes =
        serde_json::to_string(attempt.reason_codes).map_err(|_| ApiError::internal())?;
    sqlx::query(
        "INSERT INTO attempts (id, stage_id, user_id, submitted_at, qr_pass, location_pass, face_pass, is_late, outcome, reason_codes) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(attempt.attempt_id)
    .bind(attempt.stage_id)
    .bind(attempt.user_id)
    .bind(attempt.submitted_at)
    .bind(attempt.qr_pass.map(|passed| passed as i64))
    .bind(attempt.location_pass as i64)
    .bind(attempt.face_pass as i64)
    .bind(attempt.is_late as i64)
    .bind(attempt.outcome.as_db())
    .bind(reason_codes)
    .execute(&mut **transaction)
    .await
    .map_err(ApiError::from)?;

    if attempt.outcome == AttemptOutcome::Passed {
        sqlx::query(
            "UPDATE reviews SET status = 'rejected', reviewer_id = ?, reviewed_at = ?, decision_note = 'superseded_by_success' \
             WHERE status = 'pending' AND user_id = ? AND kind IN ('partial', 'late') \
               AND attempt_id IN (SELECT id FROM attempts WHERE stage_id = ? AND user_id = ?)",
        )
        .bind(attempt.user_id)
        .bind(attempt.submitted_at)
        .bind(attempt.user_id)
        .bind(attempt.stage_id)
        .bind(attempt.user_id)
        .execute(&mut **transaction)
        .await
        .map_err(ApiError::from)?;
        sqlx::query(
            "INSERT INTO stage_results (stage_id, user_id, status, attempt_id, review_id, updated_at) \
             VALUES (?, ?, 'passed', ?, NULL, ?) \
             ON CONFLICT(stage_id, user_id) DO UPDATE SET status = 'passed', attempt_id = excluded.attempt_id, \
                 review_id = NULL, updated_at = excluded.updated_at",
        )
        .bind(attempt.stage_id)
        .bind(attempt.user_id)
        .bind(attempt.attempt_id)
        .bind(attempt.submitted_at)
        .execute(&mut **transaction)
        .await
        .map_err(ApiError::from)?;
    } else {
        sqlx::query(
            "INSERT INTO stage_results (stage_id, user_id, status, attempt_id, review_id, updated_at) \
             VALUES (?, ?, 'failed', ?, NULL, ?) \
             ON CONFLICT(stage_id, user_id) DO UPDATE SET status = 'failed', attempt_id = excluded.attempt_id, \
                 review_id = NULL, updated_at = excluded.updated_at \
             WHERE stage_results.status = 'failed'",
        )
        .bind(attempt.stage_id)
        .bind(attempt.user_id)
        .bind(attempt.attempt_id)
        .bind(attempt.submitted_at)
        .execute(&mut **transaction)
        .await
        .map_err(ApiError::from)?;
    }
    Ok(())
}
