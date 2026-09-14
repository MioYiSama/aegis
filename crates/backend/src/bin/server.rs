use aegis_backend::state::AppState;
use color_eyre::eyre::*;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result {
    aegis_backend::load_dotenv();
    aegis_backend::init_tracing();

    let database_url = aegis_backend::db::url_from_env().context("DATABASE_URL not set")?;
    let state = AppState::new(&database_url).await?;
    let router = aegis_backend::routes::router().with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("Server listening on 0.0.0.0:3000");

    axum::serve(listener, router).await?;

    Ok(())
}
