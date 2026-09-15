use axum::{Router, middleware};
use tower_http::{catch_panic::CatchPanicLayer, cors::CorsLayer, trace::TraceLayer};
use utoipa::openapi::{InfoBuilder, OpenApi};
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

mod auth;

pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .nest("/auth", auth::router())
        .layer(middleware::from_fn(auth::middleware))
        .layer(CorsLayer::very_permissive())
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
}

pub fn openapi() -> OpenApi {
    let mut api = openapi_router().into_openapi();
    api.info = InfoBuilder::new()
        .title(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .build();
    api
}

pub fn router() -> Router<AppState> {
    openapi_router().into()
}
