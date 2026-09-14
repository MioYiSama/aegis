use axum::{Router, http::StatusCode, middleware, response::IntoResponse};
use tower_http::{catch_panic::CatchPanicLayer, cors::CorsLayer};
use utoipa::openapi::{InfoBuilder, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

use crate::state::AppState;

mod auth;
mod error;

fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().nest("/auth", auth::router())
}

pub fn router() -> Router<AppState> {
    let (router, api) = openapi_router().split_for_parts();

    router
        .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", api))
        .layer(middleware::from_fn(auth::middleware))
        .layer(CorsLayer::permissive())
        .layer(CatchPanicLayer::custom(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")).into_response()
        }))
}

pub fn openapi() -> OpenApi {
    let mut api = openapi_router().into_openapi();

    api.info = InfoBuilder::new()
        .title(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .build();

    api
}
