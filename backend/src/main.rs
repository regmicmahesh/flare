mod api;
mod db;
mod models;
mod services;

use axum::{middleware, Router};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::AppState;
use crate::services::analytics::track_requests;
use crate::services::poller::start_poller;
use crate::services::worker::BuildWorker;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flare=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let data_dir =
        PathBuf::from(std::env::var("FLARE_DATA_DIR").unwrap_or_else(|_| "./data".into()));
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(data_dir.join("repos"))?;
    std::fs::create_dir_all(data_dir.join("deployments"))?;
    std::fs::create_dir_all(data_dir.join("builds"))?;

    let db_path = data_dir.join("flare.db");
    let state = Arc::new(AppState::new(&db_path, data_dir.clone()).await?);

    let worker = BuildWorker::new(state.clone());
    worker.spawn();

    start_poller(state.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .merge(api::routes(state.clone()))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            track_requests,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Flare API listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
