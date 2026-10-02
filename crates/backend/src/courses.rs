use std::sync::Arc;

use axum::{
    Extension, Json as ResponseJson, Router,
    http::StatusCode,
    middleware,
    routing::{delete, get},
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
    error::{ApiError, ApiResult, Json, Path as ApiPath},
    models::{
        Course, CourseRow, Lesson, LessonDetail, LessonRow, Page, Pagination, Stage, StageRow,
        time, title,
    },
};

#[derive(Clone)]
pub struct CoursesState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCourseRequest {
    title: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddStudentRequest {
    student_no: String,
}

#[derive(Serialize, ToSchema)]
pub struct StudentRosterItem {
    pub user_id: Uuid,
    pub student_no: String,
    pub display_name: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct StudentRosterRow {
    user_id: String,
    student_no: String,
    display_name: String,
    joined_at: i64,
}

impl StudentRosterRow {
    fn dto(self) -> ApiResult<StudentRosterItem> {
        Ok(StudentRosterItem {
            user_id: Uuid::parse_str(&self.user_id).map_err(|_| ApiError::internal())?,
            student_no: self.student_no,
            display_name: self.display_name,
            joined_at: time(self.joined_at)?,
        })
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateLessonRequest {
    title: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
}

pub fn router(pool: SqlitePool, config: Arc<Config>) -> Router {
    let state = CoursesState {
        pool: pool.clone(),
        config: config.clone(),
    };
    Router::new()
        .route("/api/courses", get(list_courses).post(create_course))
        .route("/api/courses/{course_id}", get(get_course))
        .route(
            "/api/courses/{course_id}/students",
            get(list_students).post(add_student),
        )
        .route(
            "/api/courses/{course_id}/students/{user_id}",
            delete(remove_student),
        )
        .route(
            "/api/courses/{course_id}/lessons",
            get(list_lessons).post(create_lesson),
        )
        .route("/api/lessons/{lesson_id}", get(get_lesson))
        .merge(crate::stages::routes())
        .layer(Extension(state))
        .layer(Extension(AuthState {
            pool,
            config: config.clone(),
        }))
        .layer(middleware::from_fn_with_state(config, origin_guard))
}

#[utoipa::path(
    get,
    path = "/api/courses",
    params(
        ("limit" = Option<i64>, Query, description = "Page size from 1 to 100; defaults to 50"),
        ("offset" = Option<i64>, Query, description = "Zero-based offset; defaults to 0")
    ),
    responses(
        (status = 200, description = "Courses visible to the current user", body = Page<Course>),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 422, description = "Invalid pagination", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_list_courses"
)]
pub(crate) async fn list_courses(
    Extension(state): Extension<CoursesState>,
    user: CurrentUser,
    pagination: Pagination,
) -> ApiResult<ResponseJson<Page<Course>>> {
    let (total, rows) = match user.user.role {
        Role::Teacher => {
            let teacher_id = user.user.id.to_string();
            let total =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM courses WHERE teacher_id = ?")
                    .bind(&teacher_id)
                    .fetch_one(&state.pool)
                    .await?;
            let rows = sqlx::query_as::<_, CourseRow>(
                "SELECT id, teacher_id, title, created_at FROM courses WHERE teacher_id = ? \
                 ORDER BY created_at, id LIMIT ? OFFSET ?",
            )
            .bind(teacher_id)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(&state.pool)
            .await?;
            (total, rows)
        }
        Role::Student => {
            let student_id = user.user.id.to_string();
            let visibility = "EXISTS (SELECT 1 FROM enrollments e WHERE e.course_id = c.id AND e.user_id = ?) \
                OR EXISTS (SELECT 1 FROM lessons l JOIN lesson_students ls ON ls.lesson_id = l.id \
                WHERE l.course_id = c.id AND ls.user_id = ?)";
            let total_sql = format!("SELECT COUNT(*) FROM courses c WHERE {visibility}");
            let total = sqlx::query_scalar::<_, i64>(&total_sql)
                .bind(&student_id)
                .bind(&student_id)
                .fetch_one(&state.pool)
                .await?;
            let list_sql = format!(
                "SELECT c.id, c.teacher_id, c.title, c.created_at FROM courses c WHERE {visibility} \
                 ORDER BY c.created_at, c.id LIMIT ? OFFSET ?",
            );
            let rows = sqlx::query_as::<_, CourseRow>(&list_sql)
                .bind(student_id.clone())
                .bind(student_id)
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(&state.pool)
                .await?;
            (total, rows)
        }
    };
    let items = rows
        .into_iter()
        .map(CourseRow::dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(ResponseJson(Page { items, total }))
}

#[utoipa::path(
    post,
    path = "/api/courses",
    request_body = CreateCourseRequest,
    params(("Origin" = String, Header, description = "Must equal the configured application origin")),
    responses(
        (status = 201, description = "Course created", body = Course),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Teacher role required or Origin rejected", body = ApiError),
        (status = 422, description = "Invalid course title or request fields", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_create_course"
)]
pub(crate) async fn create_course(
    Extension(state): Extension<CoursesState>,
    user: CurrentUser,
    Json(input): Json<CreateCourseRequest>,
) -> ApiResult<(StatusCode, ResponseJson<Course>)> {
    user.teacher()?;
    let course_title = title(input.title)?;
    let course_id = Uuid::new_v4();
    let created_at = Utc::now().timestamp_millis();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("INSERT INTO courses (id, teacher_id, title, created_at) VALUES (?, ?, ?, ?)")
        .bind(course_id.to_string())
        .bind(user.user.id.to_string())
        .bind(&course_title)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let course = CourseRow {
        id: course_id.to_string(),
        teacher_id: user.user.id.to_string(),
        title: course_title,
        created_at,
    }
    .dto()?;
    Ok((StatusCode::CREATED, ResponseJson(course)))
}

#[utoipa::path(
    get,
    path = "/api/courses/{course_id}",
    params(("course_id" = Uuid, Path, description = "Course identifier")),
    responses(
        (status = 200, description = "Course visible to the current user", body = Course),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Course not found or not visible", body = ApiError),
        (status = 422, description = "Invalid course identifier", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_get_course"
)]
pub(crate) async fn get_course(
    Extension(state): Extension<CoursesState>,
    ApiPath(course_id): ApiPath<Uuid>,
    user: CurrentUser,
) -> ApiResult<ResponseJson<Course>> {
    Ok(ResponseJson(
        access::course(&state.pool, &user, course_id).await?.dto()?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/courses/{course_id}/students",
    params(
        ("course_id" = Uuid, Path, description = "Course identifier"),
        ("limit" = Option<i64>, Query, description = "Page size from 1 to 100; defaults to 50"),
        ("offset" = Option<i64>, Query, description = "Zero-based offset; defaults to 0")
    ),
    responses(
        (status = 200, description = "Current course roster", body = Page<StudentRosterItem>),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Teacher role required", body = ApiError),
        (status = 404, description = "Course not found or not owned", body = ApiError),
        (status = 422, description = "Invalid path or pagination", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_list_students"
)]
pub(crate) async fn list_students(
    Extension(state): Extension<CoursesState>,
    ApiPath(course_id): ApiPath<Uuid>,
    user: CurrentUser,
    pagination: Pagination,
) -> ApiResult<ResponseJson<Page<StudentRosterItem>>> {
    user.teacher()?;
    let row = access::course(&state.pool, &user, course_id).await?;
    let total =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM enrollments WHERE course_id = ?")
            .bind(&row.id)
            .fetch_one(&state.pool)
            .await?;
    let rows = sqlx::query_as::<_, StudentRosterRow>(
        "SELECT u.id AS user_id, u.student_no, u.display_name, e.joined_at \
         FROM enrollments e JOIN users u ON u.id = e.user_id \
         WHERE e.course_id = ? ORDER BY e.joined_at, e.user_id LIMIT ? OFFSET ?",
    )
    .bind(row.id)
    .bind(pagination.limit)
    .bind(pagination.offset)
    .fetch_all(&state.pool)
    .await?;
    let items = rows
        .into_iter()
        .map(StudentRosterRow::dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(ResponseJson(Page { items, total }))
}

#[utoipa::path(
    post,
    path = "/api/courses/{course_id}/students",
    request_body = AddStudentRequest,
    params(
        ("course_id" = Uuid, Path, description = "Course identifier"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    responses(
        (status = 201, description = "Student added to the roster", body = StudentRosterItem),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Teacher role required or Origin rejected", body = ApiError),
        (status = 404, description = "Course not found or student number not found", body = ApiError),
        (status = 409, description = "Student is already enrolled", body = ApiError),
        (status = 422, description = "Invalid request fields", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_add_student"
)]
pub(crate) async fn add_student(
    Extension(state): Extension<CoursesState>,
    ApiPath(course_id): ApiPath<Uuid>,
    user: CurrentUser,
    Json(input): Json<AddStudentRequest>,
) -> ApiResult<(StatusCode, ResponseJson<StudentRosterItem>)> {
    user.teacher()?;
    let student_no = input.student_no.trim();
    if student_no.is_empty() || student_no.chars().count() > 64 {
        return Err(ApiError::invalid("Invalid student number"));
    }
    let now = Utc::now().timestamp_millis();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let course = sqlx::query_as::<_, CourseRow>(
        "SELECT id, teacher_id, title, created_at FROM courses WHERE id = ?",
    )
    .bind(course_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if course.teacher_id != user.user.id.to_string() {
        return Err(ApiError::not_found());
    }
    let student = sqlx::query_as::<_, EnrollableStudent>(
        "SELECT id, student_no, display_name FROM users WHERE student_no = ? AND role = 'student'",
    )
    .bind(student_no)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    sqlx::query("INSERT INTO enrollments (course_id, user_id, joined_at) VALUES (?, ?, ?)")
        .bind(&course.id)
        .bind(&student.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let roster_item = StudentRosterItem {
        user_id: Uuid::parse_str(&student.id).map_err(|_| ApiError::internal())?,
        student_no: student.student_no,
        display_name: student.display_name,
        joined_at: time(now)?,
    };
    Ok((StatusCode::CREATED, ResponseJson(roster_item)))
}

#[derive(FromRow)]
struct EnrollableStudent {
    id: String,
    student_no: String,
    display_name: String,
}

#[utoipa::path(
    delete,
    path = "/api/courses/{course_id}/students/{user_id}",
    params(
        ("course_id" = Uuid, Path, description = "Course identifier"),
        ("user_id" = Uuid, Path, description = "Student account identifier"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    responses(
        (status = 204, description = "Student removed from the current roster"),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Teacher role required or Origin rejected", body = ApiError),
        (status = 404, description = "Course not found, not owned, or student not enrolled", body = ApiError),
        (status = 422, description = "Invalid identifiers", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_remove_student"
)]
pub(crate) async fn remove_student(
    Extension(state): Extension<CoursesState>,
    ApiPath((course_id, user_id)): ApiPath<(Uuid, Uuid)>,
    user: CurrentUser,
) -> ApiResult<StatusCode> {
    user.teacher()?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let course = sqlx::query_as::<_, CourseRow>(
        "SELECT id, teacher_id, title, created_at FROM courses WHERE id = ?",
    )
    .bind(course_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if course.teacher_id != user.user.id.to_string() {
        return Err(ApiError::not_found());
    }
    let result = sqlx::query("DELETE FROM enrollments WHERE course_id = ? AND user_id = ?")
        .bind(course.id)
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/courses/{course_id}/lessons",
    request_body = CreateLessonRequest,
    params(
        ("course_id" = Uuid, Path, description = "Course identifier"),
        ("Origin" = String, Header, description = "Must equal the configured application origin")
    ),
    responses(
        (status = 201, description = "Lesson created", body = Lesson),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Teacher role required or Origin rejected", body = ApiError),
        (status = 404, description = "Course not found or not owned", body = ApiError),
        (status = 422, description = "Invalid title or lesson time range", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_create_lesson"
)]
pub(crate) async fn create_lesson(
    Extension(state): Extension<CoursesState>,
    ApiPath(course_id): ApiPath<Uuid>,
    user: CurrentUser,
    Json(input): Json<CreateLessonRequest>,
) -> ApiResult<(StatusCode, ResponseJson<Lesson>)> {
    user.teacher()?;
    let lesson_title = title(input.title)?;
    let starts_at = input.starts_at.timestamp_millis();
    let ends_at = input.ends_at.timestamp_millis();
    if ends_at <= starts_at {
        return Err(ApiError::invalid("Lesson end must be after lesson start"));
    }
    let created_at = Utc::now().timestamp_millis();
    let lesson_id = Uuid::new_v4();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let course = sqlx::query_as::<_, CourseRow>(
        "SELECT id, teacher_id, title, created_at FROM courses WHERE id = ?",
    )
    .bind(course_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if course.teacher_id != user.user.id.to_string() {
        return Err(ApiError::not_found());
    }
    sqlx::query(
        "INSERT INTO lessons (id, course_id, title, starts_at, ends_at, closed_at, created_at) \
         VALUES (?, ?, ?, ?, ?, NULL, ?)",
    )
    .bind(lesson_id.to_string())
    .bind(course.id)
    .bind(&lesson_title)
    .bind(starts_at)
    .bind(ends_at)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let lesson = LessonRow {
        id: lesson_id.to_string(),
        course_id: course_id.to_string(),
        title: lesson_title,
        starts_at,
        ends_at,
        closed_at: None,
        created_at,
    }
    .dto()?;
    Ok((StatusCode::CREATED, ResponseJson(lesson)))
}

#[utoipa::path(
    get,
    path = "/api/courses/{course_id}/lessons",
    params(
        ("course_id" = Uuid, Path, description = "Course identifier"),
        ("limit" = Option<i64>, Query, description = "Page size from 1 to 100; defaults to 50"),
        ("offset" = Option<i64>, Query, description = "Zero-based offset; defaults to 0")
    ),
    responses(
        (status = 200, description = "Lessons visible to the current user", body = Page<Lesson>),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Course not found or not visible", body = ApiError),
        (status = 422, description = "Invalid path or pagination", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_list_lessons"
)]
pub(crate) async fn list_lessons(
    Extension(state): Extension<CoursesState>,
    ApiPath(course_id): ApiPath<Uuid>,
    user: CurrentUser,
    pagination: Pagination,
) -> ApiResult<ResponseJson<Page<Lesson>>> {
    let course = access::course(&state.pool, &user, course_id).await?;
    let (total, rows) = match user.user.role {
        Role::Teacher => {
            let total =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lessons WHERE course_id = ?")
                    .bind(&course.id)
                    .fetch_one(&state.pool)
                    .await?;
            let rows = sqlx::query_as::<_, LessonRow>(
                "SELECT id, course_id, title, starts_at, ends_at, closed_at, created_at FROM lessons \
                 WHERE course_id = ? ORDER BY created_at, id LIMIT ? OFFSET ?",
            )
            .bind(course.id).bind(pagination.limit).bind(pagination.offset).fetch_all(&state.pool).await?;
            (total, rows)
        }
        Role::Student => {
            let student_id = user.user.id.to_string();
            let visibility = "(EXISTS (SELECT 1 FROM enrollments e WHERE e.course_id = l.course_id AND e.user_id = ?) \
                OR EXISTS (SELECT 1 FROM lesson_students ls WHERE ls.lesson_id = l.id AND ls.user_id = ?))";
            let count_sql =
                format!("SELECT COUNT(*) FROM lessons l WHERE l.course_id = ? AND {visibility}");
            let total = sqlx::query_scalar::<_, i64>(&count_sql)
                .bind(&course.id)
                .bind(&student_id)
                .bind(&student_id)
                .fetch_one(&state.pool)
                .await?;
            let list_sql = format!(
                "SELECT l.id, l.course_id, l.title, l.starts_at, l.ends_at, l.closed_at, l.created_at \
                 FROM lessons l WHERE l.course_id = ? AND {visibility} ORDER BY l.created_at, l.id LIMIT ? OFFSET ?",
            );
            let rows = sqlx::query_as::<_, LessonRow>(&list_sql)
                .bind(course.id)
                .bind(student_id.clone())
                .bind(student_id)
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(&state.pool)
                .await?;
            (total, rows)
        }
    };
    let items = rows
        .into_iter()
        .map(LessonRow::dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(ResponseJson(Page { items, total }))
}

#[utoipa::path(
    get,
    path = "/api/lessons/{lesson_id}",
    params(("lesson_id" = Uuid, Path, description = "Lesson identifier")),
    responses(
        (status = 200, description = "Lesson details with stages in ordinal order", body = LessonDetail),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 404, description = "Lesson not found or not visible", body = ApiError),
        (status = 422, description = "Invalid lesson identifier", body = ApiError)
    ),
    security(("session" = [])),
    operation_id = "courses_get_lesson"
)]
pub(crate) async fn get_lesson(
    Extension(state): Extension<CoursesState>,
    ApiPath(lesson_id): ApiPath<Uuid>,
    user: CurrentUser,
) -> ApiResult<ResponseJson<LessonDetail>> {
    let lesson = access::lesson(&state.pool, &user, lesson_id).await?;
    let stage_rows = sqlx::query_as::<_, StageRow>(
        "SELECT id, lesson_id, ordinal, kind, opens_at, closes_at, closed_at, latitude, longitude, radius_m \
         FROM stages WHERE lesson_id = ? ORDER BY ordinal",
    )
    .bind(&lesson.id).fetch_all(&state.pool).await?;
    let stages = stage_rows
        .into_iter()
        .map(StageRow::dto)
        .collect::<ApiResult<Vec<Stage>>>()?;
    Ok(ResponseJson(LessonDetail {
        lesson: lesson.dto()?,
        stages,
    }))
}
