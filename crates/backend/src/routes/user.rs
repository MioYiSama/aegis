use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    db::user::{User, UserRole},
    routes::error::{AppError, AppResult},
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(sign_up))
        .routes(routes!(sign_in))
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
struct SignUpRequest {
    identity: String,
    password: String,
    name: Option<String>,
    role: UserRole,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
struct SignUpResponse {
    token: String,
}

#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-up", tag = "User",
    request_body = SignUpRequest,
    responses(
        (status = 200, body = SignUpResponse),
        (status = 400, body = String),
    )
)]
async fn sign_up(
    State(state): State<AppState>,
    Json(request): Json<SignUpRequest>,
) -> AppResult<Json<SignUpResponse>> {
    if matches!(request.role, UserRole::Admin) {
        return AppError::BadRequest("禁止注册管理员账号".to_owned()).into();
    }

    let password = Argon2::default()
        .hash_password(request.password.as_bytes())?
        .to_string();

    toasty::create!(User {
        identity: request.identity,
        password,
        name: request.name,
        role: request.role,
    })
    .exec(&mut state.db())
    .await?;

    Ok(Json(SignUpResponse {
        token: "TODO".to_owned(),
    }))
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
struct SignInRequest {
    identity: String,
    password: String,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
struct SignInResponse {
    token: String,
}

#[axum::debug_handler]
#[utoipa::path(
    post, path = "/sign-in", tag = "User",
    request_body = SignInRequest,
    responses(
        (status = 200, body = SignInResponse),
        (status = 401, body = String),
    )
)]
async fn sign_in(
    State(state): State<AppState>,
    Json(request): Json<SignInRequest>,
) -> AppResult<Json<SignInResponse>> {
    let user = User::get_by_identity(&mut state.db(), request.identity).await?;

    if Argon2::default()
        .verify_password(request.password.as_bytes(), user.password.as_str())
        .is_err()
    {
        return AppError::Unauthorized("密码错误".to_owned()).into();
    }

    Ok(Json(SignInResponse {
        token: "TODO".to_owned(),
    }))
}
