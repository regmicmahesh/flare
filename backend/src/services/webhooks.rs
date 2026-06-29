use serde_json::json;
use std::sync::Arc;

use crate::db::AppState;
use crate::models::Deployment;

/// Fire-and-forget POSTs to project webhooks that subscribe to `event`.
pub fn dispatch_event(state: Arc<AppState>, project_id: String, event: &str, dep: &Deployment) {
    let event = event.to_string();
    let payload = json!({
        "event": event,
        "deployment": {
            "id": dep.id,
            "project_id": dep.project_id,
            "commit_sha": dep.commit_sha,
            "commit_message": dep.commit_message,
            "commit_author": dep.commit_author,
            "branch": dep.branch,
            "status": dep.status,
            "framework": dep.framework,
            "url_path": dep.url_path,
            "error_message": dep.error_message,
            "created_at": dep.created_at,
            "finished_at": dep.finished_at,
        }
    });

    tokio::spawn(async move {
        let hooks = match state.list_webhooks(&project_id).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("list webhooks for {project_id}: {e}");
                return;
            }
        };

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("webhook http client: {e}");
                return;
            }
        };

        for hook in hooks {
            let subscribed = hook.events.split(',').map(str::trim).any(|e| e == event);
            if !subscribed {
                continue;
            }
            let url = hook.url.clone();
            let body = payload.clone();
            let client = client.clone();
            tokio::spawn(async move {
                match client.post(&url).json(&body).send().await {
                    Ok(res) if res.status().is_success() => {
                        tracing::debug!("webhook POST {url} ok ({})", res.status());
                    }
                    Ok(res) => {
                        tracing::warn!("webhook POST {url} status {}", res.status());
                    }
                    Err(e) => {
                        tracing::warn!("webhook POST {url} failed: {e}");
                    }
                }
            });
        }
    });
}
