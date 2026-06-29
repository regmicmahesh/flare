use axum::{routing::get, Json, Router};
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{HealthResponse, VersionResponse};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/version", get(version))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
        features: vec![
            "public-github-clone",
            "commit-polling",
            "scheduled-redeploy",
            "password-protection",
            "spa-aliases",
            "custom-domains",
            "webhooks",
            "analytics",
            "rollback",
            "ignore-patterns",
            "project-search",
        ],
    })
}
