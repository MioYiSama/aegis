use aegis_backend::state::AppState;
use axum::Router;
use color_eyre::eyre::*;
use tokio::net::TcpListener;
use tower_http::{catch_panic::CatchPanicLayer, cors::CorsLayer};

#[tokio::main]
async fn main() -> Result {
    aegis_backend::load_dotenv();
    aegis_backend::init_tracing();

    let state = AppState::new().await?;

    let router: Router<_> = aegis_backend::routes::router().into();
    let router = router
        .layer(CorsLayer::permissive())
        .layer(CatchPanicLayer::new())
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("Server listening on 0.0.0.0:3000");

    axum::serve(listener, router).await?;

    Ok(())
}
