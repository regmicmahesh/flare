use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{new_id, CreateDomainRequest, Domain, DomainListResponse};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/projects/{id}/domains",
            get(list_domains).post(create_domain),
        )
        .route(
            "/api/projects/{id}/domains/{domain_id}",
            axum::routing::delete(delete_domain),
        )
}

/// Normalize Host header / user input: lowercase, strip port, trim.
pub fn normalize_host(raw: &str) -> String {
    let s = raw.trim().to_lowercase();
    // strip port if present (but not IPv6 brackets for MVP — hosts are names)
    if let Some((host, _port)) = s.rsplit_once(':') {
        if !host.contains(']') && host.parse::<std::net::Ipv6Addr>().is_err() {
            // only strip if the part after : looks like a port
            if _port.chars().all(|c| c.is_ascii_digit()) {
                return host.to_string();
            }
        }
    }
    s
}

async fn list_domains(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DomainListResponse>, (StatusCode, String)> {
    let _ = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let domains = state
        .list_domains(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(DomainListResponse { domains }))
}

async fn create_domain(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateDomainRequest>,
) -> Result<(StatusCode, Json<Domain>), (StatusCode, String)> {
    let _ = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let host = normalize_host(&body.host);
    if host.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "host is required".into()));
    }
    if host.contains('/') || host.contains(' ') {
        return Err((StatusCode::BAD_REQUEST, "invalid host".into()));
    }

    if let Ok(Some(_)) = state.get_domain_by_host(&host).await {
        return Err((
            StatusCode::CONFLICT,
            format!("host '{host}' is already mapped"),
        ));
    }

    let d = Domain {
        id: new_id(),
        project_id: id,
        host,
        created_at: Utc::now(),
    };
    state.insert_domain(&d).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("UNIQUE") || msg.contains("unique") {
            (StatusCode::CONFLICT, "host is already mapped".into())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    })?;
    Ok((StatusCode::CREATED, Json(d)))
}

async fn delete_domain(
    State(state): State<Arc<AppState>>,
    Path((id, domain_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let n = state
        .delete_domain(&id, &domain_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if n == 0 {
        return Err((StatusCode::NOT_FOUND, "domain not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
