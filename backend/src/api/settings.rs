use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{SettingsResponse, UpdateSettingsRequest};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/api/settings", get(get_settings).patch(patch_settings))
}

async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, (StatusCode, String)> {
    let settings = state
        .get_all_settings()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(SettingsResponse { settings }))
}

async fn patch_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<SettingsResponse>, (StatusCode, String)> {
    if let Some(secs) = body.poll_interval_secs {
        if secs < 5 {
            return Err((
                StatusCode::BAD_REQUEST,
                "poll_interval_secs must be >= 5".into(),
            ));
        }
        state
            .set_setting("poll_interval_secs", &secs.to_string())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let settings = state
        .get_all_settings()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(SettingsResponse { settings }))
}
