use thiserror::Error;
use utoipa::openapi::InfoBuilder;

#[derive(Debug, Error)]
enum OpenApiError {
    #[error("Failed to serialize OpenAPI")]
    ToJson(#[from] serde_json::Error),
    #[error("Failed to write OpenAPI file")]
    FsWrite(#[from] std::io::Error),
}

fn main() -> Result<(), OpenApiError> {
    let mut api = aegis_backend::routes::router().into_openapi();

    api.info = InfoBuilder::new()
        .title(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .build();

    std::fs::write("docs/openapi.json", api.to_pretty_json()?)?;

    Ok(())
}
