use crate::error::{ApiError, ApiResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn time(ms: i64) -> ApiResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms).ok_or_else(ApiError::internal)
}
pub fn id(value: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| ApiError::internal())
}
pub fn title(value: String) -> ApiResult<String> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.chars().count() > 128 {
        Err(ApiError::invalid("Title must contain 1–128 characters"))
    } else {
        Ok(value)
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Course {
    pub id: Uuid,
    pub teacher_id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, FromRow)]
pub struct CourseRow {
    pub id: String,
    pub teacher_id: String,
    pub title: String,
    pub created_at: i64,
}
impl CourseRow {
    pub fn dto(self) -> ApiResult<Course> {
        Ok(Course {
            id: id(&self.id)?,
            teacher_id: id(&self.teacher_id)?,
            title: self.title,
            created_at: time(self.created_at)?,
        })
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Lesson {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, FromRow)]
pub struct LessonRow {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub closed_at: Option<i64>,
    pub created_at: i64,
}
impl LessonRow {
    pub fn dto(self) -> ApiResult<Lesson> {
        Ok(Lesson {
            id: id(&self.id)?,
            course_id: id(&self.course_id)?,
            title: self.title,
            starts_at: time(self.starts_at)?,
            ends_at: time(self.ends_at)?,
            closed_at: self.closed_at.map(time).transpose()?,
            created_at: time(self.created_at)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    CheckIn,
    Renew,
    CheckOut,
}
impl StageKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::CheckIn => "check_in",
            Self::Renew => "renew",
            Self::CheckOut => "check_out",
        }
    }
    pub fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "check_in" => Ok(Self::CheckIn),
            "renew" => Ok(Self::Renew),
            "check_out" => Ok(Self::CheckOut),
            _ => Err(ApiError::internal()),
        }
    }
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Stage {
    pub id: Uuid,
    pub ordinal: i64,
    pub kind: StageKind,
    pub opens_at: DateTime<Utc>,
    pub closes_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub latitude: f64,
    pub longitude: f64,
    pub radius_m: f64,
}
#[derive(Clone, FromRow)]
pub struct StageRow {
    pub id: String,
    pub lesson_id: String,
    pub ordinal: i64,
    pub kind: String,
    pub opens_at: i64,
    pub closes_at: i64,
    pub closed_at: Option<i64>,
    pub latitude: f64,
    pub longitude: f64,
    pub radius_m: f64,
}
impl StageRow {
    pub fn dto(self) -> ApiResult<Stage> {
        Ok(Stage {
            id: id(&self.id)?,
            ordinal: self.ordinal,
            kind: StageKind::parse(&self.kind)?,
            opens_at: time(self.opens_at)?,
            closes_at: time(self.closes_at)?,
            closed_at: self.closed_at.map(time).transpose()?,
            latitude: self.latitude,
            longitude: self.longitude,
            radius_m: self.radius_m,
        })
    }
}
#[derive(Serialize, ToSchema)]
pub struct LessonDetail {
    #[serde(flatten)]
    pub lesson: Lesson,
    pub stages: Vec<Stage>,
}

#[derive(Serialize, ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn default_limit() -> i64 {
    50
}
#[derive(Clone, Copy)]
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Pagination {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> ApiResult<Self> {
        let axum::extract::Query(query) =
            axum::extract::Query::<PageQuery>::from_request_parts(parts, state)
                .await
                .map_err(|_| ApiError::invalid("Invalid pagination"))?;
        if !(1..=100).contains(&query.limit) || query.offset < 0 {
            return Err(ApiError::invalid("Invalid pagination"));
        }
        Ok(Self {
            limit: query.limit,
            offset: query.offset,
        })
    }
}
