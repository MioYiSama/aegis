use std::collections::HashMap;

use axum::{Extension, Json as ResponseJson, Router, routing::get};
use chrono::Utc;
use serde::Serialize;
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::{CurrentUser, Role},
    error::{ApiError, ApiResult},
    models::{StageKind, id},
    reviews::ReviewState,
};

pub(crate) fn routes() -> Router {
    Router::new().route("/api/lessons/{lesson_id}/attendance", get(get_attendance))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageResultStatus {
    Missing,
    Failed,
    Pending,
    Passed,
}

impl StageResultStatus {
    fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "failed" => Ok(Self::Failed),
            "pending" => Ok(Self::Pending),
            "passed" => Ok(Self::Passed),
            _ => Err(ApiError::internal()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceStatus {
    InProgress,
    Excused,
    Present,
    Pending,
    Absent,
    EarlyDeparture,
}

#[derive(Serialize, ToSchema)]
pub struct StageAttendance {
    pub stage_id: Uuid,
    pub kind: StageKind,
    pub status: StageResultStatus,
}

#[derive(Serialize, ToSchema)]
pub struct StudentAttendance {
    pub user_id: Uuid,
    pub status: AttendanceStatus,
    pub attendance_success: Option<bool>,
    pub is_late: bool,
    pub stages: Vec<StageAttendance>,
}

#[derive(Serialize, ToSchema)]
pub struct LessonAttendance {
    pub lesson_id: Uuid,
    pub students: Vec<StudentAttendance>,
}

#[derive(FromRow)]
struct AccessibleLesson {
    id: String,
    course_id: String,
    ends_at: i64,
    closed_at: Option<i64>,
    teacher_id: String,
}

#[derive(FromRow)]
struct StageRow {
    id: String,
    kind: String,
}

struct StageInfo {
    id: String,
    stage_id: Uuid,
    kind: StageKind,
}

#[derive(FromRow)]
struct StageResultRow {
    stage_id: String,
    user_id: String,
    status: String,
}

#[derive(FromRow)]
struct ReviewFlagRow {
    user_id: String,
    kind: String,
    status: String,
    approved_late_attempt: i64,
}

#[derive(Clone, Default)]
struct StudentReviewFlags {
    approved_leave: bool,
    pending: bool,
    approved_late: bool,
}

#[utoipa::path(
    get,
    path = "/api/lessons/{lesson_id}/attendance",
    operation_id = "summary_get_lesson_attendance",
    params(("lesson_id" = Uuid, Path, description = "Lesson identifier")),
    responses(
        (status = 200, description = "Full lesson roster attendance summary with one row for the current student or all roster students for the teacher", body = LessonAttendance),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Lesson not found or not visible", body = ApiError),
        (status = 422, description = "Invalid lesson identifier", body = ApiError)
    ),
    security(("session" = [])),
    tag = "attendance"
)]
pub(crate) async fn get_attendance(
    Extension(state): Extension<ReviewState>,
    crate::error::Path(lesson_id): crate::error::Path<Uuid>,
    user: CurrentUser,
) -> ApiResult<ResponseJson<LessonAttendance>> {
    let mut tx = state.pool.begin().await?;
    let lesson = sqlx::query_as::<_, AccessibleLesson>(
        "SELECT l.id, l.course_id, l.ends_at, l.closed_at, c.teacher_id \
         FROM lessons l JOIN courses c ON c.id = l.course_id WHERE l.id = ?",
    )
    .bind(lesson_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let current_user_id = user.user.id.to_string();
    match user.user.role {
        Role::Teacher if lesson.teacher_id != current_user_id => return Err(ApiError::not_found()),
        Role::Student => {
            let member: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM enrollments WHERE course_id = ? AND user_id = ?) \
                 OR EXISTS(SELECT 1 FROM lesson_students WHERE lesson_id = ? AND user_id = ?)",
            )
            .bind(&lesson.course_id)
            .bind(current_user_id.clone())
            .bind(&lesson.id)
            .bind(current_user_id.clone())
            .fetch_one(&mut *tx)
            .await?;
            if member == 0 {
                return Err(ApiError::not_found());
            }
        }
        Role::Teacher => {}
    }

    let stage_rows = sqlx::query_as::<_, StageRow>(
        "SELECT id, kind FROM stages WHERE lesson_id = ? ORDER BY ordinal, id",
    )
    .bind(&lesson.id)
    .fetch_all(&mut *tx)
    .await?;
    let stages = stage_rows
        .into_iter()
        .map(|stage| {
            Ok(StageInfo {
                stage_id: id(&stage.id)?,
                id: stage.id,
                kind: StageKind::parse(&stage.kind)?,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let has_check_in = stages.iter().any(|stage| stage.kind == StageKind::CheckIn);
    let roster_ids: Vec<String> = if has_check_in {
        sqlx::query_scalar(
            "SELECT user_id FROM lesson_students WHERE lesson_id = ? ORDER BY user_id",
        )
        .bind(&lesson.id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar("SELECT user_id FROM enrollments WHERE course_id = ? ORDER BY user_id")
            .bind(&lesson.course_id)
            .fetch_all(&mut *tx)
            .await?
    };
    let stage_results = sqlx::query_as::<_, StageResultRow>(
        "SELECT sr.stage_id, sr.user_id, sr.status FROM stage_results sr \
         JOIN stages s ON s.id = sr.stage_id WHERE s.lesson_id = ?",
    )
    .bind(&lesson.id)
    .fetch_all(&mut *tx)
    .await?;
    let mut result_by_stage_user: HashMap<String, HashMap<String, StageResultStatus>> =
        HashMap::new();
    for result in stage_results {
        result_by_stage_user
            .entry(result.stage_id)
            .or_default()
            .insert(result.user_id, StageResultStatus::parse(&result.status)?);
    }

    let review_flags = sqlx::query_as::<_, ReviewFlagRow>(
        "SELECT r.user_id, r.kind, r.status, \
         CASE WHEN r.kind = 'late' AND r.status = 'approved' AND a.is_late = 1 THEN 1 ELSE 0 END AS approved_late_attempt \
         FROM reviews r LEFT JOIN attempts a ON a.id = r.attempt_id WHERE r.lesson_id = ? \
         AND (r.status = 'pending' OR (r.status = 'approved' AND r.kind IN ('leave', 'late'))) \
         ORDER BY r.created_at, r.id",
    )
    .bind(&lesson.id)
    .fetch_all(&mut *tx)
    .await?;
    let mut flags_by_user: HashMap<String, StudentReviewFlags> = HashMap::new();
    for flag in review_flags {
        let flags = flags_by_user.entry(flag.user_id).or_default();
        match (flag.kind.as_str(), flag.status.as_str()) {
            ("leave", "approved") => flags.approved_leave = true,
            (_, "pending") => flags.pending = true,
            ("late", "approved") if flag.approved_late_attempt != 0 => flags.approved_late = true,
            ("partial" | "late", "approved") => {}
            (_, "rejected") => {}
            _ => return Err(ApiError::internal()),
        }
    }

    let ended = lesson.closed_at.is_some() || Utc::now().timestamp_millis() >= lesson.ends_at;
    let check_in_index = stages
        .iter()
        .position(|stage| stage.kind == StageKind::CheckIn);
    let mut students = Vec::with_capacity(roster_ids.len());
    for student_id in roster_ids
        .into_iter()
        .filter(|id| user.user.role == Role::Teacher || id == &current_user_id)
    {
        let mut per_stage = Vec::with_capacity(stages.len());
        for stage in &stages {
            let result = result_by_stage_user
                .get(&stage.id)
                .and_then(|per_user| per_user.get(&student_id))
                .copied()
                .unwrap_or(StageResultStatus::Missing);
            per_stage.push(StageAttendance {
                stage_id: stage.stage_id,
                kind: stage.kind,
                status: result,
            });
        }
        let flags = flags_by_user.get(&student_id).cloned().unwrap_or_default();
        let check_in_passed = check_in_index
            .and_then(|index| per_stage.get(index))
            .is_some_and(|stage| stage.status == StageResultStatus::Passed);
        let all_actual_stages_passed = check_in_index.is_some()
            && !per_stage.is_empty()
            && per_stage
                .iter()
                .all(|stage| stage.status == StageResultStatus::Passed);
        let (status, attendance_success) = if flags.approved_leave {
            (AttendanceStatus::Excused, Some(true))
        } else if !ended {
            (AttendanceStatus::InProgress, None)
        } else if check_in_passed && all_actual_stages_passed {
            (AttendanceStatus::Present, Some(true))
        } else if flags.pending {
            (AttendanceStatus::Pending, None)
        } else if !check_in_passed {
            (AttendanceStatus::Absent, Some(false))
        } else {
            (AttendanceStatus::EarlyDeparture, Some(false))
        };
        students.push(StudentAttendance {
            user_id: id(&student_id)?,
            status,
            attendance_success,
            is_late: flags.approved_late,
            stages: per_stage,
        });
    }
    tx.commit().await?;
    Ok(ResponseJson(LessonAttendance {
        lesson_id,
        students,
    }))
}
