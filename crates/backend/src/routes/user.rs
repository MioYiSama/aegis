use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::openapi::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct User {
    id: i32,
}

pub fn router() -> (Router, OpenApi) {
    OpenApiRouter::new()
        .routes(routes!(get_user))
        .split_for_parts()
}

#[axum::debug_handler]
#[utoipa::path(get, path = "/user", responses((status = OK, body = User)))]
async fn get_user() -> Json<User> {
    Json(User { id: 1 })
}
