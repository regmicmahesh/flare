use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{Deployment, DeploymentListResponse, LogsResponse};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/projects/{id}/deployments", get(list_deployments))
        .route("/api/deployments/{id}", get(get_deployment))
        .route("/api/deployments/{id}/logs", get(get_logs))
        .route("/api/deployments/{id}/cancel", post(cancel_deployment))
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

/// Best-effort cancel: marks queued/building deployments as `cancelled`.
/// The worker re-checks status before heavy steps.
async fn cancel_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Deployment>, (StatusCode, String)> {
    let mut dep = state
        .get_deployment(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))?;

    if dep.status != "queued" && dep.status != "building" {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "cannot cancel deployment in status '{}'; must be queued or building",
                dep.status
            ),
        ));
    }

    dep.status = "cancelled".into();
    dep.finished_at = Some(Utc::now());
    state
        .update_deployment(&dep)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state
        .append_log(&dep.id, "Deployment cancelled by user")
        .await;
    Ok(Json(dep))
}
