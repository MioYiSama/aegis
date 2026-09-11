use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct User {
    id: i32,
}

pub fn router() -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(get_user))
}

#[axum::debug_handler]
#[utoipa::path(get, path = "/", responses((status = OK, body = User)), tag = "User")]
async fn get_user() -> Json<User> {
    Json(User { id: 1 })
}
