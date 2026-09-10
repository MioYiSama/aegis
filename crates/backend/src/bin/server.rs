use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let (router, _) = aegis_backend::routes::user::router();

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    aegis_core::f();
    axum::serve(listener, router).await.unwrap();
}
