use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

mod error;
mod user;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().nest("/user", user::router())
}
