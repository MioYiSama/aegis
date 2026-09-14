use color_eyre::eyre::*;

fn main() -> Result {
    color_eyre::install()?;

    let api = aegis_backend::routes::openapi();

    std::fs::write("docs/openapi.json", api.to_pretty_json()?)?;

    Ok(())
}
