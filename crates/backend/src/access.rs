use crate::{
    auth::{CurrentUser, Role},
    error::{ApiError, ApiResult},
    models::{CourseRow, LessonRow},
};
use sqlx::SqlitePool;
use uuid::Uuid;

pub async fn course(
    pool: &SqlitePool,
    user: &CurrentUser,
    course_id: Uuid,
) -> ApiResult<CourseRow> {
    let row = sqlx::query_as::<_, CourseRow>("SELECT * FROM courses WHERE id=?")
        .bind(course_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let allowed=match user.user.role {
  Role::Teacher=>row.teacher_id==user.user.id.to_string(),
  Role::Student=>sqlx::query_scalar::<_,i64>("SELECT EXISTS(SELECT 1 FROM enrollments WHERE course_id=? AND user_id=?) OR EXISTS(SELECT 1 FROM lessons l JOIN lesson_students ls ON ls.lesson_id=l.id WHERE l.course_id=? AND ls.user_id=?)").bind(&row.id).bind(user.user.id.to_string()).bind(&row.id).bind(user.user.id.to_string()).fetch_one(pool).await?!=0,
 };
    if allowed {
        Ok(row)
    } else {
        Err(ApiError::not_found())
    }
}

pub async fn lesson(
    pool: &SqlitePool,
    user: &CurrentUser,
    lesson_id: Uuid,
) -> ApiResult<LessonRow> {
    let row = sqlx::query_as::<_, LessonRow>("SELECT * FROM lessons WHERE id=?")
        .bind(lesson_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let allowed=match user.user.role {
  Role::Teacher=>sqlx::query_scalar::<_,String>("SELECT teacher_id FROM courses WHERE id=?").bind(&row.course_id).fetch_one(pool).await?==user.user.id.to_string(),
  Role::Student=>sqlx::query_scalar::<_,i64>("SELECT EXISTS(SELECT 1 FROM enrollments WHERE course_id=? AND user_id=?) OR EXISTS(SELECT 1 FROM lesson_students WHERE lesson_id=? AND user_id=?)").bind(&row.course_id).bind(user.user.id.to_string()).bind(&row.id).bind(user.user.id.to_string()).fetch_one(pool).await?!=0,
 };
    if allowed {
        Ok(row)
    } else {
        Err(ApiError::not_found())
    }
}
