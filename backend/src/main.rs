mod api;
mod db;
mod models;
mod services;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::AppState;
use crate::services::poller::start_poller;
use crate::services::worker::BuildWorker;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flare=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let data_dir =
        PathBuf::from(std::env::var("FLARE_DATA_DIR").unwrap_or_else(|_| "./data".into()));
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(data_dir.join("repos"))?;
    std::fs::create_dir_all(data_dir.join("deployments"))?;
    std::fs::create_dir_all(data_dir.join("builds"))?;

    let db_path = data_dir.join("flare.db");
    let state = Arc::new(AppState::new(&db_path, data_dir.clone()).await?);

    let worker = BuildWorker::new(state.clone());
    worker.spawn();

    start_poller(state.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let static_deploy = ServeDir::new(data_dir.join("deployments"));

    let app = Router::new()
        .merge(api::routes())
        .nest_service("/_deploy", static_deploy)
        .fallback(domain_fallback)
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Flare API listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Serve the project's latest ready deployment when `Host` matches a custom domain.
/// For real use, point DNS or `/etc/hosts` at this Flare instance.
async fn domain_fallback(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let host_hdr = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host = api::normalize_host(host_hdr);
    if host.is_empty() || host.starts_with("localhost") || host.starts_with("127.0.0.1") {
        return (StatusCode::NOT_FOUND, "not found (no custom domain match)").into_response();
    }

    let domain = match state.get_domain_by_host(&host).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "unknown host").into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let dep = match state.latest_ready_deployment(&domain.project_id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "no ready deployment for this domain").into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let deploy_dir = state.data_dir.join("deployments").join(&dep.id);
    if !deploy_dir.is_dir() {
        return (StatusCode::NOT_FOUND, "deployment files missing").into_response();
    }

    let path = req.uri().path();
    let rel = path.trim_start_matches('/');
    let candidate = if rel.is_empty() {
        deploy_dir.join("index.html")
    } else {
        deploy_dir.join(rel)
    };

    let Ok(canon_root) = deploy_dir.canonicalize() else {
        return (StatusCode::NOT_FOUND, "deployment not found").into_response();
    };

    let file_path = if candidate.is_file() {
        candidate
    } else if candidate.is_dir() && candidate.join("index.html").is_file() {
        candidate.join("index.html")
    } else if deploy_dir.join("index.html").is_file() && !rel.contains('.') {
        deploy_dir.join("index.html")
    } else {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    };

    let Ok(canon_file) = file_path.canonicalize() else {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    };
    if !canon_file.starts_with(&canon_root) {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }

    match tokio::fs::read(&canon_file).await {
        Ok(bytes) => {
            let mut res = Response::new(Body::from(bytes));
            *res.status_mut() = StatusCode::OK;
            if let Some(ct) = guess_content_type(&canon_file) {
                if let Ok(v) = HeaderValue::from_str(ct) {
                    res.headers_mut().insert(header::CONTENT_TYPE, v);
                }
            }
            res
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "read error").into_response(),
    }
}

fn guess_content_type(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => Some("text/html; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "js" | "mjs" => Some("application/javascript; charset=utf-8"),
        "json" => Some("application/json"),
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "ico" => Some("image/x-icon"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "txt" => Some("text/plain; charset=utf-8"),
        "xml" => Some("application/xml"),
        "map" => Some("application/json"),
        _ => Some("application/octet-stream"),
    }
}
