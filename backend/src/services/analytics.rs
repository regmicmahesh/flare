//! Lightweight request analytics middleware for preview / production aliases.

use axum::{body::Body, extract::State, http::Request, middleware::Next, response::Response};
use std::sync::Arc;

use crate::db::AppState;

/// Record hits for `/_deploy/{id}`, `/p/{project_id}`, and `/s/{slug}` requests.
pub async fn track_requests(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let response = next.run(req).await;

    // Only count successful-ish responses (avoid flooding on 404 noise for missing assets is ok;
    // we still count 404 for simplicity — path was attempted).
    if let Some(dep_id) = extract_deployment_id_from_path(&path) {
        let state = state.clone();
        tokio::spawn(async move {
            let _ = state.record_hit(&dep_id).await;
        });
        return response;
    }

    if let Some(rest) = path.strip_prefix("/p/") {
        let project_id = rest.split('/').next().unwrap_or("").to_string();
        if !project_id.is_empty() {
            let state = state.clone();
            tokio::spawn(async move {
                if let Ok(Some(project)) = state.get_project(&project_id).await {
                    if let Ok(Some(dep)) = state.resolve_alias_deployment(&project).await {
                        let _ = state.record_hit(&dep.id).await;
                    }
                }
            });
        }
        return response;
    }

    if let Some(rest) = path.strip_prefix("/s/") {
        let slug = rest.split('/').next().unwrap_or("").to_string();
        if !slug.is_empty() {
            let state = state.clone();
            tokio::spawn(async move {
                if let Ok(Some(project)) = state.get_project_by_slug(&slug).await {
                    if let Ok(Some(dep)) = state.resolve_alias_deployment(&project).await {
                        let _ = state.record_hit(&dep.id).await;
                    }
                }
            });
        }
    }

    response
}

fn extract_deployment_id_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/_deploy/")?;
    let id = rest.split('/').next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::extract_deployment_id_from_path;

    #[test]
    fn parse_deploy_path() {
        assert_eq!(
            extract_deployment_id_from_path("/_deploy/abc-123/index.html").as_deref(),
            Some("abc-123")
        );
        assert_eq!(
            extract_deployment_id_from_path("/_deploy/abc-123/").as_deref(),
            Some("abc-123")
        );
        assert_eq!(extract_deployment_id_from_path("/api/projects"), None);
        assert_eq!(extract_deployment_id_from_path("/p/foo"), None);
    }
}
