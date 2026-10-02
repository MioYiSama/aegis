use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use aegis_backend::{auth, config::Config, courses};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Method, Request, Response, StatusCode,
        header::{COOKIE, ORIGIN, SET_COOKIE},
    },
};
use chrono::{SecondsFormat, Utc};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN_VALUE: &str = "http://localhost:3000";
const PASSWORD: &str = "test-password-123";

struct TestApp {
    pool: SqlitePool,
    config: Arc<Config>,
    _directory: tempfile::TempDir,
}

impl TestApp {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("courses.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let config = Arc::new(Config {
            bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            database_url: format!("sqlite://{}", database_path.display()),
            origin: ORIGIN_VALUE.to_owned(),
            cookie_secure: false,
            model_dir: PathBuf::from("models"),
        });
        Self {
            pool,
            config,
            _directory: directory,
        }
    }

    fn router(&self) -> Router {
        Router::new()
            .merge(auth::router(self.pool.clone(), self.config.clone()))
            .merge(courses::router(self.pool.clone(), self.config.clone()))
    }
}

fn request(
    method: Method,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Request<Body> {
    let has_body = body.is_some();
    let body = body
        .map(|value| Body::from(value.to_string()))
        .unwrap_or_else(Body::empty);
    let mut builder = Request::builder().method(method).uri(path);
    let headers = builder.headers_mut().unwrap();
    if has_body {
        headers.insert("content-type", "application/json".parse().unwrap());
    }
    if let Some(cookie) = cookie {
        headers.insert(COOKIE, cookie.parse().unwrap());
    }
    if let Some(origin) = origin {
        headers.insert(ORIGIN, origin.parse().unwrap());
    }
    builder.body(body).unwrap()
}

async fn send(
    router: Router,
    method: Method,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Response<Body> {
    router
        .oneshot(request(method, path, body, cookie, origin))
        .await
        .unwrap()
}

async fn json_body<T: DeserializeOwned>(response: Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn register(app: &TestApp, username: &str, role: &str, student_no: Option<&str>) {
    let mut body = json!({
        "username": username,
        "password": PASSWORD,
        "display_name": format!("Display {username}"),
        "role": role,
    });
    if let Some(student_no) = student_no {
        body["student_no"] = json!(student_no);
    }
    let response = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(body),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn login(app: &TestApp, username: &str) -> String {
    let response = send(
        app.router(),
        Method::POST,
        "/api/auth/login",
        Some(json!({ "username": username, "password": PASSWORD })),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    cookie
}

fn lesson_body(title: &str) -> Value {
    let now = Utc::now();
    json!({
        "title": title,
        "starts_at": (now - chrono::Duration::minutes(1)).to_rfc3339_opts(SecondsFormat::Millis, true),
        "ends_at": (now + chrono::Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

#[tokio::test]
async fn roster_permissions_and_frozen_lesson_visibility_survive_unenrollment() {
    let app = TestApp::new().await;
    register(&app, "teacher-a", "teacher", None).await;
    register(&app, "teacher-b", "teacher", None).await;
    register(&app, "student-a", "student", Some("student-a-no")).await;
    register(&app, "student-b", "student", Some("student-b-no")).await;
    let teacher_a = login(&app, "teacher-a").await;
    let teacher_b = login(&app, "teacher-b").await;
    let student_a = login(&app, "student-a").await;
    let student_b = login(&app, "student-b").await;

    let response = send(
        app.router(),
        Method::POST,
        "/api/courses",
        Some(json!({"title":"History course"})),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let owner_course: Value = json_body(response).await;
    let course_id = owner_course["id"].as_str().unwrap().to_owned();

    let response = send(
        app.router(),
        Method::POST,
        "/api/courses",
        Some(json!({"title":"Other course"})),
        Some(&teacher_b),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let other_course: Value = json_body(response).await;
    let other_course_id = other_course["id"].as_str().unwrap();

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/students"),
        Some(json!({"student_no":"student-a-no"})),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let roster_entry: Value = json_body(response).await;
    let student_id = roster_entry["user_id"].as_str().unwrap().to_owned();
    assert_eq!(roster_entry["student_no"], "student-a-no");

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/students"),
        Some(json!({"student_no":"student-a-no"})),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/students"),
        Some(json!({"student_no":"student-b-no","user_id":student_id})),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{other_course_id}/students"),
        Some(json!({"student_no":"student-a-no"})),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/students"),
        Some(json!({"student_no":"student-b-no"})),
        Some(&teacher_b),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}/students"),
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/students"),
        Some(json!({"student_no":"student-b-no"})),
        Some(&student_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = send(
        app.router(),
        Method::DELETE,
        &format!("/api/courses/{course_id}/students/{student_id}"),
        None,
        Some(&student_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/students"),
        Some(json!({"student_no":"not-a-registered-student"})),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}/students?limit=1&offset=0"),
        None,
        Some(&teacher_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let roster: Value = json_body(response).await;
    assert_eq!(roster["total"], 1);
    assert_eq!(roster["items"][0]["user_id"], student_id);

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/lessons"),
        Some(lesson_body("Frozen lesson")),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let first_lesson: Value = json_body(response).await;
    let first_lesson_id = first_lesson["id"].as_str().unwrap().to_owned();

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/lessons"),
        Some(lesson_body("Not snapshotted")),
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let second_lesson: Value = json_body(response).await;
    let second_lesson_id = second_lesson["id"].as_str().unwrap().to_owned();
    let response = send(
        app.router(),
        Method::GET,
        "/api/courses?limit=10&offset=0",
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let courses_page: Value = json_body(response).await;
    assert_eq!(courses_page["total"], 1);
    assert_eq!(courses_page["items"][0]["id"], course_id);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}/lessons?limit=10"),
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let lessons_page: Value = json_body(response).await;
    assert_eq!(lessons_page["total"], 2);

    let stage_id = Uuid::new_v4().to_string();
    let opens_at = Utc::now().timestamp_millis();
    let closes_at = opens_at + 60_000;
    sqlx::query("INSERT INTO lesson_students (lesson_id, user_id) VALUES (?, ?)")
        .bind(&first_lesson_id)
        .bind(&student_id)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO stages (id, lesson_id, ordinal, kind, opens_at, closes_at, closed_at, latitude, longitude, radius_m) \
         VALUES (?, ?, 0, 'check_in', ?, ?, NULL, 31.23, 121.47, 100.0)",
    )
    .bind(&stage_id).bind(&first_lesson_id).bind(opens_at).bind(closes_at)
    .execute(&app.pool).await.unwrap();
    let token_hash = [7_u8; 32];
    sqlx::query("INSERT INTO qr_challenges (stage_id, slot, token_hash, token, expires_at) VALUES (?, 0, ?, ?, ?)")
        .bind(&stage_id).bind(token_hash.as_slice()).bind("0123456789abcdefghijklmnopqrstuv")
        .bind(closes_at).execute(&app.pool).await.unwrap();

    let response = send(
        app.router(),
        Method::DELETE,
        &format!("/api/courses/{course_id}/students/{student_id}"),
        None,
        Some(&teacher_a),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}/students?limit=10"),
        None,
        Some(&teacher_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let roster: Value = json_body(response).await;
    assert_eq!(roster["total"], 0);

    let response = send(
        app.router(),
        Method::GET,
        "/api/courses?limit=10&offset=0",
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let courses_page: Value = json_body(response).await;
    assert_eq!(courses_page["total"], 1);
    assert_eq!(courses_page["items"][0]["id"], course_id);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}"),
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}/lessons?limit=10"),
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let lesson_page: Value = json_body(response).await;
    assert_eq!(lesson_page["total"], 1);
    assert_eq!(lesson_page["items"][0]["id"], first_lesson_id);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/lessons/{first_lesson_id}"),
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let details: Value = json_body(response).await;
    assert_eq!(details["stages"][0]["id"], stage_id);
    assert_eq!(details["stages"][0]["ordinal"], 0);
    assert!(details.get("qr_token").is_none());
    assert!(
        details
            .to_string()
            .find("0123456789abcdefghijklmnopqrstuv")
            .is_none()
    );

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/lessons/{second_lesson_id}"),
        None,
        Some(&student_a),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{course_id}"),
        None,
        Some(&teacher_b),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        app.router(),
        Method::GET,
        &format!("/api/courses/{other_course_id}"),
        None,
        Some(&student_b),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn lesson_creation_rejects_invalid_ranges_and_never_accepts_client_actor_ids() {
    let app = TestApp::new().await;
    register(&app, "teacher", "teacher", None).await;
    register(&app, "student", "student", Some("s-1")).await;
    let teacher = login(&app, "teacher").await;
    let student = login(&app, "student").await;

    let response = send(
        app.router(),
        Method::POST,
        "/api/courses",
        Some(json!({"title":"Owned","teacher_id":Uuid::new_v4()})),
        Some(&teacher),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let response = send(
        app.router(),
        Method::POST,
        "/api/courses",
        Some(json!({"title":"Owned"})),
        Some(&teacher),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let course: Value = json_body(response).await;
    let course_id = course["id"].as_str().unwrap();

    let now = Utc::now();
    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/lessons"),
        Some(json!({
            "title":"Bad range",
            "starts_at":now.to_rfc3339(),
            "ends_at":now.to_rfc3339(),
        })),
        Some(&teacher),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let response = send(
        app.router(),
        Method::POST,
        "/api/courses",
        Some(json!({"title":"Student attempt"})),
        Some(&student),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = send(
        app.router(),
        Method::POST,
        &format!("/api/courses/{course_id}/lessons"),
        Some(json!({
            "title":"Injected actor",
            "starts_at":(now - chrono::Duration::minutes(1)).to_rfc3339(),
            "ends_at":(now + chrono::Duration::minutes(1)).to_rfc3339(),
            "teacher_id":Uuid::new_v4(),
        })),
        Some(&teacher),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
