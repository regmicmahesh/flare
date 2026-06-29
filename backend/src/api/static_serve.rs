//! Static deployment serving with SPA fallback, production aliases, and optional password protection.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::db::AppState;
use crate::models::{check_project_access, Project};

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_deploy/{deployment_id}", get(serve_deploy_root))
        .route("/_deploy/{deployment_id}/{*path}", get(serve_deploy_path))
        .route("/p/{project_id}", get(serve_project_root))
        .route("/p/{project_id}/{*path}", get(serve_project_path))
        .route("/s/{slug}", get(serve_slug_root))
        .route("/s/{slug}/{*path}", get(serve_slug_path))
        .with_state(state)
}

async fn serve_deploy_root(
    State(state): State<Arc<AppState>>,
    Path(deployment_id): Path<String>,
) -> Response {
    serve_from_deployment(&state, &deployment_id, "").await
}

async fn serve_deploy_path(
    State(state): State<Arc<AppState>>,
    Path((deployment_id, path)): Path<(String, String)>,
) -> Response {
    serve_from_deployment(&state, &deployment_id, &path).await
}

async fn serve_project_root(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    req: Request<Body>,
) -> Response {
    match load_project_by_id(&state, &project_id).await {
        Ok(project) => {
            if let Some(denied) = protection_denied(&project, &req) {
                return denied;
            }
            alias_response(
                &state,
                resolve_alias_dep_id(&state, &project).await,
                "",
            )
            .await
        }
        Err(r) => r,
    }
}

async fn serve_project_path(
    State(state): State<Arc<AppState>>,
    Path((project_id, path)): Path<(String, String)>,
    req: Request<Body>,
) -> Response {
    match load_project_by_id(&state, &project_id).await {
        Ok(project) => {
            if let Some(denied) = protection_denied(&project, &req) {
                return denied;
            }
            alias_response(
                &state,
                resolve_alias_dep_id(&state, &project).await,
                &path,
            )
            .await
        }
        Err(r) => r,
    }
}

async fn serve_slug_root(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    req: Request<Body>,
) -> Response {
    match load_project_by_slug(&state, &slug).await {
        Ok(project) => {
            if let Some(denied) = protection_denied(&project, &req) {
                return denied;
            }
            alias_response(
                &state,
                resolve_alias_dep_id(&state, &project).await,
                "",
            )
            .await
        }
        Err(r) => r,
    }
}

async fn serve_slug_path(
    State(state): State<Arc<AppState>>,
    Path((slug, path)): Path<(String, String)>,
    req: Request<Body>,
) -> Response {
    match load_project_by_slug(&state, &slug).await {
        Ok(project) => {
            if let Some(denied) = protection_denied(&project, &req) {
                return denied;
            }
            alias_response(
                &state,
                resolve_alias_dep_id(&state, &project).await,
                &path,
            )
            .await
        }
        Err(r) => r,
    }
}

/// Shared protection gate for /p, /s, and custom domains.
pub fn protection_denied(project: &Project, req: &Request<Body>) -> Option<Response> {
    let Some(secret) = project.protect_secret.as_deref() else {
        return None;
    };
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok());
    if check_project_access(&project.id, secret, auth, cookie) {
        None
    } else {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                "password required — send Authorization: Bearer <token> or cookie flare_access={project_id}:{token}",
            )
                .into_response(),
        )
    }
}

async fn load_project_by_id(state: &AppState, project_id: &str) -> Result<Project, Response> {
    match state.get_project(project_id).await {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err((StatusCode::NOT_FOUND, "project not found").into_response()),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()),
    }
}

async fn load_project_by_slug(state: &AppState, slug: &str) -> Result<Project, Response> {
    match state.get_project_by_slug(slug).await {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err((StatusCode::NOT_FOUND, "project not found").into_response()),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()),
    }
}

enum ResolveErr {
    Internal(String),
}

async fn alias_response(
    state: &AppState,
    resolved: Result<Option<String>, ResolveErr>,
    path: &str,
) -> Response {
    match resolved {
        Ok(Some(dep_id)) => serve_from_deployment(state, &dep_id, path).await,
        Ok(None) => (StatusCode::NOT_FOUND, "no ready deployment").into_response(),
        Err(ResolveErr::Internal(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn resolve_alias_dep_id(
    state: &AppState,
    project: &Project,
) -> Result<Option<String>, ResolveErr> {
    state
        .resolve_alias_deployment(project)
        .await
        .map(|d| d.map(|d| d.id))
        .map_err(|e| ResolveErr::Internal(e.to_string()))
}

/// Serve a file from a deployment directory with SPA fallback:
/// if the path is missing and has no file extension, serve index.html.
async fn serve_from_deployment(state: &AppState, deployment_id: &str, rel: &str) -> Response {
    let rel = rel.trim_start_matches('/');
    if has_parent_component(rel) {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }

    let root = state.data_dir.join("deployments").join(deployment_id);
    if !root.is_dir() {
        return (StatusCode::NOT_FOUND, "deployment not found").into_response();
    }

    let candidate = if rel.is_empty() {
        root.join("index.html")
    } else {
        root.join(rel)
    };

    // If requesting a directory-like path ending with /, try index.html inside.
    let file_to_try = if rel.ends_with('/') {
        root.join(rel).join("index.html")
    } else {
        candidate.clone()
    };

    if file_to_try.is_file() {
        return serve_file(&file_to_try).await;
    }

    // Try the exact relative path as a file under root.
    if !rel.is_empty() && candidate.is_file() {
        return serve_file(&candidate).await;
    }

    // SPA fallback: no extension => index.html of the deployment.
    if !has_file_extension(rel) {
        let index = root.join("index.html");
        if index.is_file() {
            return serve_file(&index).await;
        }
    }

    // Fall back to ServeDir for content-type / range handling on existing files.
    let req_path = if rel.is_empty() {
        "/"
    } else {
        &format!("/{rel}")
    };
    let req = Request::builder()
        .uri(req_path)
        .body(Body::empty())
        .unwrap();
    match ServeDir::new(&root).oneshot(req).await {
        Ok(res) if res.status() == StatusCode::NOT_FOUND && !has_file_extension(rel) => {
            let index = root.join("index.html");
            if index.is_file() {
                serve_file(&index).await
            } else {
                res.into_response()
            }
        }
        Ok(res) => res.into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "serve error").into_response(),
    }
}

fn has_parent_component(rel: &str) -> bool {
    FsPath::new(rel)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
}

fn has_file_extension(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.contains('.') && !name.starts_with('.')
}

async fn serve_file(path: &PathBuf) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let mime = mime_guess(path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn mime_guess(path: &FsPath) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" | "map" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}
