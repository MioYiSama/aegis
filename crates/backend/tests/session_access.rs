use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use aegis_backend::{
    auth::{self, AuthState, CurrentUser},
    config::Config,
    error::ApiResult,
};
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{
        Method, Request, Response, StatusCode,
        header::{COOKIE, ORIGIN, SET_COOKIE},
    },
    routing::get,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tower::ServiceExt;

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
        let database_path = directory.path().join("sessions.sqlite");
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
        auth::router(self.pool.clone(), self.config.clone())
    }

    fn role_router(&self) -> Router {
        let auth_state = AuthState {
            pool: self.pool.clone(),
            config: self.config.clone(),
        };
        Router::new()
            .route("/test/student-only", get(student_only))
            .route("/test/teacher-only", get(teacher_only))
            .merge(self.router())
            .layer(Extension(auth_state))
    }
}

async fn student_only(user: CurrentUser) -> ApiResult<StatusCode> {
    user.student()?;
    Ok(StatusCode::NO_CONTENT)
}

async fn teacher_only(user: CurrentUser) -> ApiResult<StatusCode> {
    user.teacher()?;
    Ok(StatusCode::NO_CONTENT)
}

fn request(
    method: Method,
    path: &str,
    body: Option<String>,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Request<Body> {
    let has_body = body.is_some();
    let body = body.map(Body::from).unwrap_or_else(Body::empty);
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
    body: Option<String>,
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

fn register_body(username: &str, role: &str, student_no: Option<&str>) -> String {
    let mut value = json!({
        "username": username,
        "password": PASSWORD,
        "display_name": format!("Display {username}"),
        "role": role,
    });
    if let Some(student_no) = student_no {
        value["student_no"] = json!(student_no);
    }
    value.to_string()
}

async fn register(app: &TestApp, username: &str, role: &str, student_no: Option<&str>) -> Value {
    let response = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(register_body(username, role, student_no)),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await
}

async fn login(app: &TestApp, username: &str) -> (String, Value) {
    let body = json!({ "username": username, "password": PASSWORD }).to_string();
    let response = send(
        app.router(),
        Method::POST,
        "/api/auth/login",
        Some(body),
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
    assert!(
        response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("HttpOnly")
    );
    assert!(
        response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("SameSite=Strict")
    );
    let body: Value = json_body(response).await;
    assert!(body.get("password_hash").is_none());
    assert!(body.get("token").is_none());
    (cookie, body)
}

#[tokio::test]
async fn second_login_invalidates_the_previous_opaque_cookie() {
    let app = TestApp::new().await;
    register(&app, "student.one", "student", Some("s-001")).await;

    let (first_cookie, first_user) = login(&app, "STUDENT.ONE").await;
    let (second_cookie, second_user) = login(&app, "student.one").await;
    assert_eq!(first_user["id"], second_user["id"]);
    assert_ne!(first_cookie, second_cookie);

    let old_response = send(
        app.router(),
        Method::GET,
        "/api/auth/me",
        None,
        Some(&first_cookie),
        None,
    )
    .await;
    assert_eq!(old_response.status(), StatusCode::UNAUTHORIZED);
    let current_response = send(
        app.router(),
        Method::GET,
        "/api/auth/me",
        None,
        Some(&second_cookie),
        None,
    )
    .await;
    assert_eq!(current_response.status(), StatusCode::OK);
    let current_user: Value = json_body(current_response).await;
    assert_eq!(current_user["id"], second_user["id"]);
    assert_eq!(current_user["username"], "student.one");
}

#[tokio::test]
async fn logout_clears_the_cookie_and_expired_sessions_are_rejected() {
    let app = TestApp::new().await;
    let anonymous_logout = send(
        app.router(),
        Method::POST,
        "/api/auth/logout",
        None,
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(anonymous_logout.status(), StatusCode::UNAUTHORIZED);

    let user = register(&app, "logout.user", "student", Some("s-logout")).await;
    let (cookie, _) = login(&app, "logout.user").await;

    let logout_response = send(
        app.router(),
        Method::POST,
        "/api/auth/logout",
        None,
        Some(&cookie),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);
    assert!(
        logout_response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let after_logout = send(
        app.router(),
        Method::GET,
        "/api/auth/me",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(after_logout.status(), StatusCode::UNAUTHORIZED);

    let (expired_cookie, _) = login(&app, "logout.user").await;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE sessions SET created_at = ?, expires_at = ? WHERE user_id = ?")
        .bind(now - 20_000)
        .bind(now - 10_000)
        .bind(user["id"].as_str().unwrap())
        .execute(&app.pool)
        .await
        .unwrap();
    let expired = send(
        app.router(),
        Method::GET,
        "/api/auth/me",
        None,
        Some(&expired_cookie),
        None,
    )
    .await;
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
    let expired_logout = send(
        app.router(),
        Method::POST,
        "/api/auth/logout",
        None,
        Some(&expired_cookie),
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(expired_logout.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn origin_schema_uniqueness_and_role_guards_are_enforced() {
    let app = TestApp::new().await;

    let no_origin = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(register_body("no.origin", "teacher", None)),
        None,
        None,
    )
    .await;
    assert_eq!(no_origin.status(), StatusCode::FORBIDDEN);
    let wrong_origin = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(register_body("wrong.origin", "teacher", None)),
        None,
        Some("https://attacker.example"),
    )
    .await;
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
    let unknown_field = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(
            json!({
                "username": "unknown.field",
                "password": PASSWORD,
                "display_name": "Unknown Field",
                "role": "teacher",
                "extra": true,
            })
            .to_string(),
        ),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(unknown_field.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_password = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(
            json!({
                "username": "short.password",
                "password": "short",
                "display_name": "Short Password",
                "role": "teacher",
            })
            .to_string(),
        ),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(invalid_password.status(), StatusCode::UNPROCESSABLE_ENTITY);

    register(&app, "  DUP.NAME ", "teacher", None).await;
    let wrong_password = send(
        app.router(),
        Method::POST,
        "/api/auth/login",
        Some(json!({ "username": "dup.name", "password": "wrong-password-value" }).to_string()),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    let unknown_user = send(
        app.router(),
        Method::POST,
        "/api/auth/login",
        Some(json!({ "username": "missing.user", "password": PASSWORD }).to_string()),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_user.status(), StatusCode::UNAUTHORIZED);
    let wrong_password_body: Value = json_body(wrong_password).await;
    let unknown_user_body: Value = json_body(unknown_user).await;
    assert_eq!(wrong_password_body, unknown_user_body);

    let duplicate_username = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(register_body("dup.name", "teacher", None)),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(duplicate_username.status(), StatusCode::CONFLICT);
    register(&app, "student.first", "student", Some("same-student-no")).await;
    let duplicate_student_no = send(
        app.router(),
        Method::POST,
        "/api/auth/register",
        Some(register_body(
            "student.second",
            "student",
            Some("same-student-no"),
        )),
        None,
        Some(ORIGIN_VALUE),
    )
    .await;
    assert_eq!(duplicate_student_no.status(), StatusCode::CONFLICT);

    register(&app, "teacher.role", "teacher", None).await;
    register(&app, "student.role", "student", Some("s-role")).await;
    let (teacher_cookie, _) = login(&app, "teacher.role").await;
    let (student_cookie, _) = login(&app, "student.role").await;
    let router = app.role_router();

    assert_eq!(
        send(
            router.clone(),
            Method::GET,
            "/test/teacher-only",
            None,
            Some(&teacher_cookie),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            router.clone(),
            Method::GET,
            "/test/student-only",
            None,
            Some(&student_cookie),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            router.clone(),
            Method::GET,
            "/test/student-only",
            None,
            Some(&teacher_cookie),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            router,
            Method::GET,
            "/test/teacher-only",
            None,
            Some(&student_cookie),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}
