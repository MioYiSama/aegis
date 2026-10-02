use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use aegis_backend::{auth, config::Config, courses, reviews};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Method, Request, Response, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, COOKIE, ORIGIN, SET_COOKIE},
    },
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{Value, json};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN_VALUE: &str = "http://localhost:3000";
const PASSWORD: &str = "review-test-password-123";
const PDF_EVIDENCE: &[u8] = b"%PDF-1.7\nprivate evidence\n";

struct TestApp {
    pool: SqlitePool,
    router: Router,
    _directory: tempfile::TempDir,
}

impl TestApp {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("review-transactions.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
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
            .merge(courses::router(pool.clone(), config.clone()))
            .merge(reviews::router(pool.clone(), config));
        Self {
            pool,
            router,
            _directory: directory,
        }
    }

    async fn register(&self, username: &str, role: &str, student_no: Option<&str>) -> Uuid {
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
            .send_json(Method::POST, "/api/auth/register", Some(input), None)
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = json_body(response).await;
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
    }

    async fn login(&self, username: &str) -> String {
        let response = self
            .send_json(
                Method::POST,
                "/api/auth/login",
                Some(json!({ "username": username, "password": PASSWORD })),
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

    async fn create_course(&self, teacher_cookie: &str, title: &str) -> Uuid {
        let response = self
            .send_json(
                Method::POST,
                "/api/courses",
                Some(json!({ "title": title })),
                Some(teacher_cookie),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        Uuid::parse_str(json_body(response).await["id"].as_str().unwrap()).unwrap()
    }

    async fn enroll(&self, course_id: Uuid, teacher_cookie: &str, student_no: &str) -> StatusCode {
        self.send_json(
            Method::POST,
            &format!("/api/courses/{course_id}/students"),
            Some(json!({ "student_no": student_no })),
            Some(teacher_cookie),
        )
        .await
        .status()
    }

    async fn remove_student(
        &self,
        course_id: Uuid,
        student_id: Uuid,
        teacher_cookie: &str,
    ) -> StatusCode {
        self.send(
            Method::DELETE,
            &format!("/api/courses/{course_id}/students/{student_id}"),
            Body::empty(),
            None,
            Some(teacher_cookie),
            None,
        )
        .await
        .status()
    }

    async fn create_lesson(
        &self,
        course_id: Uuid,
        teacher_cookie: &str,
        title: &str,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> Uuid {
        let response = self
            .send_json(
                Method::POST,
                &format!("/api/courses/{course_id}/lessons"),
                Some(json!({
                    "title": title,
                    "starts_at": starts_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    "ends_at": ends_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                })),
                Some(teacher_cookie),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        Uuid::parse_str(json_body(response).await["id"].as_str().unwrap()).unwrap()
    }

    async fn send_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        cookie: Option<&str>,
    ) -> Response<Body> {
        let has_json = body.is_some();
        let body = body
            .map(|value| Body::from(value.to_string()))
            .unwrap_or_else(Body::empty);
        self.send(method, path, body, Some(has_json), cookie, None)
            .await
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Body,
        json_content: Option<bool>,
        cookie: Option<&str>,
        content_type: Option<&str>,
    ) -> Response<Body> {
        let mut builder = Request::builder().method(method.clone()).uri(path);
        let headers = builder.headers_mut().unwrap();
        if json_content == Some(true) {
            headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        }
        if let Some(content_type) = content_type {
            headers.insert(CONTENT_TYPE, content_type.parse().unwrap());
        }
        if let Some(cookie) = cookie {
            headers.insert(COOKIE, cookie.parse().unwrap());
        }
        if method != Method::GET && method != Method::HEAD && method != Method::OPTIONS {
            headers.insert(ORIGIN, ORIGIN_VALUE.parse().unwrap());
        }
        self.router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn send_multipart(
        &self,
        path: &str,
        cookie: &str,
        reason: &str,
        evidence: &[u8],
    ) -> Response<Body> {
        const BOUNDARY: &str = "AegisReviewEvidenceBoundary";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"reason\"\r\nContent-Type: text/plain\r\n\r\n{reason}\r\n"
        ).as_bytes());
        bytes.extend_from_slice(format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"evidence\"; filename=\"evidence.pdf\"\r\nContent-Type: application/pdf\r\n\r\n"
        ).as_bytes());
        bytes.extend_from_slice(evidence);
        bytes.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        self.send(
            Method::POST,
            path,
            Body::from(bytes),
            None,
            Some(cookie),
            Some(&format!("multipart/form-data; boundary={BOUNDARY}")),
        )
        .await
    }
    async fn send_multipart_reason_only(
        &self,
        path: &str,
        cookie: &str,
        reason: &str,
    ) -> Response<Body> {
        const BOUNDARY: &str = "AegisMissingEvidenceBoundary";
        let bytes = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"reason\"\r\nContent-Type: text/plain\r\n\r\n{reason}\r\n--{BOUNDARY}--\r\n"
        );
        self.send(
            Method::POST,
            path,
            Body::from(bytes),
            None,
            Some(cookie),
            Some(&format!("multipart/form-data; boundary={BOUNDARY}")),
        )
        .await
    }
}

async fn json_body(response: Response<Body>) -> Value {
    let bytes = to_bytes(response.into_body(), 12 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn create_attempt_review(
    app: &TestApp,
    attempt_id: Uuid,
    cookie: &str,
    reason: &str,
) -> Response<Body> {
    app.send_json(
        Method::POST,
        &format!("/api/attempts/{attempt_id}/review"),
        Some(json!({ "reason": reason })),
        Some(cookie),
    )
    .await
}

async fn decide(
    app: &TestApp,
    review_id: Uuid,
    teacher_cookie: &str,
    decision: &str,
) -> Response<Body> {
    app.send_json(
        Method::POST,
        &format!("/api/reviews/{review_id}/decision"),
        Some(json!({ "decision": decision })),
        Some(teacher_cookie),
    )
    .await
}

async fn create_stage(
    app: &TestApp,
    lesson_id: Uuid,
    kind: &str,
    ordinal: i64,
    snapshot: bool,
) -> Uuid {
    let stage_id = Uuid::new_v4();
    let now = Utc::now().timestamp_millis();
    let mut tx = app.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query(
        "INSERT INTO stages (id, lesson_id, ordinal, kind, opens_at, closes_at, closed_at, latitude, longitude, radius_m) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, 31.2304, 121.4737, 100.0)",
    )
    .bind(stage_id.to_string())
    .bind(lesson_id.to_string())
    .bind(ordinal)
    .bind(kind)
    .bind(now - 60_000)
    .bind(now + 60_000)
    .execute(&mut *tx)
    .await
    .unwrap();
    if snapshot {
        sqlx::query(
            "INSERT INTO lesson_students (lesson_id, user_id) \
             SELECT ?, user_id FROM enrollments WHERE course_id = (SELECT course_id FROM lessons WHERE id = ?)",
        )
        .bind(lesson_id.to_string())
        .bind(lesson_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    stage_id
}

async fn seed_attempt(
    app: &TestApp,
    stage_id: Uuid,
    user_id: Uuid,
    qr_pass: Option<i64>,
    location_pass: bool,
    face_pass: bool,
    is_late: bool,
    outcome: &str,
    result_status: Option<&str>,
) -> Uuid {
    let attempt_id = Uuid::new_v4();
    let submitted_at = Utc::now().timestamp_millis();
    let mut tx = app.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query(
        "INSERT INTO attempts (id, stage_id, user_id, submitted_at, qr_pass, location_pass, face_pass, is_late, outcome, reason_codes) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '[]')",
    )
    .bind(attempt_id.to_string())
    .bind(stage_id.to_string())
    .bind(user_id.to_string())
    .bind(submitted_at)
    .bind(qr_pass)
    .bind(if location_pass { 1_i64 } else { 0_i64 })
    .bind(if face_pass { 1_i64 } else { 0_i64 })
    .bind(if is_late { 1_i64 } else { 0_i64 })
    .bind(outcome)
    .execute(&mut *tx)
    .await
    .unwrap();
    if let Some(status) = result_status {
        sqlx::query(
            "INSERT INTO stage_results (stage_id, user_id, status, attempt_id, review_id, updated_at) \
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(stage_id.to_string())
        .bind(user_id.to_string())
        .bind(status)
        .bind(attempt_id.to_string())
        .bind(submitted_at)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    attempt_id
}

fn student_row<'a>(body: &'a Value, student_id: Uuid) -> &'a Value {
    body["students"]
        .as_array()
        .unwrap()
        .iter()
        .find(|student| student["user_id"] == student_id.to_string())
        .unwrap()
}

#[tokio::test]
async fn reviews_are_private_idempotent_and_approved_late_is_the_only_late_flag() {
    let app = TestApp::new().await;
    app.register("review-teacher", "teacher", None).await;
    app.register("review-other-teacher", "teacher", None).await;
    let student_a_id = app
        .register("review-student-a", "student", Some("review-a"))
        .await;
    let student_b_id = app
        .register("review-student-b", "student", Some("review-b"))
        .await;
    let teacher = app.login("review-teacher").await;
    let other_teacher = app.login("review-other-teacher").await;
    let student_a = app.login("review-student-a").await;
    let student_b = app.login("review-student-b").await;
    let course_id = app.create_course(&teacher, "Review decisions").await;
    assert_eq!(
        app.enroll(course_id, &teacher, "review-a").await,
        StatusCode::CREATED
    );
    assert_eq!(
        app.enroll(course_id, &teacher, "review-b").await,
        StatusCode::CREATED
    );
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher,
            "Completed reviews",
            Utc::now() - Duration::minutes(20),
            Utc::now() - Duration::minutes(1),
        )
        .await;
    let check_in = create_stage(&app, lesson_id, "check_in", 0, true).await;
    let attempt_a = seed_attempt(
        &app,
        check_in,
        student_a_id,
        Some(1),
        true,
        false,
        false,
        "reviewable",
        Some("failed"),
    )
    .await;
    let attempt_b = seed_attempt(
        &app,
        check_in,
        student_b_id,
        Some(1),
        true,
        false,
        true,
        "reviewable",
        Some("failed"),
    )
    .await;

    let created_a = create_attempt_review(&app, attempt_a, &student_a, "Network outage").await;
    assert_eq!(created_a.status(), StatusCode::CREATED);
    let review_a_body = json_body(created_a).await;
    let review_a_id = Uuid::parse_str(review_a_body["id"].as_str().unwrap()).unwrap();
    assert_eq!(review_a_body["kind"], "partial");
    assert_eq!(review_a_body["status"], "pending");
    assert_eq!(review_a_body["has_evidence"], false);
    let repeated =
        create_attempt_review(&app, attempt_a, &student_a, "different repeated reason").await;
    assert_eq!(repeated.status(), StatusCode::OK);
    assert_eq!(json_body(repeated).await["id"], review_a_id.to_string());
    let other_attempt_same_stage = seed_attempt(
        &app,
        check_in,
        student_a_id,
        Some(1),
        true,
        false,
        false,
        "reviewable",
        None,
    )
    .await;
    assert_eq!(
        create_attempt_review(
            &app,
            other_attempt_same_stage,
            &student_a,
            "Second pending request"
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );

    let created_b =
        create_attempt_review(&app, attempt_b, &student_b, "Arrived after QR closed").await;
    assert_eq!(created_b.status(), StatusCode::CREATED);
    let review_b_id = Uuid::parse_str(json_body(created_b).await["id"].as_str().unwrap()).unwrap();
    let own_list = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/reviews"),
            Body::empty(),
            None,
            Some(&student_a),
            None,
        )
        .await;
    assert_eq!(own_list.status(), StatusCode::OK);
    let own_list: Value = json_body(own_list).await;
    assert_eq!(own_list["total"], 1);
    assert_eq!(own_list["items"][0]["id"], review_a_id.to_string());
    assert!(own_list["items"][0].get("evidence_bytes").is_none());
    let teacher_list = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/reviews?limit=1&offset=0"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(teacher_list.status(), StatusCode::OK);
    let teacher_list: Value = json_body(teacher_list).await;
    assert_eq!(teacher_list["total"], 2);
    assert_eq!(teacher_list["items"].as_array().unwrap().len(), 1);
    assert!(teacher_list["items"][0].get("evidence_bytes").is_none());
    let hidden_list = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/reviews"),
            Body::empty(),
            None,
            Some(&other_teacher),
            None,
        )
        .await;
    assert_eq!(hidden_list.status(), StatusCode::NOT_FOUND);
    let hidden_evidence = app
        .send(
            Method::GET,
            &format!("/api/reviews/{review_a_id}/evidence"),
            Body::empty(),
            None,
            Some(&student_b),
            None,
        )
        .await;
    assert_eq!(hidden_evidence.status(), StatusCode::NOT_FOUND);
    let no_evidence = app
        .send(
            Method::GET,
            &format!("/api/reviews/{review_a_id}/evidence"),
            Body::empty(),
            None,
            Some(&student_a),
            None,
        )
        .await;
    assert_eq!(no_evidence.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        decide(&app, review_a_id, &other_teacher, "approve")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        decide(&app, review_a_id, &student_a, "approve")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    let before = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(before.status(), StatusCode::OK);
    let before: Value = json_body(before).await;
    assert_eq!(student_row(&before, student_a_id)["status"], "pending");
    assert_eq!(student_row(&before, student_b_id)["status"], "pending");
    assert_eq!(student_row(&before, student_b_id)["is_late"], false);

    let approved_a = decide(&app, review_a_id, &teacher, "approve").await;
    assert_eq!(approved_a.status(), StatusCode::OK);
    assert_eq!(json_body(approved_a).await["status"], "approved");
    assert_eq!(
        decide(&app, review_a_id, &teacher, "approve")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        decide(&app, review_a_id, &teacher, "reject").await.status(),
        StatusCode::CONFLICT
    );
    let approved_b = decide(&app, review_b_id, &teacher, "approve").await;
    assert_eq!(approved_b.status(), StatusCode::OK);

    let another_reviewable_attempt = seed_attempt(
        &app,
        check_in,
        student_a_id,
        Some(1),
        true,
        false,
        false,
        "reviewable",
        None,
    )
    .await;
    assert_eq!(
        create_attempt_review(
            &app,
            another_reviewable_attempt,
            &student_a,
            "Already passed stage"
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let failed_attempt = seed_attempt(
        &app,
        check_in,
        student_a_id,
        Some(0),
        true,
        false,
        false,
        "failed",
        None,
    )
    .await;
    assert_eq!(
        create_attempt_review(&app, failed_attempt, &student_a, "Failed attempt")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let summary = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(summary.status(), StatusCode::OK);
    let summary: Value = json_body(summary).await;
    assert_eq!(student_row(&summary, student_a_id)["status"], "present");
    assert_eq!(
        student_row(&summary, student_a_id)["attendance_success"],
        true
    );
    assert_eq!(student_row(&summary, student_a_id)["is_late"], false);
    assert_eq!(student_row(&summary, student_b_id)["status"], "present");
    assert_eq!(student_row(&summary, student_b_id)["is_late"], true);
    assert_eq!(
        student_row(&summary, student_b_id)["stages"][0]["status"],
        "passed"
    );
}

#[tokio::test]
async fn concurrent_review_creation_and_decisions_are_atomic_and_success_retry_cannot_be_undone() {
    let app = TestApp::new().await;
    app.register("race-teacher", "teacher", None).await;
    let student_id = app
        .register("race-student", "student", Some("race-a"))
        .await;
    let teacher = app.login("race-teacher").await;
    let student = app.login("race-student").await;
    let course_id = app
        .create_course(&teacher, "Concurrent review decisions")
        .await;
    assert_eq!(
        app.enroll(course_id, &teacher, "race-a").await,
        StatusCode::CREATED
    );
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher,
            "Race lesson",
            Utc::now() - Duration::minutes(2),
            Utc::now() + Duration::hours(1),
        )
        .await;
    let check_in = create_stage(&app, lesson_id, "check_in", 0, true).await;
    let attempt_id = seed_attempt(
        &app,
        check_in,
        student_id,
        Some(1),
        true,
        false,
        false,
        "reviewable",
        Some("failed"),
    )
    .await;

    let (first, second) = tokio::join!(
        create_attempt_review(&app, attempt_id, &student, "Concurrent request A"),
        create_attempt_review(&app, attempt_id, &student, "Concurrent request B"),
    );
    let (first_status, first_json) = (first.status(), json_body(first).await);
    let (second_status, second_json) = (second.status(), json_body(second).await);
    assert!([StatusCode::CREATED, StatusCode::OK].contains(&first_status));
    assert!([StatusCode::CREATED, StatusCode::OK].contains(&second_status));
    assert_ne!(first_status, second_status);
    assert_eq!(first_json["id"], second_json["id"]);
    let review_id = Uuid::parse_str(first_json["id"].as_str().unwrap()).unwrap();
    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reviews WHERE lesson_id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(lesson_id.to_string())
    .bind(student_id.to_string())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(pending_count, 1);

    let (approve, reject) = tokio::join!(
        decide(&app, review_id, &teacher, "approve"),
        decide(&app, review_id, &teacher, "reject"),
    );
    let decision_statuses = [approve.status(), reject.status()];
    assert!(decision_statuses.contains(&StatusCode::OK));
    assert!(decision_statuses.contains(&StatusCode::CONFLICT));
    let review_status: String = sqlx::query_scalar("SELECT status FROM reviews WHERE id = ?")
        .bind(review_id.to_string())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let stage_status: String =
        sqlx::query_scalar("SELECT status FROM stage_results WHERE stage_id = ? AND user_id = ?")
            .bind(check_in.to_string())
            .bind(student_id.to_string())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        stage_status,
        if review_status == "approved" {
            "passed"
        } else {
            "failed"
        }
    );
    assert_eq!(
        review_status,
        if decision_statuses[0] == StatusCode::OK {
            "approved"
        } else {
            "rejected"
        }
    );

    let retry_lesson = app
        .create_lesson(
            course_id,
            &teacher,
            "Retry lesson",
            Utc::now() - Duration::minutes(2),
            Utc::now() + Duration::hours(1),
        )
        .await;
    let retry_check_in = create_stage(&app, retry_lesson, "check_in", 0, true).await;
    let failed_attempt = seed_attempt(
        &app,
        retry_check_in,
        student_id,
        Some(1),
        true,
        false,
        false,
        "reviewable",
        Some("failed"),
    )
    .await;
    let pending =
        create_attempt_review(&app, failed_attempt, &student, "Temporary factor failure").await;
    assert_eq!(pending.status(), StatusCode::CREATED);
    let retry_review_id =
        Uuid::parse_str(json_body(pending).await["id"].as_str().unwrap()).unwrap();
    let successful_retry = seed_attempt(
        &app,
        retry_check_in,
        student_id,
        Some(1),
        true,
        true,
        false,
        "passed",
        None,
    )
    .await;
    let now = Utc::now().timestamp_millis();
    let mut tx = app.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query(
        "UPDATE reviews SET status = 'rejected', reviewer_id = ?, reviewed_at = ?, decision_note = 'superseded_by_success' \
         WHERE id = ? AND status = 'pending'",
    )
    .bind(student_id.to_string())
    .bind(now)
    .bind(retry_review_id.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE stage_results SET status = 'passed', attempt_id = ?, review_id = NULL, updated_at = ? \
         WHERE stage_id = ? AND user_id = ?",
    )
    .bind(successful_retry.to_string())
    .bind(now)
    .bind(retry_check_in.to_string())
    .bind(student_id.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        decide(&app, retry_review_id, &teacher, "approve")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        decide(&app, retry_review_id, &teacher, "reject")
            .await
            .status(),
        StatusCode::OK
    );
    let (result_status, result_attempt, decision_note): (String, String, Option<String>) = sqlx::query_as(
        "SELECT sr.status, sr.attempt_id, r.decision_note FROM stage_results sr JOIN reviews r ON r.id = ? \
         WHERE sr.stage_id = ? AND sr.user_id = ?",
    )
    .bind(retry_review_id.to_string())
    .bind(retry_check_in.to_string())
    .bind(student_id.to_string())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(result_status, "passed");
    assert_eq!(result_attempt, successful_retry.to_string());
    assert_eq!(decision_note.as_deref(), Some("superseded_by_success"));
}

#[tokio::test]
async fn leave_evidence_is_private_and_summary_obeys_leave_snapshot_and_all_stage_rules() {
    let app = TestApp::new().await;
    app.register("summary-teacher", "teacher", None).await;
    app.register("summary-other-teacher", "teacher", None).await;
    let student_a_id = app
        .register("summary-student-a", "student", Some("summary-a"))
        .await;
    let student_b_id = app
        .register("summary-student-b", "student", Some("summary-b"))
        .await;
    let student_c_id = app
        .register("summary-student-c", "student", Some("summary-c"))
        .await;
    let student_d_id = app
        .register("summary-student-d", "student", Some("summary-d"))
        .await;
    let teacher = app.login("summary-teacher").await;
    let other_teacher = app.login("summary-other-teacher").await;
    let student_a = app.login("summary-student-a").await;
    let student_b = app.login("summary-student-b").await;
    let student_c = app.login("summary-student-c").await;
    let student_d = app.login("summary-student-d").await;
    let course_id = app.create_course(&teacher, "Roster snapshot course").await;
    for student_no in ["summary-a", "summary-b", "summary-c"] {
        assert_eq!(
            app.enroll(course_id, &teacher, student_no).await,
            StatusCode::CREATED
        );
    }
    let lesson_id = app
        .create_lesson(
            course_id,
            &teacher,
            "Snapshot and leave",
            Utc::now() - Duration::minutes(5),
            Utc::now() + Duration::hours(1),
        )
        .await;
    let empty_lesson = app
        .create_lesson(
            course_id,
            &teacher,
            "No stages",
            Utc::now() - Duration::hours(2),
            Utc::now() - Duration::hours(1),
        )
        .await;
    let empty_summary = app
        .send(
            Method::GET,
            &format!("/api/lessons/{empty_lesson}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(empty_summary.status(), StatusCode::OK);
    let empty_summary: Value = json_body(empty_summary).await;
    assert_eq!(
        student_row(&empty_summary, student_b_id)["status"],
        "absent"
    );
    assert_eq!(
        student_row(&empty_summary, student_b_id)["attendance_success"],
        false
    );
    let pending_leave = app
        .send_multipart(
            &format!("/api/lessons/{empty_lesson}/leave"),
            &student_c,
            "Pending leave",
            PDF_EVIDENCE,
        )
        .await;
    assert_eq!(pending_leave.status(), StatusCode::CREATED);
    let pending_leave_summary = app
        .send(
            Method::GET,
            &format!("/api/lessons/{empty_lesson}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    let pending_leave_summary: Value = json_body(pending_leave_summary).await;
    assert_eq!(
        student_row(&pending_leave_summary, student_c_id)["status"],
        "pending"
    );
    assert_eq!(
        student_row(&pending_leave_summary, student_c_id)["attendance_success"],
        Value::Null
    );

    let missing_evidence = app
        .send_multipart_reason_only(
            &format!("/api/lessons/{lesson_id}/leave"),
            &student_a,
            "Missing attachment",
        )
        .await;
    assert_eq!(missing_evidence.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        app.send_multipart(
            &format!("/api/lessons/{lesson_id}/leave"),
            &student_a,
            "Invalid evidence",
            b"not a PDF"
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let leave_response = app
        .send_multipart(
            &format!("/api/lessons/{lesson_id}/leave"),
            &student_a,
            "Medical leave",
            PDF_EVIDENCE,
        )
        .await;
    assert_eq!(leave_response.status(), StatusCode::CREATED);
    let leave_json = json_body(leave_response).await;
    let leave_id = Uuid::parse_str(leave_json["id"].as_str().unwrap()).unwrap();
    assert_eq!(leave_json["kind"], "leave");
    assert_eq!(leave_json["has_evidence"], true);
    assert!(leave_json.get("evidence_bytes").is_none());
    assert_eq!(
        app.send_multipart(
            &format!("/api/lessons/{lesson_id}/leave"),
            &student_a,
            "Duplicate leave",
            PDF_EVIDENCE
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let approved = decide(&app, leave_id, &teacher, "approve").await;
    assert_eq!(approved.status(), StatusCode::OK);

    let fetched = app
        .send(
            Method::GET,
            &format!("/api/reviews/{leave_id}/evidence"),
            Body::empty(),
            None,
            Some(&student_a),
            None,
        )
        .await;
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        fetched.headers().get(CONTENT_TYPE).unwrap(),
        "application/pdf"
    );
    assert_eq!(
        fetched.headers().get(CONTENT_DISPOSITION).unwrap(),
        "attachment; filename=\"evidence\""
    );
    assert_eq!(
        fetched.headers().get("X-Content-Type-Options").unwrap(),
        "nosniff"
    );
    assert_eq!(fetched.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(
        to_bytes(fetched.into_body(), 1024).await.unwrap().as_ref(),
        PDF_EVIDENCE
    );
    let teacher_evidence = app
        .send(
            Method::GET,
            &format!("/api/reviews/{leave_id}/evidence"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(teacher_evidence.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(teacher_evidence.into_body(), 1024)
            .await
            .unwrap()
            .as_ref(),
        PDF_EVIDENCE
    );
    assert_eq!(
        app.send(
            Method::GET,
            &format!("/api/reviews/{leave_id}/evidence"),
            Body::empty(),
            None,
            Some(&student_b),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(
            Method::GET,
            &format!("/api/reviews/{leave_id}/evidence"),
            Body::empty(),
            None,
            Some(&other_teacher),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let listed = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/reviews?limit=1&offset=0"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: Value = json_body(listed).await;
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["items"][0]["has_evidence"], true);
    assert!(listed["items"][0].get("evidence_bytes").is_none());
    let student_list = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/reviews"),
            Body::empty(),
            None,
            Some(&student_b),
            None,
        )
        .await;
    assert_eq!(json_body(student_list).await["total"], 0);

    let before_checkin = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(before_checkin.status(), StatusCode::OK);
    let before_checkin: Value = json_body(before_checkin).await;
    assert_eq!(before_checkin["students"].as_array().unwrap().len(), 3);
    let excused = student_row(&before_checkin, student_a_id);
    assert_eq!(excused["status"], "excused");
    assert_eq!(excused["attendance_success"], true);
    assert_eq!(excused["stages"].as_array().unwrap().len(), 0);
    assert_eq!(
        student_row(&before_checkin, student_b_id)["status"],
        "in_progress"
    );

    let check_in = create_stage(&app, lesson_id, "check_in", 0, true).await;
    let renew_one = create_stage(&app, lesson_id, "renew", 1, false).await;
    let renew_two = create_stage(&app, lesson_id, "renew", 2, false).await;
    assert_eq!(
        app.remove_student(course_id, student_a_id, &teacher).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.enroll(course_id, &teacher, "summary-d").await,
        StatusCode::CREATED
    );
    let end = Utc::now().timestamp_millis() - 1;
    sqlx::query("UPDATE lessons SET ends_at = ?, closed_at = ? WHERE id = ?")
        .bind(end)
        .bind(end)
        .bind(lesson_id.to_string())
        .execute(&app.pool)
        .await
        .unwrap();

    let pending_attempt = seed_attempt(
        &app,
        check_in,
        student_b_id,
        Some(1),
        true,
        false,
        false,
        "reviewable",
        Some("failed"),
    )
    .await;
    let pending_review =
        create_attempt_review(&app, pending_attempt, &student_b, "Need teacher review").await;
    assert_eq!(pending_review.status(), StatusCode::CREATED);
    let pending_review_id =
        Uuid::parse_str(json_body(pending_review).await["id"].as_str().unwrap()).unwrap();
    let checkin_c = seed_attempt(
        &app,
        check_in,
        student_c_id,
        Some(1),
        true,
        true,
        false,
        "passed",
        Some("passed"),
    )
    .await;
    let renew_one_c = seed_attempt(
        &app,
        renew_one,
        student_c_id,
        None,
        true,
        true,
        false,
        "passed",
        Some("passed"),
    )
    .await;

    let summary = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(summary.status(), StatusCode::OK);
    let summary: Value = json_body(summary).await;
    assert_eq!(summary["students"].as_array().unwrap().len(), 3);
    assert!(
        summary["students"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["user_id"] != student_d_id.to_string())
    );
    assert_eq!(student_row(&summary, student_a_id)["status"], "excused");
    assert_eq!(
        student_row(&summary, student_a_id)["stages"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(student_row(&summary, student_b_id)["status"], "pending");
    assert_eq!(
        student_row(&summary, student_b_id)["attendance_success"],
        Value::Null
    );
    assert_eq!(
        student_row(&summary, student_b_id)["stages"][0]["status"],
        "pending"
    );
    assert_eq!(
        student_row(&summary, student_c_id)["status"],
        "early_departure"
    );
    assert_eq!(
        student_row(&summary, student_c_id)["attendance_success"],
        false
    );
    assert_eq!(
        student_row(&summary, student_c_id)["stages"][0]["status"],
        "passed"
    );
    assert_eq!(
        student_row(&summary, student_c_id)["stages"][1]["status"],
        "passed"
    );
    assert_eq!(
        student_row(&summary, student_c_id)["stages"][2]["status"],
        "missing"
    );

    let own_summary = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&student_c),
            None,
        )
        .await;
    let own_summary: Value = json_body(own_summary).await;
    assert_eq!(own_summary["students"].as_array().unwrap().len(), 1);
    assert_eq!(
        own_summary["students"][0]["user_id"],
        student_c_id.to_string()
    );
    let outside_snapshot = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&student_d),
            None,
        )
        .await;
    assert_eq!(outside_snapshot.status(), StatusCode::OK);
    assert!(
        json_body(outside_snapshot).await["students"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let renewed = seed_attempt(
        &app,
        renew_two,
        student_c_id,
        None,
        true,
        true,
        false,
        "passed",
        Some("passed"),
    )
    .await;
    let _ = (checkin_c, renew_one_c, renewed);
    let after_all_stages = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    assert_eq!(
        student_row(&json_body(after_all_stages).await, student_c_id)["status"],
        "present"
    );
    assert_eq!(
        decide(&app, pending_review_id, &teacher, "reject")
            .await
            .status(),
        StatusCode::OK
    );
    let after_reject = app
        .send(
            Method::GET,
            &format!("/api/lessons/{lesson_id}/attendance"),
            Body::empty(),
            None,
            Some(&teacher),
            None,
        )
        .await;
    let after_reject: Value = json_body(after_reject).await;
    assert_eq!(student_row(&after_reject, student_b_id)["status"], "absent");
    assert_eq!(
        student_row(&after_reject, student_b_id)["attendance_success"],
        false
    );
    assert_eq!(
        student_row(&after_reject, student_a_id)["status"],
        "excused"
    );
    let historical_evidence = app
        .send(
            Method::GET,
            &format!("/api/reviews/{leave_id}/evidence"),
            Body::empty(),
            None,
            Some(&student_a),
            None,
        )
        .await;
    assert_eq!(historical_evidence.status(), StatusCode::OK);
}
