use thiserror::Error;
use utoipa::openapi::{InfoBuilder, OpenApiBuilder};

#[derive(Debug, Error)]
enum OpenApiError {
    #[error("Failed to call api.to_pretty_json")]
    ToJson(#[from] serde_json::Error),
    #[error("Failed to call std::fs::write")]
    FsWrite(#[from] std::io::Error),
}

fn main() -> Result<(), OpenApiError> {
    let (_, user_api) = aegis_backend::routes::user::router();

    let info = InfoBuilder::new()
        .title(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"));

    let api = OpenApiBuilder::new()
        .info(info)
        .build()
        .nest("/user", user_api);

    let json = api.to_pretty_json()?;
    std::fs::write("docs/openapi.json", json)?;

    Ok(())
}
