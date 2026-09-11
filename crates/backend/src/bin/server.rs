use axum::Router;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

#[tokio::main]
async fn main() {
    let router: Router<_> = aegis_backend::routes::router().into();
    let router = router.layer(CorsLayer::very_permissive());

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, router).await.unwrap();
}
