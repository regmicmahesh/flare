use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde::Serialize;
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{new_id, EnvVar, UpsertEnvRequest};

#[derive(Serialize)]
struct EnvList {
    env: Vec<EnvVar>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/projects/{id}/env",
            get(list_env).post(upsert_env).delete(delete_env_all),
        )
        .route(
            "/api/projects/{id}/env/{key}",
            axum::routing::delete(delete_env),
        )
}

async fn list_env(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<EnvList>, (StatusCode, String)> {
    let env = state
        .list_env(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EnvList { env }))
}

async fn upsert_env(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpsertEnvRequest>,
) -> Result<Json<EnvVar>, (StatusCode, String)> {
    let v = EnvVar {
        id: new_id(),
        project_id: id,
        key: body.key,
        value: body.value,
        created_at: Utc::now(),
    };
    state
        .upsert_env(&v)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(v))
}

async fn delete_env(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .delete_env(&id, &key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_env_all() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}
