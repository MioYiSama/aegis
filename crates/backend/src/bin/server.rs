use aegis_backend::{handler::router, state::AppState};
use color_eyre::eyre::*;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result {
    aegis_backend::load_dotenv();
    aegis_backend::init_tracing();

    let state = AppState::new().await?;
    let router = router().with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("Server listening on 0.0.0.0:3000");

    axum::serve(listener, router).await?;

    Ok(())
}
