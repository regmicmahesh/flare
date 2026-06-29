use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{Deployment, DeploymentListResponse, LogsResponse};

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects/{id}/deployments", get(list_deployments))
        .route("/api/deployments/{id}", get(get_deployment))
        .route("/api/deployments/{id}/logs", get(get_logs))
        .with_state(state)
}

async fn list_deployments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeploymentListResponse>, (StatusCode, String)> {
    let deployments = state
        .list_deployments(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(DeploymentListResponse { deployments }))
}

async fn get_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Deployment>, (StatusCode, String)> {
    state
        .get_deployment(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))
}

async fn get_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LogsResponse>, (StatusCode, String)> {
    let logs = state
        .get_logs(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(LogsResponse { logs }))
}
