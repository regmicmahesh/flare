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
    new_id, slugify, ActivityEntry, ActivityResponse, CommitEntry, CommitsResponse,
    CreateProjectRequest, DeployRequest, Deployment, DeploymentStatsResponse, Project,
    ProjectListResponse, ProjectStatsResponse, PromoteRequest, RollbackRequest,
    UpdateProjectRequest,
};
use crate::services::framework::detect_framework;
use crate::services::git::{
    clone_or_fetch, list_commits, parse_github_input, remote_head, should_skip_build,
};

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/{id}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route("/api/projects/{id}/deploy", post(trigger_deploy))
        .route("/api/projects/{id}/promote", post(promote_deployment))
        .route("/api/projects/{id}/rollback", post(rollback_deployment))
        .route("/api/projects/{id}/stats", get(project_stats))
        .route("/api/projects/{id}/commits", get(list_project_commits))
        .route("/api/projects/{id}/activity", get(project_activity))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct CommitsQuery {
    limit: Option<usize>,
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProjectListResponse>, (StatusCode, String)> {
    let projects = state
        .list_projects()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
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
    if let Some(pe) = body.poll_enabled {
        p.poll_enabled = pe;
    }
    p.updated_at = Utc::now();

    state
        .update_project(&p)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(p))
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

    let skip_msg = should_skip_build(&project.root_directory, changed.as_deref());
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
        state.enqueue_build(dep_id);
    }
    Ok(Json(dep))
}

/// Pin a ready deployment as production (served at /p/{id} and /s/{slug}).
async fn promote_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PromoteRequest>,
) -> Result<Json<Project>, (StatusCode, String)> {
    set_production(&state, &id, &body.deployment_id).await
}

/// Instant rollback: promote previous ready deployment, or an explicit deployment_id.
async fn rollback_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Result<Json<RollbackRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Project>, (StatusCode, String)> {
    let req = body.map(|Json(b)| b).unwrap_or_default();

    let target_id = if let Some(dep_id) = req.deployment_id.filter(|s| !s.is_empty()) {
        dep_id
    } else {
        let project = state
            .get_project(&id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

        let ready = state
            .list_ready_deployments(&id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if ready.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "no ready deployments to roll back to".into(),
            ));
        }

        // Prefer the ready deployment immediately older than current production;
        // if production is unset or missing, use the second-latest ready (or latest if only one?).
        let current = project.production_deployment_id.as_deref();
        let target = if let Some(cur) = current {
            ready.iter().find(|d| d.id != cur).or_else(|| ready.first())
        } else if ready.len() >= 2 {
            // No pin: treat latest as "current" and roll back to previous.
            ready.get(1)
        } else {
            ready.first()
        };

        target.map(|d| d.id.clone()).ok_or((
            StatusCode::BAD_REQUEST,
            "no previous ready deployment".into(),
        ))?
    };

    set_production(&state, &id, &target_id).await
}

async fn set_production(
    state: &AppState,
    project_id: &str,
    deployment_id: &str,
) -> Result<Json<Project>, (StatusCode, String)> {
    let mut project = state
        .get_project(project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let dep = state
        .get_deployment(deployment_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))?;

    if dep.project_id != project_id {
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

async fn project_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectStatsResponse>, (StatusCode, String)> {
    let _project = state
        .get_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;

    let rows = state
        .list_hits_for_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let deployments: Vec<DeploymentStatsResponse> = rows
        .into_iter()
        .map(|h| DeploymentStatsResponse {
            deployment_id: h.deployment_id,
            hits: h.hits,
            last_hit: Some(h.last_hit),
        })
        .collect();
    let hits: i64 = deployments.iter().map(|d| d.hits).sum();

    Ok(Json(ProjectStatsResponse {
        project_id: id,
        hits,
        deployments,
    }))
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
