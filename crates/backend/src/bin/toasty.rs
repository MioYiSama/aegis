use aegis_backend::state::connect_db;
use color_eyre::eyre::*;
use toasty_cli::ToastyCli;

#[tokio::main]
async fn main() -> Result {
    aegis_backend::load_dotenv();

    let db = connect_db().await?;
    ToastyCli::new(db).parse_and_run().await.unwrap();

    Ok(())
}
