use axum::{routing::get, Router};

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(|| async { "Hello from Showflow Rust Backend!" }));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Backend server running on http://localhost:8080");
    
    axum::serve(listener, app).await.unwrap();
}