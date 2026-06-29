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
    new_id, ActivityEntry, ActivityResponse, CommitEntry, CommitsResponse, CreateProjectRequest,
    DeployRequest, Deployment, Project, ProjectListResponse, UpdateProjectRequest,
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
        github_url: parsed.html_url.clone(),
        owner_repo: format!("{}/{}", parsed.owner, parsed.repo),
        default_branch: branch.clone(),
        framework: fw.framework.clone(),
        root_directory: root,
        build_command: body.build_command.or(fw.build_command.clone()),
        output_directory: body.output_directory.or(fw.output_directory.clone()),
        install_command: body.install_command.or(fw.install_command.clone()),
        last_commit_sha: None,
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
