mod deployments;
mod domains;
mod env;
mod health;
mod projects;
mod settings;
pub mod static_serve;
mod webhooks;

use axum::Router;
use std::sync::Arc;

use crate::db::AppState;

/// API routes that share `Arc<AppState>` (apply `.with_state` in `main` together with fallback).
pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(health::routes())
        .merge(projects::routes())
        .merge(deployments::routes())
        .merge(env::routes())
        .merge(settings::routes())
        .merge(webhooks::routes())
        .merge(domains::routes())
}

pub fn static_routes(state: Arc<AppState>) -> Router {
    static_serve::routes(state)
}

pub use domains::normalize_host;
