use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use http::HeaderValue;
use sqlx::sqlite::SqlitePoolOptions;
use tower_http::{
    cors::CorsLayer,
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
    // Storage directory from env or default
    let data_dir: PathBuf = PathBuf::from(
        std::env::var("BENCHWARMER_DATA_DIR").unwrap_or_else(|_| "./data".to_string()),
    );

    // Set up sqlite pool
    let db_path = data_dir.join("benchwarmer.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("Failed to connect to database");
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let storage = Arc::new(storage::Storage::new(&data_dir, pool));

    // Initialize tracing
    let logging_dir = data_dir.join("logs");
    let file_appender = tracing_appender::rolling::daily(logging_dir, "server.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let (journald_layer, stdout_layer) = match tracing_journald::layer() {
        Ok(layer) => (Some(layer), None),
        Err(_) => (None, Some(tracing_subscriber::fmt::layer())),
    };

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
        .with(journald_layer)
        .with(stdout_layer)
        .init();

    tracing::info!(data_dir = %data_dir.display(), "Starting benchwarmer server");

    // Set up CORS for redirecting
    let cors = CorsLayer::new()
        .allow_origin("https://ui.perfetto.dev".parse::<HeaderValue>().unwrap())
        .allow_methods([http::Method::GET]);

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
        .route("/{org}/{repo}/{commit}/trace", get(routes::get_trace_file))
        .layer(cors)
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
        .with_state(Arc::clone(&storage));

    // Spawn periodic cleanup of temporary extraction directories (every 6 hours)
    tokio::spawn({
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                if let Err(e) = storage.clean_temp_dirs().await {
                    tracing::error!("Temp directory cleanup failed: {e}");
                }
            }
        }
    });

    tracing::info!(%addr, "Listening");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
