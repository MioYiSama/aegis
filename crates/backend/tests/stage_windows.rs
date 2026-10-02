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
use chrono::{Duration, SecondsFormat, Utc};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN_VALUE: &str = "http://localhost:3000";
const PASSWORD: &str = "stage-test-password-123";

struct TestApp {
    pool: SqlitePool,
    router: Router,
    _directory: tempfile::TempDir,
}

impl TestApp {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("stage-windows.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
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
        let router = Router::new()
            .merge(auth::router(pool.clone(), config.clone()))
            .merge(courses::router(pool.clone(), config));
        Self {
            pool,
            router,
            _directory: directory,
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        cookie: Option<&str>,
    ) -> Response<Body> {
        let has_body = body.is_some();
        let needs_origin =
            method != Method::GET && method != Method::HEAD && method != Method::OPTIONS;
        let mut builder = Request::builder().method(method).uri(path);
        let headers = builder.headers_mut().unwrap();
        if has_body {
            headers.insert("content-type", "application/json".parse().unwrap());
        }
        if let Some(cookie) = cookie {
            headers.insert(COOKIE, cookie.parse().unwrap());
        }
        if needs_origin {
            headers.insert(ORIGIN, ORIGIN_VALUE.parse().unwrap());
        }
        self.router
            .clone()
            .oneshot(
                builder
                    .body(body.map(Body::from).unwrap_or_else(Body::empty))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn json<T: DeserializeOwned>(response: Response<Body>) -> T {
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn register(&self, username: &str, role: &str, student_no: Option<&str>) -> Value {
        let mut input = json!({
            "username": username,
            "password": PASSWORD,
            "display_name": format!("Display {username}"),
            "role": role,
        });
        if let Some(student_no) = student_no {
            input["student_no"] = json!(student_no);
        }
        let response = self
            .send(
                Method::POST,
                "/api/auth/register",
                Some(input.to_string()),
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        Self::json(response).await
    }

    async fn login(&self, username: &str) -> String {
        let response = self
            .send(
                Method::POST,
                "/api/auth/login",
                Some(json!({ "username": username, "password": PASSWORD }).to_string()),
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    async fn create_course(&self, teacher_cookie: &str) -> Uuid {
        let response = self
            .send(
                Method::POST,
                "/api/courses",
                Some(json!({ "title": "Window testing" }).to_string()),
                Some(teacher_cookie),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = Self::json(response).await;
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
    }

    async fn enroll(&self, course_id: Uuid, teacher_cookie: &str, student_no: &str) -> StatusCode {
        self.send(
            Method::POST,
            &format!("/api/courses/{course_id}/students"),
            Some(json!({ "student_no": student_no }).to_string()),
            Some(teacher_cookie),
        )
        .await
        .status()
    }

    async fn remove_student(
        &self,
        course_id: Uuid,
        user_id: Uuid,
        teacher_cookie: &str,
    ) -> StatusCode {
        self.send(
            Method::DELETE,
            &format!("/api/courses/{course_id}/students/{user_id}"),
            None,
            Some(teacher_cookie),
        )
        .await
        .status()
    }

    async fn create_lesson(
        &self,
        course_id: Uuid,
        teacher_cookie: &str,
        starts_at: chrono::DateTime<Utc>,
        ends_at: chrono::DateTime<Utc>,
    ) -> Uuid {
        let response = self
            .send(
                Method::POST,
                &format!("/api/courses/{course_id}/lessons"),
                Some(
                    json!({
                        "title": "Window lesson",
                        "starts_at": starts_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                        "ends_at": ends_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    })
                    .to_string(),
                ),
                Some(teacher_cookie),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = Self::json(response).await;
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
    }

    async fn stage(
        &self,
        lesson_id: Uuid,
        teacher_cookie: &str,
        kind: &str,
        duration_seconds: i64,
    ) -> Response<Body> {
        self.send(
            Method::POST,
            &format!("/api/lessons/{lesson_id}/stages"),
            Some(
                json!({
                    "kind": kind,
                    "duration_seconds": duration_seconds,
                    "latitude": 31.2304,
                    "longitude": 121.4737,
                    "radius_m": 100.0,
                })
                .to_string(),
            ),
            Some(teacher_cookie),
        )
        .await
    }
}

#[tokio::test]
async fn stage_order_roster_snapshot_and_checkout_terminal_are_enforced() {
    let app = TestApp::new().await;
    app.register("stage-teacher", "teacher", None).await;
    let student_a = app
        .register("stage-student-a", "student", Some("stage-a"))
        .await;
    let student_b = app
        .register("stage-student-b", "student", Some("stage-b"))
        .await;
    let teacher_cookie = app.login("stage-teacher").await;
    let student_a_cookie = app.login("stage-student-a").await;
    let course_id = app.create_course(&teacher_cookie).await;
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher_cookie,
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::minutes(30),
        )
        .await;

    let empty_roster = app.stage(lesson_id, &teacher_cookie, "check_in", 60).await;
    assert_eq!(empty_roster.status(), StatusCode::CONFLICT);
    let stage_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stages WHERE lesson_id = ?")
        .bind(lesson_id.to_string())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let snapshot_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM lesson_students WHERE lesson_id = ?")
            .bind(lesson_id.to_string())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!((stage_count, snapshot_count), (0, 0));

    assert_eq!(
        app.enroll(course_id, &teacher_cookie, "stage-a").await,
        StatusCode::CREATED
    );
    let prestart_lesson = app
        .create_lesson(
            course_id,
            &teacher_cookie,
            Utc::now() + Duration::minutes(1),
            Utc::now() + Duration::minutes(2),
        )
        .await;
    assert_eq!(
        app.stage(prestart_lesson, &teacher_cookie, "check_in", 60)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let invalid_duration = app.stage(lesson_id, &teacher_cookie, "check_in", 0).await;
    assert_eq!(invalid_duration.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_coordinates = app
        .send(
            Method::POST,
            &format!("/api/lessons/{lesson_id}/stages"),
            Some(
                json!({
                    "kind": "check_in",
                    "duration_seconds": 60,
                    "latitude": 91.0,
                    "longitude": 121.4737,
                    "radius_m": 100.0,
                })
                .to_string(),
            ),
            Some(&teacher_cookie),
        )
        .await;
    assert_eq!(
        invalid_coordinates.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let invalid_radius = app
        .send(
            Method::POST,
            &format!("/api/lessons/{lesson_id}/stages"),
            Some(
                json!({
                    "kind": "check_in",
                    "duration_seconds": 60,
                    "latitude": 31.2304,
                    "longitude": 121.4737,
                    "radius_m": 1_001.0,
                })
                .to_string(),
            ),
            Some(&teacher_cookie),
        )
        .await;
    assert_eq!(invalid_radius.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let check_in_response = app.stage(lesson_id, &teacher_cookie, "check_in", 60).await;
    assert_eq!(check_in_response.status(), StatusCode::CREATED);
    let check_in: Value = TestApp::json(check_in_response).await;
    let check_in_id = Uuid::parse_str(check_in["id"].as_str().unwrap()).unwrap();
    assert_eq!(check_in["ordinal"], 0);

    assert_eq!(
        app.enroll(course_id, &teacher_cookie, "stage-b").await,
        StatusCode::CREATED
    );
    assert_eq!(
        app.remove_student(
            course_id,
            Uuid::parse_str(student_a["id"].as_str().unwrap()).unwrap(),
            &teacher_cookie
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let snapshot_users: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM lesson_students WHERE lesson_id = ? ORDER BY user_id",
    )
    .bind(lesson_id.to_string())
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        snapshot_users,
        vec![student_a["id"].as_str().unwrap().to_owned()]
    );
    assert_ne!(snapshot_users[0], student_b["id"].as_str().unwrap());

    assert_eq!(
        app.stage(lesson_id, &teacher_cookie, "renew", 60)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.stage(lesson_id, &teacher_cookie, "check_in", 60)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let closed_once = app
        .send(
            Method::POST,
            &format!("/api/stages/{check_in_id}/close"),
            None,
            Some(&teacher_cookie),
        )
        .await;
    assert_eq!(closed_once.status(), StatusCode::OK);
    let closed_once: Value = TestApp::json(closed_once).await;
    assert!(closed_once["closed_at"].is_string());
    assert_eq!(closed_once["closes_at"], closed_once["closed_at"]);
    let closed_twice = app
        .send(
            Method::POST,
            &format!("/api/stages/{check_in_id}/close"),
            None,
            Some(&teacher_cookie),
        )
        .await;
    assert_eq!(closed_twice.status(), StatusCode::OK);
    let closed_twice: Value = TestApp::json(closed_twice).await;
    assert_eq!(closed_once["closes_at"], closed_twice["closes_at"]);
    assert_eq!(closed_once["closed_at"], closed_twice["closed_at"]);

    let renew_response = app.stage(lesson_id, &teacher_cookie, "renew", 60).await;
    assert_eq!(renew_response.status(), StatusCode::CREATED);
    let renew: Value = TestApp::json(renew_response).await;
    assert_eq!(renew["ordinal"], 1);
    let renew_id = Uuid::parse_str(renew["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        app.send(
            Method::POST,
            &format!("/api/stages/{renew_id}/close"),
            None,
            Some(&teacher_cookie)
        )
        .await
        .status(),
        StatusCode::OK
    );

    let checkout_response = app.stage(lesson_id, &teacher_cookie, "check_out", 60).await;
    assert_eq!(checkout_response.status(), StatusCode::CREATED);
    let checkout: Value = TestApp::json(checkout_response).await;
    assert_eq!(checkout["ordinal"], 2);
    assert_eq!(
        app.stage(lesson_id, &teacher_cookie, "renew", 60)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.stage(lesson_id, &teacher_cookie, "check_out", 60)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let removed_student_still_sees_frozen_lesson = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}"),
            None,
            Some(&student_a_cookie),
        )
        .await;
    assert_eq!(
        removed_student_still_sees_frozen_lesson.status(),
        StatusCode::OK
    );
    let short_lesson = app
        .create_lesson(
            course_id,
            &teacher_cookie,
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::seconds(30),
        )
        .await;
    let short_lesson_end: i64 = sqlx::query_scalar("SELECT ends_at FROM lessons WHERE id = ?")
        .bind(short_lesson.to_string())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let capped_stage_response = app
        .stage(short_lesson, &teacher_cookie, "check_in", 900)
        .await;
    assert_eq!(capped_stage_response.status(), StatusCode::CREATED);
    let capped_stage: Value = TestApp::json(capped_stage_response).await;
    let capped_end =
        chrono::DateTime::parse_from_rfc3339(capped_stage["closes_at"].as_str().unwrap())
            .unwrap()
            .timestamp_millis();
    assert_eq!(capped_end, short_lesson_end);
}

#[tokio::test]
async fn qr_image_is_teacher_only_slot_stable_expiring_and_not_cached() {
    let app = TestApp::new().await;
    app.register("qr-teacher", "teacher", None).await;
    app.register("qr-student", "student", Some("qr-student-no"))
        .await;
    let teacher_cookie = app.login("qr-teacher").await;
    let student_cookie = app.login("qr-student").await;
    let course_id = app.create_course(&teacher_cookie).await;
    assert_eq!(
        app.enroll(course_id, &teacher_cookie, "qr-student-no")
            .await,
        StatusCode::CREATED
    );
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher_cookie,
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::minutes(30),
        )
        .await;
    let stage_response = app.stage(lesson_id, &teacher_cookie, "check_in", 120).await;
    assert_eq!(stage_response.status(), StatusCode::CREATED);
    let stage: Value = TestApp::json(stage_response).await;
    let stage_id = Uuid::parse_str(stage["id"].as_str().unwrap()).unwrap();
    let now = Utc::now().timestamp_millis();
    let aligned_open = now - 2_500;
    sqlx::query("UPDATE stages SET opens_at = ? WHERE id = ?")
        .bind(aligned_open)
        .bind(stage_id.to_string())
        .execute(&app.pool)
        .await
        .unwrap();

    let denied = app
        .send(
            Method::GET,
            &format!("/api/stages/{stage_id}/qr"),
            None,
            Some(&student_cookie),
        )
        .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let before_slot = (Utc::now().timestamp_millis() - aligned_open) / 5_000;
    let qr_path = format!("/api/stages/{stage_id}/qr");
    let (first, second) = tokio::join!(
        app.send(Method::GET, &qr_path, None, Some(&teacher_cookie)),
        app.send(Method::GET, &qr_path, None, Some(&teacher_cookie)),
    );
    let completed_slot = (Utc::now().timestamp_millis() - aligned_open) / 5_000;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(first.headers()["content-type"], "image/png");
    assert_eq!(first.headers()["cache-control"], "no-store");
    let expires = first
        .headers()
        .get("X-QR-Expires-At")
        .unwrap()
        .to_str()
        .unwrap();
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires)
        .unwrap()
        .timestamp_millis();
    assert!(expires_at > Utc::now().timestamp_millis());
    let stage_closes = chrono::DateTime::parse_from_rfc3339(stage["closes_at"].as_str().unwrap())
        .unwrap()
        .timestamp_millis();
    assert!(expires_at <= stage_closes);

    let first_png = to_bytes(first.into_body(), 1024 * 1024).await.unwrap();
    let second_png = to_bytes(second.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(first_png, second_png);
    let image = image::load_from_memory(&first_png).unwrap().into_rgb8();
    let token = aegis_core::qr::decode_chroma(&image).unwrap();
    assert_eq!(token.len(), 32);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    );

    let (slot, stored_token, stored_expiry): (i64, String, i64) =
        sqlx::query_as("SELECT slot, token, expires_at FROM qr_challenges WHERE stage_id = ?")
            .bind(stage_id.to_string())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored_token, token);
    assert_eq!(stored_expiry, expires_at);
    assert_eq!(slot, before_slot);
    assert_eq!(slot, completed_slot);
    let stored_hash: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM qr_challenges WHERE stage_id = ? AND slot = ?")
            .bind(stage_id.to_string())
            .bind(slot)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored_hash, Sha256::digest(token.as_bytes()).to_vec());

    assert_eq!(
        app.send(
            Method::POST,
            &format!("/api/stages/{stage_id}/close"),
            None,
            Some(&teacher_cookie)
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.send(
            Method::GET,
            &format!("/api/stages/{stage_id}/qr"),
            None,
            Some(&teacher_cookie)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn lesson_close_requires_checkin_and_closes_the_open_stage_window() {
    let app = TestApp::new().await;
    app.register("close-teacher", "teacher", None).await;
    app.register("close-student", "student", Some("close-student-no"))
        .await;
    let teacher_cookie = app.login("close-teacher").await;
    let course_id = app.create_course(&teacher_cookie).await;
    assert_eq!(
        app.enroll(course_id, &teacher_cookie, "close-student-no")
            .await,
        StatusCode::CREATED
    );
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher_cookie,
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::minutes(30),
        )
        .await;

    assert_eq!(
        app.send(
            Method::POST,
            &format!("/api/lessons/{lesson_id}/close"),
            None,
            Some(&teacher_cookie)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let check_in_response = app.stage(lesson_id, &teacher_cookie, "check_in", 120).await;
    assert_eq!(check_in_response.status(), StatusCode::CREATED);
    let check_in: Value = TestApp::json(check_in_response).await;
    let check_in_id = Uuid::parse_str(check_in["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        app.send(
            Method::POST,
            &format!("/api/stages/{check_in_id}/close"),
            None,
            Some(&teacher_cookie)
        )
        .await
        .status(),
        StatusCode::OK
    );
    let renew_response = app.stage(lesson_id, &teacher_cookie, "renew", 120).await;
    assert_eq!(renew_response.status(), StatusCode::CREATED);
    let renew: Value = TestApp::json(renew_response).await;
    let renew_id = Uuid::parse_str(renew["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        app.send(
            Method::GET,
            &format!("/api/stages/{renew_id}/qr"),
            None,
            Some(&teacher_cookie)
        )
        .await
        .status(),
        StatusCode::CONFLICT,
    );

    let close_response = app
        .send(
            Method::POST,
            &format!("/api/lessons/{lesson_id}/close"),
            None,
            Some(&teacher_cookie),
        )
        .await;
    assert_eq!(close_response.status(), StatusCode::OK);
    let closed_lesson: Value = TestApp::json(close_response).await;
    assert!(closed_lesson["closed_at"].is_string());
    let closed_stage: (Option<i64>, i64) =
        sqlx::query_as("SELECT closed_at, closes_at FROM stages WHERE id = ?")
            .bind(renew_id.to_string())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(closed_stage.0.is_some());
    assert_eq!(closed_stage.0, Some(closed_stage.1));
    assert_eq!(
        app.stage(lesson_id, &teacher_cookie, "check_out", 60)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let repeated_close = app
        .send(
            Method::POST,
            &format!("/api/lessons/{lesson_id}/close"),
            None,
            Some(&teacher_cookie),
        )
        .await;
    assert_eq!(repeated_close.status(), StatusCode::OK);
    let repeated_lesson: Value = TestApp::json(repeated_close).await;
    assert_eq!(closed_lesson["closed_at"], repeated_lesson["closed_at"]);
}

#[tokio::test]
async fn concurrent_first_stage_requests_freeze_roster_only_once() {
    let app = TestApp::new().await;
    app.register("race-teacher", "teacher", None).await;
    app.register("race-student", "student", Some("race-student-no"))
        .await;
    let teacher_cookie = app.login("race-teacher").await;
    let course_id = app.create_course(&teacher_cookie).await;
    assert_eq!(
        app.enroll(course_id, &teacher_cookie, "race-student-no")
            .await,
        StatusCode::CREATED
    );
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher_cookie,
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::minutes(30),
        )
        .await;

    let (first, second) = tokio::join!(
        app.stage(lesson_id, &teacher_cookie, "check_in", 60),
        app.stage(lesson_id, &teacher_cookie, "check_in", 60),
    );
    let first_status = first.status();
    let second_status = second.status();
    assert!(
        (first_status == StatusCode::CREATED && second_status == StatusCode::CONFLICT)
            || (first_status == StatusCode::CONFLICT && second_status == StatusCode::CREATED)
    );
    let (stage_count, roster_count): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM stages WHERE lesson_id = ?), \
                (SELECT COUNT(*) FROM lesson_students WHERE lesson_id = ?)",
    )
    .bind(lesson_id.to_string())
    .bind(lesson_id.to_string())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((stage_count, roster_count), (1, 1));
}
