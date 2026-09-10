use axum::Router;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let router = Router::new();

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    aegis_core::f();
    axum::serve(listener, router).await.unwrap();
}
