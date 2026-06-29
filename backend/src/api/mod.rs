mod deployments;
mod domains;
mod env;
mod health;
mod projects;
mod settings;
mod webhooks;

use axum::Router;
use std::sync::Arc;

use crate::db::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(health::routes())
        .merge(projects::routes())
        .merge(deployments::routes())
        .merge(env::routes())
        .merge(settings::routes())
        .merge(webhooks::routes())
        .merge(domains::routes())
}

pub use domains::normalize_host;
