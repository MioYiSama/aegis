pub mod access;
pub mod api;
pub mod attendance;
pub mod auth;
pub mod config;
pub mod courses;
pub mod db;
pub mod error;
pub mod media;
pub mod models;
pub mod policy;
pub mod reviews;
pub mod stages;
pub mod summary;
pub mod workers;

pub use config::Config;

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("Invalid backend configuration")]
    Configuration,
    #[error("Database initialization failed")]
    Database(#[from] sqlx::Error),
    #[error("Local authentication models unavailable or invalid")]
    Models,
    #[error("HTTP listener unavailable")]
    Listener(#[from] std::io::Error),
    #[error("OpenAPI document generation failed")]
    Api(#[from] api::ApiSchemaError),
}

pub async fn build_app(config: Config) -> Result<axum::Router, StartupError> {
    config.validate()?;
    let pool = db::connect(&config).await?;
    let workers = workers::FaceWorkers::load(config.model_dir.clone()).await?;
    let config = std::sync::Arc::new(config);
    let api = api::router()?;
    Ok(axum::Router::new()
        .merge(api)
        .merge(auth::router(pool.clone(), config.clone()))
        .merge(courses::router(pool.clone(), config.clone()))
        .merge(attendance::router(
            pool.clone(),
            config.clone(),
            workers.clone(),
        ))
        .merge(reviews::router(pool.clone(), config))
        .route("/healthz", axum::routing::get(health))
        .route("/readyz", axum::routing::get(ready))
        .layer(axum::Extension(pool))
        .layer(axum::Extension(workers)))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct Health {
    pub status: &'static str,
}

#[utoipa::path(
    get,
    path = "/healthz",
    operation_id = "system_healthz",
    responses((status = 200, description = "Process health", body = Health)),
    tag = "system"
)]
pub(crate) async fn health() -> axum::Json<Health> {
    axum::Json(Health { status: "ok" })
}

#[utoipa::path(
    get,
    path = "/readyz",
    operation_id = "system_readyz",
    responses(
        (status = 200, description = "Database and local inference services are ready", body = Health),
        (status = 503, description = "A required service is unavailable", body = error::ApiError)
    ),
    tag = "system"
)]
pub(crate) async fn ready(
    axum::Extension(pool): axum::Extension<sqlx::SqlitePool>,
    axum::Extension(_workers): axum::Extension<workers::FaceWorkers>,
) -> error::ApiResult<axum::Json<Health>> {
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|_| error::ApiError::factor_unavailable())?;
    Ok(axum::Json(Health { status: "ok" }))
}
