use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::db::AppState;
use crate::models::{
    hash_password, new_id, slugify, ActivityEntry, ActivityResponse, CommitEntry, CommitsResponse,
    CreateProjectRequest, DeployRequest, Deployment, DeploymentHitRow, Domain, ImportProjectRequest,
    Project, ProjectExport, ProjectExportWebhook, ProjectListResponse, ProjectStatsResponse,
    PromoteRequest, ProtectionResponse, RollbackRequest, SetProtectionRequest,
    UpdateProjectRequest, WEBHOOK_EVENTS, Webhook,
};
use crate::services::framework::detect_framework;
use crate::services::git::{
    clone_or_fetch, list_commits, parse_github_input, remote_head, should_skip_build,
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/import", post(import_project))
        .route(
            "/api/projects/{id}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route("/api/projects/{id}/export", get(export_project))
        .route("/api/projects/{id}/deploy", post(trigger_deploy))
        .route("/api/projects/{id}/promote", post(promote_deployment))
        .route("/api/projects/{id}/rollback", post(rollback_deployment))
        .route("/api/projects/{id}/stats", get(project_stats))
        .route("/api/projects/{id}/commits", get(list_project_commits))
        .route("/api/projects/{id}/activity", get(project_activity))
        .route("/api/projects/{id}/protection", post(set_protection))
}

#[derive(Debug, Deserialize)]
struct CommitsQuery {
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct ListProjectsQuery {
    /// Case-insensitive filter on name, slug, or owner_repo.
    q: Option<String>,
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<Json<ProjectListResponse>, (StatusCode, String)> {
    let projects = if let Some(q) = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        state
            .search_projects(q)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        state
            .list_projects()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };
    Ok(Json(ProjectListResponse { projects }))
}

async fn get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Project>, (StatusCode, String)> {
    state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))
}

/// Export project settings as redacted JSON (env keys only, no secret values).
async fn export_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectExport>, (StatusCode, String)> {
    let project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let env = state
        .list_env(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let domains = state
        .list_domains(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let webhooks = state
        .list_webhooks(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ProjectExport {
        version: 1,
        name: project.name,
        github_url: project.github_url,
        owner_repo: project.owner_repo,
        default_branch: project.default_branch,
        framework: project.framework,
        root_directory: project.root_directory,
        build_command: project.build_command,
        output_directory: project.output_directory,
        install_command: project.install_command,
        ignore_patterns: project.ignore_patterns,
        poll_enabled: project.poll_enabled,
        redeploy_interval_mins: project.redeploy_interval_mins,
        password_protect: project.protect_secret.is_some(),
        env_keys: env.into_iter().map(|e| e.key).collect(),
        domain_hosts: domains.into_iter().map(|d| d.host).collect(),
        webhooks: webhooks
            .into_iter()
            .map(|w| ProjectExportWebhook {
                url: w.url,
                events: w.events,
            })
            .collect(),
    }))
}

/// Create a project from GitHub plus optional non-secret overrides (from export JSON).
/// Never accepts or stores env values or protection passwords from the import body.
async fn import_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ImportProjectRequest>,
) -> Result<(StatusCode, Json<Project>), (StatusCode, String)> {
    let parsed =
        parse_github_input(&body.github).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let branch = body.branch.clone().unwrap_or_else(|| "main".to_string());
    let name = body.name.clone().unwrap_or_else(|| parsed.repo.clone());
    let now = Utc::now();
    let id = new_id();
    let slug = state
        .allocate_slug(&slugify(&name), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = state.data_dir.join("repos").join(&id);
    clone_or_fetch(&parsed.clone_url, &repo_path, &branch)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("git clone failed: {e}")))?;

    let root = body.root_directory.clone().unwrap_or_else(|| ".".into());
    let work = if root == "." {
        repo_path.clone()
    } else {
        repo_path.join(&root)
    };
    let fw = detect_framework(&work);

    let ignore_patterns = body.ignore_patterns.and_then(|ip| {
        let t = ip.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    });

    let mut project = Project {
        id: id.clone(),
        name,
        slug,
        github_url: parsed.html_url.clone(),
        owner_repo: format!("{}/{}", parsed.owner, parsed.repo),
        default_branch: branch.clone(),
        framework: fw.framework.clone(),
        root_directory: root,
        build_command: body.build_command.or(fw.build_command.clone()),
        output_directory: body.output_directory.or(fw.output_directory.clone()),
        install_command: body.install_command.or(fw.install_command.clone()),
        ignore_patterns,
        // Never import protection passwords / hashes.
        protect_secret: None,
        redeploy_interval_mins: i64::from(body.redeploy_interval_mins.unwrap_or(0)),
        last_commit_sha: None,
        production_deployment_id: None,
        created_at: now,
        updated_at: now,
        poll_enabled: body.poll_enabled.unwrap_or(true),
    };

    let head = remote_head(&repo_path, &branch).await.ok();
    project.last_commit_sha = head.clone();

    state
        .insert_project(&project)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Apply non-secret overrides: domains and webhook URLs only.
    if let Some(hosts) = body.domain_hosts {
        for host_raw in hosts {
            let host = super::normalize_host(&host_raw);
            if host.is_empty() {
                continue;
            }
            let domain = Domain {
                id: new_id(),
                project_id: id.clone(),
                host,
                created_at: Utc::now(),
            };
            // Skip duplicates / conflicts without failing the whole import.
            let _ = state.insert_domain(&domain).await;
        }
    }

    if let Some(hooks) = body.webhooks {
        let default_events = WEBHOOK_EVENTS.join(",");
        for h in hooks {
            let url = h.url.trim().to_string();
            if url.is_empty() {
                continue;
            }
            let events = h
                .events
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&default_events)
                .to_string();
            let wh = Webhook {
                id: new_id(),
                project_id: id.clone(),
                url,
                events,
                created_at: Utc::now(),
            };
            let _ = state.insert_webhook(&wh).await;
        }
    }

    // env_keys from export are accepted for round-trip compatibility but never create rows
    // (no values / secrets). Operator must set secrets via POST /api/projects/{id}/env.
    let _env_keys_hint = body.env_keys.unwrap_or_default();
    let _ = _env_keys_hint;

    // Initial deployment
    if let Some(sha) = head {
        let dep_id = new_id();
        let dep = Deployment {
            id: dep_id.clone(),
            project_id: id,
            commit_sha: sha,
            commit_message: Some("Initial import".into()),
            commit_author: None,
            branch,
            status: "queued".into(),
            framework: project.framework.clone(),
            url_path: None,
            error_message: None,
            changed_files: None,
            created_at: Utc::now(),
            finished_at: None,
        };
        let _ = state.insert_deployment(&dep).await;
        crate::services::webhooks::dispatch_event(
            state.clone(),
            project.id.clone(),
            "deployment.queued",
            &dep,
        );
        state.enqueue_build(dep_id);
    }

    Ok((StatusCode::CREATED, Json(project)))
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<Project>), (StatusCode, String)> {
    let parsed =
        parse_github_input(&body.github).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let branch = body.branch.clone().unwrap_or_else(|| "main".to_string());
    let name = body.name.clone().unwrap_or_else(|| parsed.repo.clone());
    let now = Utc::now();
    let id = new_id();
    let slug = state
        .allocate_slug(&slugify(&name), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = state.data_dir.join("repos").join(&id);
    clone_or_fetch(&parsed.clone_url, &repo_path, &branch)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("git clone failed: {e}")))?;

    let root = body.root_directory.clone().unwrap_or_else(|| ".".into());
    let work = if root == "." {
        repo_path.clone()
    } else {
        repo_path.join(&root)
    };
    let fw = detect_framework(&work);

    let mut project = Project {
        id: id.clone(),
        name,
        slug,
        github_url: parsed.html_url.clone(),
        owner_repo: format!("{}/{}", parsed.owner, parsed.repo),
        default_branch: branch.clone(),
        framework: fw.framework.clone(),
        root_directory: root,
        build_command: body.build_command.or(fw.build_command.clone()),
        output_directory: body.output_directory.or(fw.output_directory.clone()),
        install_command: body.install_command.or(fw.install_command.clone()),
        ignore_patterns: None,
        protect_secret: None,
        redeploy_interval_mins: 0,
        last_commit_sha: None,
        production_deployment_id: None,
        created_at: now,
        updated_at: now,
        poll_enabled: true,
    };

    let head = remote_head(&repo_path, &branch).await.ok();
    project.last_commit_sha = head.clone();

    state
        .insert_project(&project)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Initial deployment
    if let Some(sha) = head {
        let dep_id = new_id();
        let dep = Deployment {
            id: dep_id.clone(),
            project_id: id,
            commit_sha: sha,
            commit_message: Some("Initial import".into()),
            commit_author: None,
            branch,
            status: "queued".into(),
            framework: project.framework.clone(),
            url_path: None,
            error_message: None,
            changed_files: None,
            created_at: Utc::now(),
            finished_at: None,
        };
        let _ = state.insert_deployment(&dep).await;
        crate::services::webhooks::dispatch_event(
            state.clone(),
            project.id.clone(),
            "deployment.queued",
            &dep,
        );
        state.enqueue_build(dep_id);
    }

    Ok((StatusCode::CREATED, Json(project)))
}

async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProjectRequest>,
) -> Result<Json<Project>, (StatusCode, String)> {
    let mut p = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    if let Some(n) = body.name {
        p.name = n;
    }
    if let Some(b) = body.default_branch {
        p.default_branch = b;
    }
    if let Some(r) = body.root_directory {
        p.root_directory = r;
    }
    if let Some(c) = body.build_command {
        p.build_command = Some(c);
    }
    if let Some(o) = body.output_directory {
        p.output_directory = Some(o);
    }
    if let Some(i) = body.install_command {
        p.install_command = Some(i);
    }
    if let Some(ip) = body.ignore_patterns {
        // Empty string clears the field.
        p.ignore_patterns = if ip.trim().is_empty() { None } else { Some(ip) };
    }
    if let Some(pe) = body.poll_enabled {
        p.poll_enabled = pe;
    }
    if let Some(mins) = body.redeploy_interval_mins {
        p.redeploy_interval_mins = i64::from(mins);
    }
    p.updated_at = Utc::now();

    state
        .update_project(&p)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(p))
}

/// Set or clear deployment password protection.
/// Body: `{ "password": "..." }` to enable, `{ "password": null }` to clear.
async fn set_protection(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SetProtectionRequest>,
) -> Result<Json<ProtectionResponse>, (StatusCode, String)> {
    let mut p = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    match body.password {
        None => {
            p.protect_secret = None;
        }
        Some(pw) if pw.is_empty() => {
            p.protect_secret = None;
        }
        Some(pw) => {
            p.protect_secret = Some(hash_password(&pw));
        }
    }
    p.updated_at = Utc::now();
    state
        .update_project(&p)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ProtectionResponse {
        password_protect: p.protect_secret.is_some(),
    }))
}

async fn delete_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .delete_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let repo = state.data_dir.join("repos").join(&id);
    let _ = tokio::fs::remove_dir_all(repo).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn trigger_deploy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<DeployRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Deployment>, (StatusCode, String)> {
    let mut project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let repo_path = state.data_dir.join("repos").join(&id);
    clone_or_fetch(&project.github_url, &repo_path, &project.default_branch)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let deploy_req = body.map(|Json(b)| b).unwrap_or_default();
    let requested_sha = deploy_req.commit_sha.filter(|s| !s.is_empty());
    let sha = if let Some(s) = requested_sha {
        s
    } else {
        remote_head(&repo_path, &project.default_branch)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let msg = crate::services::git::commit_message(&repo_path, &sha)
        .await
        .ok();
    let author = crate::services::git::commit_author(&repo_path, &sha)
        .await
        .ok();

    let changed = if let Some(prev) = &project.last_commit_sha {
        if prev.as_str() != sha.as_str() {
            crate::services::git::changed_files(&repo_path, prev, &sha)
                .await
                .ok()
                .map(|f| f.join("\n"))
        } else {
            None
        }
    } else {
        None
    };

    let skip_msg = should_skip_build(
        &project.root_directory,
        project.ignore_patterns.as_deref(),
        changed.as_deref(),
    );
    let now = Utc::now();
    let dep_id = new_id();
    let dep = Deployment {
        id: dep_id.clone(),
        project_id: id,
        commit_sha: sha.clone(),
        commit_message: msg,
        commit_author: author,
        branch: project.default_branch.clone(),
        status: if skip_msg.is_some() {
            "skipped".into()
        } else {
            "queued".into()
        },
        framework: project.framework.clone(),
        url_path: None,
        error_message: skip_msg.clone(),
        changed_files: changed,
        created_at: now,
        finished_at: if skip_msg.is_some() { Some(now) } else { None },
    };
    state
        .insert_deployment(&dep)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if skip_msg.is_some() {
        // Advance last_commit_sha so the poller does not re-queue the same commit.
        project.last_commit_sha = Some(sha);
        project.updated_at = Utc::now();
        let _ = state.update_project(&project).await;
        let _ = state
            .append_log(&dep.id, dep.error_message.as_deref().unwrap_or("Skipped"))
            .await;
    } else {
        crate::services::webhooks::dispatch_event(
            state.clone(),
            project.id.clone(),
            "deployment.queued",
            &dep,
        );
        state.enqueue_build(dep_id);
    }
    Ok(Json(dep))
}

async fn promote_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PromoteRequest>,
) -> Result<Json<Project>, (StatusCode, String)> {
    let mut project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let dep = state
        .get_deployment(&body.deployment_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))?;

    if dep.project_id != id {
        return Err((
            StatusCode::BAD_REQUEST,
            "deployment does not belong to this project".into(),
        ));
    }
    if dep.status != "ready" {
        return Err((
            StatusCode::BAD_REQUEST,
            "only ready deployments can be promoted".into(),
        ));
    }

    project.production_deployment_id = Some(dep.id);
    project.updated_at = Utc::now();
    state
        .update_project(&project)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(project))
}

async fn list_project_commits(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<CommitsQuery>,
) -> Result<Json<CommitsResponse>, (StatusCode, String)> {
    let project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let repo_path = state.data_dir.join("repos").join(&id);
    if !repo_path.join(".git").exists() {
        clone_or_fetch(&project.github_url, &repo_path, &project.default_branch)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    }

    let limit = q.limit.unwrap_or(20);
    let commits = list_commits(&repo_path, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .map(|c| CommitEntry {
            sha: c.sha,
            message: c.message,
            author: c.author,
            date: c.date,
        })
        .collect();

    Ok(Json(CommitsResponse { commits }))
}

async fn project_activity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ActivityResponse>, (StatusCode, String)> {
    let _project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let deployments = state
        .list_deployments(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let activity = deployments
        .into_iter()
        .map(|d| ActivityEntry {
            id: d.id,
            status: d.status,
            commit_sha: d.commit_sha,
            commit_message: d.commit_message,
            created_at: d.created_at,
            url_path: d.url_path,
        })
        .collect();

    Ok(Json(ActivityResponse { activity }))
}

async fn rollback_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<RollbackRequest>>,
) -> Result<Json<Project>, (StatusCode, String)> {
    let body = body.map(|j| j.0).unwrap_or_default();
    let mut project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let target_id = if let Some(did) = body.deployment_id {
        did
    } else {
        let deps = state
            .list_deployments(&id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let ready: Vec<_> = deps.into_iter().filter(|d| d.status == "ready").collect();
        if let Some(prod) = &project.production_deployment_id {
            ready
                .iter()
                .find(|d| &d.id != prod)
                .map(|d| d.id.clone())
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    "no previous ready deployment to roll back to".into(),
                ))?
        } else if ready.len() >= 2 {
            ready[1].id.clone()
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                "need at least two ready deployments to roll back".into(),
            ));
        }
    };

    let dep = state
        .get_deployment(&target_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))?;
    if dep.project_id != id {
        return Err((
            StatusCode::BAD_REQUEST,
            "deployment does not belong to this project".into(),
        ));
    }
    if dep.status != "ready" {
        return Err((
            StatusCode::BAD_REQUEST,
            "only ready deployments can be rolled back to".into(),
        ));
    }

    project.production_deployment_id = Some(dep.id);
    project.updated_at = Utc::now();
    state
        .update_project(&project)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(project))
}

async fn project_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectStatsResponse>, (StatusCode, String)> {
    let _ = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let rows = state
        .list_project_hits(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let total_hits = rows.iter().map(|r| r.1).sum();
    let deployments = rows
        .into_iter()
        .map(|(deployment_id, hits, last_hit)| DeploymentHitRow {
            deployment_id,
            hits,
            last_hit,
        })
        .collect();

    Ok(Json(ProjectStatsResponse {
        project_id: id,
        total_hits,
        deployments,
    }))
}
