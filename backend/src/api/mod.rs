mod deployments;
mod env;
mod health;
mod projects;
pub mod static_serve;

use axum::Router;
use std::sync::Arc;

use crate::db::AppState;

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(projects::routes(state.clone()))
        .merge(deployments::routes(state.clone()))
        .merge(env::routes(state.clone()))
        .merge(static_serve::routes(state))
}
