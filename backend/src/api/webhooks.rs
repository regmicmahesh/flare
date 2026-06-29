use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use url::Url;

use crate::db::AppState;
use crate::models::{new_id, CreateWebhookRequest, Webhook, WebhookListResponse, WEBHOOK_EVENTS};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/projects/{id}/webhooks",
            get(list_webhooks).post(create_webhook),
        )
        .route(
            "/api/projects/{id}/webhooks/{webhook_id}",
            axum::routing::delete(delete_webhook),
        )
}

async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WebhookListResponse>, (StatusCode, String)> {
    let _ = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let webhooks = state
        .list_webhooks(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(WebhookListResponse { webhooks }))
}

async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<Webhook>), (StatusCode, String)> {
    let _ = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let url_str = body.url.trim();
    if url_str.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "url is required".into()));
    }
    let parsed =
        Url::parse(url_str).map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid url: {e}")))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err((StatusCode::BAD_REQUEST, "url must be http or https".into()));
    }

    let events = normalize_events(body.events.as_deref())?;
    let w = Webhook {
        id: new_id(),
        project_id: id,
        url: url_str.to_string(),
        events,
        created_at: Utc::now(),
    };
    state
        .insert_webhook(&w)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(w)))
}

async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path((id, webhook_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let n = state
        .delete_webhook(&id, &webhook_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if n == 0 {
        return Err((StatusCode::NOT_FOUND, "webhook not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

fn normalize_events(requested: Option<&[String]>) -> Result<String, (StatusCode, String)> {
    let known: Vec<&str> = WEBHOOK_EVENTS.to_vec();
    let list: Vec<String> = match requested {
        None | Some([]) => known.iter().map(|s| (*s).to_string()).collect(),
        Some(evs) => {
            let mut out = Vec::new();
            for e in evs {
                let t = e.trim();
                if t.is_empty() {
                    continue;
                }
                if !known.contains(&t) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("unknown event '{t}'; allowed: {}", known.join(", ")),
                    ));
                }
                if !out.iter().any(|x: &String| x == t) {
                    out.push(t.to_string());
                }
            }
            if out.is_empty() {
                known.iter().map(|s| (*s).to_string()).collect()
            } else {
                out
            }
        }
    };
    Ok(list.join(","))
}
