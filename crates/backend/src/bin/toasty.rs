use color_eyre::eyre::*;
use toasty_cli::ToastyCli;

#[tokio::main]
async fn main() -> Result {
    aegis_backend::load_dotenv();
    aegis_backend::init_tracing();

    let database_url = aegis_backend::db::url_from_env().context("DATABASE_URL not set")?;
    let db = aegis_backend::db::connect(&database_url).await.unwrap();

    ToastyCli::new(db).parse_and_run().await.unwrap();

    Ok(())
}
