use color_eyre::eyre::*;
use toasty_cli::ToastyCli;

#[tokio::main]
async fn main() -> Result {
    aegis_backend::load_dotenv();
    aegis_backend::init_tracing();

    let db = aegis_backend::db::connect().await.unwrap();

    ToastyCli::new(db).parse_and_run().await.unwrap();

    Ok(())
}
