use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{
    Deployment, DeploymentDiffResponse, DeploymentListResponse, DeploymentStatsResponse,
    LogsResponse,
};
use crate::services::git::changed_files;

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects/{id}/deployments", get(list_deployments))
        .route("/api/deployments/{id}", get(get_deployment))
        .route("/api/deployments/{id}/logs", get(get_logs))
        .route("/api/deployments/{id}/stats", get(deployment_stats))
        .route("/api/deployments/{a}/diff/{b}", get(deployment_diff))
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

async fn deployment_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeploymentStatsResponse>, (StatusCode, String)> {
    let _dep = state
        .get_deployment(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))?;

    let hits = state
        .get_deployment_hits(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DeploymentStatsResponse {
        deployment_id: id,
        hits: hits.as_ref().map(|h| h.hits).unwrap_or(0),
        last_hit: hits.map(|h| h.last_hit),
    }))
}

async fn deployment_diff(
    State(state): State<Arc<AppState>>,
    Path((a, b)): Path<(String, String)>,
) -> Result<Json<DeploymentDiffResponse>, (StatusCode, String)> {
    let dep_a = state
        .get_deployment(&a)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment A not found".into()))?;
    let dep_b = state
        .get_deployment(&b)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment B not found".into()))?;

    if dep_a.project_id != dep_b.project_id {
        return Err((
            StatusCode::BAD_REQUEST,
            "deployments must belong to the same project".into(),
        ));
    }

    let repo_path = state.data_dir.join("repos").join(&dep_a.project_id);
    if !repo_path.join(".git").exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            "local git repo not found for project".into(),
        ));
    }

    let files = changed_files(&repo_path, &dep_a.commit_sha, &dep_b.commit_sha)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DeploymentDiffResponse {
        a,
        b,
        commit_sha_a: dep_a.commit_sha,
        commit_sha_b: dep_b.commit_sha,
        files,
    }))
}
