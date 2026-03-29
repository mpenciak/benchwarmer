use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use tower_http::{
    request_id::{MakeRequestId, RequestId, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct Id;

impl MakeRequestId for Id {
    fn make_request_id<B>(&mut self, _request: &http::Request<B>) -> Option<RequestId> {
        let id = uuid::Uuid::now_v7().to_string();
        Some(RequestId::new(id.parse().unwrap()))
    }
}

use benchwarmer_server::{routes, storage};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Storage directory from env or default
    let data_dir = std::env::var("BENCHWARMER_DATA_DIR").unwrap_or_else(|_| "./data".to_string());
    let storage = Arc::new(storage::Storage::new(&data_dir));

    tracing::info!(data_dir = %data_dir, "Starting benchwarmer server");

    // Bind address from env or default
    let addr: SocketAddr = std::env::var("BENCHWARMER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()
        .expect("Invalid BENCHWARMER_ADDR");

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/{org}/{repo}/{commit}", post(routes::upload_artifact))
        .route(
            "/{org}/{repo}/{commit}/report/weekly",
            get(routes::get_report_weekly),
        )
        .route(
            "/{org}/{repo}/{commit}/report/pr",
            get(routes::get_report_pr),
        )
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &http::Request<_>| {
                let request_id = request
                    .headers()
                    .get("x-request-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unknown");

                tracing::info_span!(
                    "request",
                    request_id = %request_id,
                )
            }),
        )
        .layer(SetRequestIdLayer::x_request_id(Id))
        .with_state(storage);

    tracing::info!(%addr, "Listening");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
