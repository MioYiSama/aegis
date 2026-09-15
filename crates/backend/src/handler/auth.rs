use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use enum_assoc::Assoc;
use jiff::Timestamp;
use serde::Deserialize;
use thiserror::Error;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use validator::{Validate, ValidationErrors};

use crate::{
    model::{Session, User, UserRole},
    service::{auth::*, validate::non_empty_after_trimmed},
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(sign_in))
        .routes(routes!(sign_up))
        .routes(routes!(sign_out))
        .routes(routes!(refresh))
}

#[derive(Deserialize, ToSchema)]
struct SignInRequest {
    identity: String,
    password: String,
}
#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-in", tag = "Auth",
    request_body = SignInRequest,
    responses(
        (status = 200, headers(("Set-Cookie" = String))),
        (status = 400, body = String),
        (status = 500, body = String),
    )
)]
async fn sign_in(
    jar: CookieJar,
    State(mut state): State<AppState>,
    Json(request): Json<SignInRequest>,
) -> AuthResult<CookieJar> {
    let Some(user) = User::filter_by_identity(request.identity)
        .first()
        .exec(&mut state.db)
        .await?
    else {
        return Err(AuthError::IncorrectAccountOrPassword);
    };

    if !verify_password(&user.password, &request.password) {
        return Err(AuthError::IncorrectAccountOrPassword);
    }

    create_session(jar, &mut state, &user).await
}

#[derive(Deserialize, ToSchema, Validate)]
struct SignUpRequest {
    #[validate(custom(function = non_empty_after_trimmed))]
    identity: String,
    #[validate(length(min = 6))]
    password: String,
    #[validate(custom(function = non_empty_after_trimmed))]
    name: Option<String>,
    #[validate(custom(function = validate_role))]
    role: UserRole,
}
#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-up", tag = "Auth",
    request_body = SignUpRequest,
    responses(
        (status = 200, headers(("Set-Cookie" = String))),
        (status = 400, body = String),
        (status = 500, body = String),
    )
)]
async fn sign_up(
    jar: CookieJar,
    State(mut state): State<AppState>,
    Json(mut request): Json<SignUpRequest>,
) -> AuthResult<CookieJar> {
    request.identity = request.identity.trim().to_owned();
    request.name = request.name.map(|x| x.trim().to_owned());
    request.validate()?;

    if User::filter_by_identity(&request.identity)
        .first()
        .exec(&mut state.db)
        .await?
        .is_some()
    {
        return Err(AuthError::AccountExist);
    }

    let user = User::create()
        .identity(request.identity)
        .password(hash_password(&request.password)?)
        .name(request.name)
        .role(request.role)
        .exec(&mut state.db)
        .await?;

    create_session(jar, &mut state, &user).await
}

#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-out", tag = "Auth",
    params(
        ("access_token" = String, Cookie),
        ("refresh_token" = String, Cookie),
    ),
    responses(
        (status = 200, headers(("Set-Cookie" = String))),
        (status = 500, body = String),
    )
)]
async fn sign_out(jar: CookieJar, State(mut state): State<AppState>) -> AuthResult<CookieJar> {
    if let Some(cookie) = jar.get(REFRESH_TOKEN_COOKIE) {
        Session::delete_by_token(&mut state.db, digest_token(cookie.value())).await?;
    }

    Ok(remove_tokens_from_cookies(jar))
}

#[axum::debug_handler]
#[utoipa::path(
    post, path = "/refresh", tag = "Auth",
    params(
        ("access_token" = String, Cookie),
        ("refresh_token" = String, Cookie),
    ),
    responses(
        (status = 200, headers(("Set-Cookie" = String))),
        (status = 400, body = String),
        (status = 401, body = String),
        (status = 500, body = String),
    )
)]
async fn refresh(jar: CookieJar, State(mut state): State<AppState>) -> AuthResult<CookieJar> {
    let Some(cookie) = jar.get(REFRESH_TOKEN_COOKIE) else {
        return Err(AuthError::NoRefreshToken);
    };

    let Some(session) = Session::filter_by_token(digest_token(cookie.value()))
        .first()
        .exec(&mut state.db)
        .await?
    else {
        return Err(AuthError::InvalidRefreshToken);
    };

    if Timestamp::now() >= session.expires_at {
        return Err(AuthError::RefreshTokenExpired);
    }

    let user = session.user().exec(&mut state.db).await?;

    create_session(jar, &mut state, &user).await
}

#[derive(Clone, Default)]
pub enum AuthContext {
    #[default]
    Anonymous,
    Authenticated {
        identity: String,
        role: UserRole,
    },
}

impl From<Claims> for AuthContext {
    fn from(claims: Claims) -> Self {
        Self::Authenticated {
            identity: claims.sub,
            role: claims.role,
        }
    }
}

#[axum::debug_middleware]
pub async fn middleware(jar: CookieJar, mut request: Request, next: Next) -> Response {
    let ctx = jar
        .get(ACCESS_TOKEN_COOKIE)
        .and_then(|cookie| parse_jwt(cookie.value()).ok())
        .map(AuthContext::from)
        .unwrap_or_default();

    request.extensions_mut().insert(ctx);
    next.run(request).await
}

type AuthResult<T = ()> = Result<T, AuthError>;

#[derive(Debug, Error, Assoc)]
#[func(fn status(&self) -> StatusCode)]
enum AuthError {
    #[error("输入格式不正确：{0}")]
    #[assoc(status = StatusCode::BAD_REQUEST)]
    Validation(#[from] ValidationErrors),

    #[error("账号或密码错误")]
    #[assoc(status = StatusCode::BAD_REQUEST)]
    IncorrectAccountOrPassword,

    #[error("此学号/工号已被注册")]
    #[assoc(status = StatusCode::BAD_REQUEST)]
    AccountExist,

    #[error("缺少刷新令牌")]
    #[assoc(status = StatusCode::BAD_REQUEST)]
    NoRefreshToken,

    #[error("刷新令牌无效")]
    #[assoc(status = StatusCode::UNAUTHORIZED)]
    InvalidRefreshToken,

    #[error("刷新令牌过期")]
    #[assoc(status = StatusCode::UNAUTHORIZED)]
    RefreshTokenExpired,

    #[error("jiff错误")]
    #[assoc(status = StatusCode::INTERNAL_SERVER_ERROR)]
    Jiff(#[from] jiff::Error),

    #[error("数据库错误")]
    #[assoc(status = StatusCode::INTERNAL_SERVER_ERROR)]
    Database(#[from] toasty::Error),

    #[error("未知错误")]
    #[assoc(status = StatusCode::INTERNAL_SERVER_ERROR)]
    Unknown(#[from] color_eyre::eyre::Report),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

const ACCESS_TOKEN_COOKIE: &str = "access_token";
const REFRESH_TOKEN_COOKIE: &str = "refresh_token";
const ACCESS_TOKEN_TTL_SECONDS: i64 = 300; // 5 minutes
const REFRESH_TOKEN_TTL_SECONDS: i64 = 7 * 24 * 3600; // 1 week
const ACCESS_TOKEN_PATH: &str = "/";
const REFRESH_TOKEN_PATH: &str = "/auth";
const COOKIE_SECURE: bool = crate::RELEASE;

fn add_tokens_to_cookies(jar: CookieJar, access_token: String, refresh_token: String) -> CookieJar {
    jar.add(
        Cookie::build((ACCESS_TOKEN_COOKIE, access_token))
            .max_age(time::Duration::seconds(ACCESS_TOKEN_TTL_SECONDS))
            .path(ACCESS_TOKEN_PATH)
            .http_only(true)
            .secure(COOKIE_SECURE)
            .same_site(SameSite::Lax),
    )
    .add(
        Cookie::build((REFRESH_TOKEN_COOKIE, refresh_token))
            .max_age(time::Duration::seconds(REFRESH_TOKEN_TTL_SECONDS))
            .path(REFRESH_TOKEN_PATH)
            .http_only(true)
            .secure(COOKIE_SECURE)
            .same_site(SameSite::Lax),
    )
}

fn remove_tokens_from_cookies(jar: CookieJar) -> CookieJar {
    jar.remove(
        Cookie::build(ACCESS_TOKEN_COOKIE)
            .path(ACCESS_TOKEN_PATH)
            .secure(COOKIE_SECURE)
            .removal(),
    )
    .remove(
        Cookie::build(REFRESH_TOKEN_COOKIE)
            .path(REFRESH_TOKEN_PATH)
            .secure(COOKIE_SECURE)
            .removal(),
    )
}

async fn create_session(
    jar: CookieJar,
    state: &mut AppState,
    user: &User,
) -> AuthResult<CookieJar> {
    let now = Timestamp::now().as_second();

    let access_token = generate_jwt(&Claims {
        sub: user.identity.clone(),
        exp: now + ACCESS_TOKEN_TTL_SECONDS,
        role: user.role.clone(),
    })?;

    let refresh_token = generate_token();

    Session::upsert_by_user_id(user.id)
        .token(digest_token(&refresh_token))
        .expires_at(Timestamp::from_second(now + REFRESH_TOKEN_TTL_SECONDS)?)
        .exec(&mut state.db)
        .await?;

    Ok(add_tokens_to_cookies(jar, access_token, refresh_token))
}
