use color_eyre::eyre::*;
use utoipa::openapi::InfoBuilder;

fn main() -> Result {
    color_eyre::install()?;

    let mut api = aegis_backend::routes::router().into_openapi();
    api.info = InfoBuilder::new()
        .title(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .build();

    std::fs::write("docs/openapi.json", api.to_pretty_json()?)?;

    Ok(())
}
