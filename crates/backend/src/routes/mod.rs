use utoipa_axum::router::OpenApiRouter;

mod user;

pub fn router() -> OpenApiRouter {
    OpenApiRouter::new().nest("/user", user::router())
}
