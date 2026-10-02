use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Extension, Json as ResponseJson, Router,
    extract::{FromRequestParts, Request, State},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{COOKIE, ORIGIN, SET_COOKIE},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    config::Config,
    error::{ApiError, ApiResult, Json},
};

const SESSION_COOKIE: &str = "aegis_session";
const SESSION_MAX_AGE_SECONDS: i64 = 7 * 24 * 60 * 60;
const SESSION_LIFETIME_MS: i64 = SESSION_MAX_AGE_SECONDS * 1000;

#[derive(Clone)]
pub struct AuthState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Student,
    Teacher,
}

impl Role {
    fn as_db_value(self) -> &'static str {
        match self {
            Self::Student => "student",
            Self::Teacher => "teacher",
        }
    }

    fn from_db_value(value: &str) -> ApiResult<Self> {
        match value {
            "student" => Ok(Self::Student),
            "teacher" => Ok(Self::Teacher),
            _ => Err(ApiError::internal()),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub role: Role,
    pub student_no: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct CurrentUser {
    pub user: User,
    token_hash: Vec<u8>,
}

impl CurrentUser {
    pub fn student(&self) -> ApiResult<()> {
        if self.user.role == Role::Student {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }

    pub fn teacher(&self) -> ApiResult<()> {
        if self.user.role == Role::Teacher {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let state = parts
            .extensions
            .get::<AuthState>()
            .ok_or_else(ApiError::internal)?;
        let token = session_token(&parts.headers).ok_or_else(ApiError::unauthenticated)?;
        let token_hash = Sha256::digest(token.as_bytes()).to_vec();
        let row = sqlx::query_as::<_, SessionUserRow>(
            "SELECT u.id, u.username, u.display_name, u.role, u.student_no, u.created_at, s.expires_at \
             FROM sessions AS s JOIN users AS u ON u.id = s.user_id WHERE s.token_hash = ?",
        )
        .bind(&token_hash[..])
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::unauthenticated)?;

        if row.expires_at <= Utc::now().timestamp_millis() {
            return Err(ApiError::unauthenticated());
        }

        Ok(Self {
            user: row.into_user()?,
            token_hash,
        })
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterRequest {
    /// Trimmed and ASCII-lowercased before validation; the normalized username must be 3–64 ASCII letters, digits, dots, underscores, or hyphens.
    #[schema(min_length = 3, max_length = 64, pattern = "^[A-Za-z0-9_.-]{3,64}$")]
    username: String,
    /// Password containing 12–128 characters.
    #[schema(min_length = 12, max_length = 128)]
    password: String,
    /// Display name is trimmed and must contain 1–128 characters.
    #[schema(min_length = 1, max_length = 128)]
    display_name: String,
    /// Self-selected account role; a student requires student_no and a teacher must omit it.
    role: Role,
    /// Trimmed student number, required for students and omitted or null for teachers; maximum 64 characters.
    #[serde(default)]
    #[schema(min_length = 1, max_length = 64)]
    student_no: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRequest {
    /// Username is trimmed and ASCII-lowercased for lookup; registered usernames are 3–64 ASCII letters, digits, dots, underscores, or hyphens.
    #[schema(min_length = 3, max_length = 64, pattern = "^[A-Za-z0-9_.-]{3,64}$")]
    username: String,
    /// Registered passwords contain 12–128 characters.
    #[schema(min_length = 12, max_length = 128)]
    password: String,
}

#[derive(FromRow)]
struct UserRow {
    id: String,
    username: String,
    display_name: String,
    role: String,
    student_no: Option<String>,
    created_at: i64,
}

impl UserRow {
    fn into_user(self) -> ApiResult<User> {
        let id = Uuid::parse_str(&self.id).map_err(|_| ApiError::internal())?;
        let created_at = DateTime::<Utc>::from_timestamp_millis(self.created_at)
            .ok_or_else(ApiError::internal)?;
        Ok(User {
            id,
            username: self.username,
            display_name: self.display_name,
            role: Role::from_db_value(&self.role)?,
            student_no: self.student_no,
            created_at,
        })
    }
}

#[derive(FromRow)]
struct LoginRow {
    id: String,
    username: String,
    display_name: String,
    role: String,
    student_no: Option<String>,
    created_at: i64,
    password_hash: String,
}

impl LoginRow {
    fn into_user_row(self) -> (UserRow, String) {
        (
            UserRow {
                id: self.id,
                username: self.username,
                display_name: self.display_name,
                role: self.role,
                student_no: self.student_no,
                created_at: self.created_at,
            },
            self.password_hash,
        )
    }
}

#[derive(FromRow)]
struct SessionUserRow {
    id: String,
    username: String,
    display_name: String,
    role: String,
    student_no: Option<String>,
    created_at: i64,
    expires_at: i64,
}

impl SessionUserRow {
    fn into_user(self) -> ApiResult<User> {
        UserRow {
            id: self.id,
            username: self.username,
            display_name: self.display_name,
            role: self.role,
            student_no: self.student_no,
            created_at: self.created_at,
        }
        .into_user()
    }
}

pub fn router(pool: SqlitePool, config: Arc<Config>) -> Router {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .layer(Extension(AuthState {
            pool,
            config: config.clone(),
        }))
        .layer(middleware::from_fn_with_state(config, origin_guard))
}

pub async fn origin_guard(
    State(config): State<Arc<Config>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method();
    let needs_origin = method != Method::GET && method != Method::HEAD && method != Method::OPTIONS;
    let origin_matches = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| origin == config.origin.as_str());

    if needs_origin && !origin_matches {
        return ApiError::forbidden().into_response();
    }
    next.run(request).await
}

#[utoipa::path(
    post,
    path = "/api/auth/register",
    operation_id = "auth_register",
    params(("Origin" = String, Header, description = "Must equal the configured application origin")),
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Account created; registration does not create a session", body = User),
        (status = 403, description = "Origin rejected", body = ApiError),
        (status = 409, description = "Username or student number already exists", body = ApiError),
        (status = 413, description = "Request body exceeds the JSON limit", body = ApiError),
        (status = 422, description = "Invalid registration fields", body = ApiError),
        (status = 500, description = "Internal service error", body = ApiError)
    ),
    tag = "auth"
)]
pub(crate) async fn register(
    Extension(state): Extension<AuthState>,
    Json(input): Json<RegisterRequest>,
) -> ApiResult<Response> {
    let username = normalize_registration_username(&input.username)?;
    let display_name = normalize_display_name(input.display_name)?;
    let student_no = normalize_student_no(input.student_no)?;
    validate_role_student_no(input.role, student_no.as_deref())?;
    validate_password(&input.password)?;

    let password = input.password;
    let password_hash = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| ApiError::internal())
    })
    .await
    .map_err(|_| ApiError::internal())??;

    let id = Uuid::new_v4();
    let created_at_ms = Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, role, student_no, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&username)
    .bind(password_hash)
    .bind(&display_name)
    .bind(input.role.as_db_value())
    .bind(&student_no)
    .bind(created_at_ms)
    .execute(&state.pool)
    .await
    .map_err(ApiError::from)?;

    let created_at =
        DateTime::<Utc>::from_timestamp_millis(created_at_ms).ok_or_else(ApiError::internal)?;
    Ok((
        StatusCode::CREATED,
        ResponseJson(User {
            id,
            username,
            display_name,
            role: input.role,
            student_no,
            created_at,
        }),
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/auth/login",
    operation_id = "auth_login",
    params(("Origin" = String, Header, description = "Must equal the configured application origin")),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authenticated user; opaque session token is set only in the cookie", body = User, headers(("Set-Cookie" = String, description = "HttpOnly; SameSite=Strict; Path=/; Max-Age=604800; Secure when configured"))),
        (status = 401, description = "Invalid credentials", body = ApiError),
        (status = 403, description = "Origin rejected", body = ApiError),
        (status = 413, description = "Request body exceeds the JSON limit", body = ApiError),
        (status = 422, description = "Invalid login request", body = ApiError),
        (status = 500, description = "Internal service error", body = ApiError)
    ),
    tag = "auth"
)]
pub(crate) async fn login(
    Extension(state): Extension<AuthState>,
    Json(input): Json<LoginRequest>,
) -> ApiResult<Response> {
    let username = input.username.trim().to_ascii_lowercase();
    let row = sqlx::query_as::<_, LoginRow>(
        "SELECT id, username, display_name, role, student_no, created_at, password_hash \
         FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::from)?
    .ok_or_else(ApiError::unauthenticated)?;
    let (user_row, password_hash) = row.into_user_row();
    let password = input.password;
    let password_matches = tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&password_hash).ok()?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .ok()?;
        Some(())
    })
    .await
    .map_err(|_| ApiError::internal())?
    .is_some();
    if !password_matches {
        return Err(ApiError::unauthenticated());
    }

    let user = user_row.into_user()?;
    let mut token_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);
    let token_hash = Sha256::digest(token.as_bytes()).to_vec();
    let now = Utc::now().timestamp_millis();
    let expires_at = now
        .checked_add(SESSION_LIFETIME_MS)
        .ok_or_else(ApiError::internal)?;

    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::from)?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user.id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::from)?;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(token_hash)
    .bind(user.id.to_string())
    .bind(now)
    .bind(expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::from)?;
    transaction.commit().await.map_err(ApiError::from)?;

    let cookie = session_cookie(&token, state.config.cookie_secure)?;
    let mut response = (StatusCode::OK, ResponseJson(user)).into_response();
    response.headers_mut().insert(SET_COOKIE, cookie);
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/auth/logout",
    operation_id = "auth_logout",
    params(("Origin" = String, Header, description = "Must equal the configured application origin")),
    responses(
        (status = 204, description = "Session invalidated and cookie cleared", headers(("Set-Cookie" = String, description = "Max-Age=0; HttpOnly; SameSite=Strict; Path=/; Secure when configured"))),
        (status = 401, description = "Authentication required", body = ApiError),
        (status = 403, description = "Origin rejected", body = ApiError),
        (status = 500, description = "Internal service error", body = ApiError)
    ),
    security(("session" = [])),
    tag = "auth"
)]
pub(crate) async fn logout(
    Extension(state): Extension<AuthState>,
    user: CurrentUser,
) -> ApiResult<Response> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(user.token_hash)
        .execute(&state.pool)
        .await
        .map_err(ApiError::from)?;

    let cookie = cleared_session_cookie(state.config.cookie_secure)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(SET_COOKIE, cookie);
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/auth/me",
    operation_id = "auth_me",
    responses(
        (status = 200, description = "Current authenticated user", body = User),
        (status = 401, description = "Authentication required", body = ApiError)
    ),
    security(("session" = [])),
    tag = "auth"
)]
pub(crate) async fn me(user: CurrentUser) -> ResponseJson<User> {
    ResponseJson(user.user)
}

fn normalize_registration_username(value: &str) -> ApiResult<String> {
    let username = value.trim().to_ascii_lowercase();
    if !(3..=64).contains(&username.len())
        || !username.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
        })
    {
        return Err(ApiError::invalid("Invalid username"));
    }
    Ok(username)
}

fn normalize_display_name(value: String) -> ApiResult<String> {
    let value = value.trim().to_owned();
    let length = value.chars().count();
    if length == 0 || length > 128 {
        return Err(ApiError::invalid("Invalid display name"));
    }
    Ok(value)
}

fn normalize_student_no(value: Option<String>) -> ApiResult<Option<String>> {
    value
        .map(|student_no| {
            let student_no = student_no.trim().to_owned();
            if student_no.is_empty() || student_no.chars().count() > 64 {
                Err(ApiError::invalid("Invalid student number"))
            } else {
                Ok(student_no)
            }
        })
        .transpose()
}

fn validate_role_student_no(role: Role, student_no: Option<&str>) -> ApiResult<()> {
    let valid = match role {
        Role::Student => student_no.is_some(),
        Role::Teacher => student_no.is_none(),
    };
    if valid {
        Ok(())
    } else {
        Err(ApiError::invalid("Student number does not match role"))
    }
}

fn validate_password(password: &str) -> ApiResult<()> {
    let length = password.chars().count();
    if !(12..=128).contains(&length) {
        return Err(ApiError::invalid("Invalid password length"));
    }
    Ok(())
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    let mut token = None;
    for value in headers.get_all(COOKIE) {
        for cookie in value.to_str().ok()?.split(';') {
            let Some((name, value)) = cookie.trim().split_once('=') else {
                continue;
            };
            if name.trim() == SESSION_COOKIE {
                if token.is_some() {
                    return None;
                }
                token = Some(value.trim());
            }
        }
    }
    let token = token?;
    if token.len() != 43
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    Some(token)
}

fn session_cookie(token: &str, secure: bool) -> ApiResult<HeaderValue> {
    let secure_attribute = if secure { "; Secure" } else { "" };
    let value = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_MAX_AGE_SECONDS}{secure_attribute}"
    );
    HeaderValue::from_bytes(value.as_bytes()).map_err(|_| ApiError::internal())
}

fn cleared_session_cookie(secure: bool) -> ApiResult<HeaderValue> {
    let secure_attribute = if secure { "; Secure" } else { "" };
    let value = format!(
        "{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{secure_attribute}"
    );
    HeaderValue::from_bytes(value.as_bytes()).map_err(|_| ApiError::internal())
}
