use axum::{routing::get, Json, Router};

use crate::models::HealthResponse;

pub fn routes() -> Router {
    Router::new().route("/api/health", get(health))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
