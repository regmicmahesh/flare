use axum::{routing::get, Json, Router};
use std::sync::Arc;

use crate::db::AppState;
use crate::models::HealthResponse;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/api/health", get(health))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
