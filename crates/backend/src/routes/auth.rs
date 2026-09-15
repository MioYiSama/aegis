use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{
    Json,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    db::{
        auth::Session,
        user::{User, UserRole},
    },
    routes::error::{AppError, AppResult},
    state::AppState,
    token::{Claims, digest_token, generate_jwt, generate_token, parse_jwt},
};

const ACCESS_TOKEN_COOKIE: &str = "access_token";
const REFRESH_TOKEN_COOKIE: &str = "refresh_token";
const ACCESS_TOKEN_TTL_SECONDS: i64 = 300; // 5 minutes
const REFRESH_TOKEN_TTL_SECONDS: i64 = 7 * 24 * 3600; // 1 week
const ACCESS_TOKEN_PATH: &str = "/";
const REFRESH_TOKEN_PATH: &str = "/auth";
const COOKIE_SECURE: bool = cfg!(not(debug_assertions));

fn hash_password(password: &str) -> AppResult<String> {
    let hash = Argon2::default().hash_password(password.as_bytes())?;
    Ok(hash.to_string())
}

fn verify_password(expect: &str, actual: &str) -> bool {
    let Ok(hash) = PasswordHash::new(expect) else {
        return false;
    };

    Argon2::default()
        .verify_password(actual.as_bytes(), &hash)
        .is_ok()
}

fn add_auth_cookies(jar: CookieJar, access_token: String, refresh_token: String) -> CookieJar {
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

fn remove_auth_cookies(jar: CookieJar) -> CookieJar {
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

async fn create_session(jar: CookieJar, state: &mut AppState, user: &User) -> AppResult<CookieJar> {
    let access_token = generate_jwt(user)?;
    let refresh_token = generate_token();

    Session::upsert_by_user_id(user.id)
        .token(digest_token(&refresh_token))
        .expires_at(
            jiff::Timestamp::now() + jiff::SignedDuration::from_secs(REFRESH_TOKEN_TTL_SECONDS),
        )
        .exec(&mut state.db)
        .await?;

    Ok(add_auth_cookies(jar, access_token, refresh_token))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(sign_up))
        .routes(routes!(sign_in))
        .routes(routes!(sign_out))
        .routes(routes!(refresh))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct SignUpRequest {
    identity: String,
    password: String,
    name: Option<String>,
    role: UserRole,
}
#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-up", tag = "Auth",
    request_body = SignUpRequest,
    responses(
        (status = 200, body = (), headers(("Set-Cookie" = String))),
        (status = 400, body = String),
        (status = 500, body = String),
    )
)]
async fn sign_up(
    jar: CookieJar,
    State(mut state): State<AppState>,
    Json(request): Json<SignUpRequest>,
) -> AppResult<CookieJar> {
    if request.identity.trim().is_empty() {
        return AppError::BadRequest("学号/工号不能为空".to_owned()).into();
    }
    if request.password.len() < 6 {
        return AppError::BadRequest("密码长度必须至少为6个字符".to_owned()).into();
    }

    if matches!(request.role, UserRole::Admin) {
        return AppError::BadRequest("禁止注册管理员身份的账号".to_owned()).into();
    }

    if User::get_by_identity(&mut state.db, &request.identity)
        .await
        .is_ok()
    {
        return AppError::BadRequest("此学号/工号已被注册".to_owned()).into();
    }

    let password_hash = hash_password(&request.password)?;
    let user = User::create()
        .identity(request.identity)
        .password(password_hash)
        .name(request.name)
        .role(request.role)
        .exec(&mut state.db)
        .await?;

    create_session(jar, &mut state, &user).await
}

#[derive(Deserialize, utoipa::ToSchema)]
struct SignInRequest {
    identity: String,
    password: String,
}
#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-in", tag = "Auth",
    request_body = SignInRequest,
    responses(
        (status = 200, body = (), headers(("Set-Cookie" = String))),
        (status = 401, body = String),
        (status = 500, body = String),
    )
)]
async fn sign_in(
    jar: CookieJar,
    State(mut state): State<AppState>,
    Json(request): Json<SignInRequest>,
) -> AppResult<CookieJar> {
    let Ok(user) = User::get_by_identity(&mut state.db, request.identity).await else {
        return AppError::Unauthorized("账号或密码错误".to_owned()).into();
    };

    if !verify_password(&user.password, &request.password) {
        return AppError::Unauthorized("账号或密码错误".to_owned()).into();
    }

    create_session(jar, &mut state, &user).await
}

#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-out", tag = "Auth",
    responses(
        (status = 200, body = (), headers(("Set-Cookie" = String))),
        (status = 500, body = String),
    )
)]
pub async fn sign_out(jar: CookieJar, State(mut state): State<AppState>) -> AppResult<CookieJar> {
    if let Some(cookie) = jar.get(REFRESH_TOKEN_COOKIE) {
        let refresh_token = cookie.value();

        Session::delete_by_token(&mut state.db, digest_token(refresh_token)).await?;
    }

    Ok(remove_auth_cookies(jar))
}

#[axum::debug_handler]
#[utoipa::path(
    post, path = "/refresh", tag = "Auth",
    responses(
        (status = 200, body = (), headers(("Set-Cookie" = String))),
        (status = 401, body = String),
        (status = 500, body = String),
    )
)]
pub async fn refresh(jar: CookieJar, State(mut state): State<AppState>) -> AppResult<CookieJar> {
    let Some(refresh_token) = jar.get(REFRESH_TOKEN_COOKIE) else {
        return AppError::Unauthorized("缺少刷新令牌".to_owned()).into();
    };

    let digest = digest_token(refresh_token.value());
    let Ok(session) = Session::get_by_token(&mut state.db, digest).await else {
        return AppError::Unauthorized("刷新令牌无效".to_owned()).into();
    };

    if session.expires_at < jiff::Timestamp::now() {
        return AppError::Unauthorized("刷新令牌已过期".to_owned()).into();
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

impl AuthContext {
    pub fn authorize(&self, target_role: UserRole) -> AppResult<()> {
        match self {
            AuthContext::Anonymous => AppError::Unauthorized("未登录".to_owned()).into(),
            AuthContext::Authenticated {
                role: UserRole::Admin,
                ..
            } => Ok(()),
            AuthContext::Authenticated { role, .. } if *role == target_role => Ok(()),
            _ => AppError::PermissionDenied("权限不足".to_owned()).into(),
        }
    }
}

#[axum::debug_middleware]
pub async fn middleware(jar: CookieJar, mut req: Request, next: Next) -> Response {
    let ctx = jar
        .get(ACCESS_TOKEN_COOKIE)
        .and_then(|cookie| parse_jwt(cookie.value()).ok())
        .map(AuthContext::from)
        .unwrap_or_default();

    req.extensions_mut().insert(ctx);

    next.run(req).await
}
