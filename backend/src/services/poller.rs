use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;

use crate::db::AppState;
use crate::models::{new_id, Deployment, Project};
use crate::services::git::{
    changed_files, clone_or_fetch, commit_author, commit_message, remote_head, should_skip_build,
};

/// Poll public GitHub remotes for new commits (no webhooks / OAuth required).
/// Interval is read from the `settings` table (`poll_interval_secs`) each loop.
/// Also forces redeploy when `redeploy_interval_mins` elapses without new commits.
pub fn start_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(state.clone()).await {
                tracing::warn!("poller tick error: {e:#}");
            }
            let secs = state.poll_interval_secs().await;
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    });
}

async fn tick(state: Arc<AppState>) -> anyhow::Result<()> {
    let projects = state.list_pollable_projects().await?;
    for project in projects {
        let repo = state.data_dir.join("repos").join(&project.id);
        if let Err(e) = clone_or_fetch(&project.github_url, &repo, &project.default_branch).await {
            tracing::debug!("poll fetch {} failed: {e}", project.owner_repo);
            continue;
        }
        let Ok(sha) = remote_head(&repo, &project.default_branch).await else {
            continue;
        };

        let same_commit = project.last_commit_sha.as_deref() == Some(sha.as_str());
        if same_commit {
            if !should_scheduled_redeploy(&state, &project).await? {
                continue;
            }
            tracing::info!(
                "scheduled redeploy for {} (interval {}m)",
                project.owner_repo,
                project.redeploy_interval_mins
            );
            queue_redeploy(state.clone(), &project, &repo, &sha, true).await?;
            continue;
        }

        tracing::info!(
            "new commit on {} : {} -> {}",
            project.owner_repo,
            project.last_commit_sha.as_deref().unwrap_or("-"),
            &sha[..7.min(sha.len())]
        );

        queue_redeploy(state.clone(), &project, &repo, &sha, false).await?;
    }
    Ok(())
}

/// True when project has redeploy_interval_mins > 0 and enough time has passed
/// since the latest deployment was created.
async fn should_scheduled_redeploy(state: &AppState, project: &Project) -> anyhow::Result<bool> {
    let mins = project.redeploy_interval_mins;
    if mins <= 0 {
        return Ok(false);
    }
    let deps = state.list_deployments(&project.id).await?;
    let Some(latest) = deps.first() else {
        return Ok(false);
    };
    let elapsed = Utc::now().signed_duration_since(latest.created_at);
    Ok(elapsed.num_minutes() >= mins)
}

async fn queue_redeploy(
    state: Arc<AppState>,
    project: &Project,
    repo: &std::path::Path,
    sha: &str,
    force: bool,
) -> anyhow::Result<()> {
    let changed = if force {
        None
    } else if let Some(prev) = &project.last_commit_sha {
        changed_files(repo, prev, sha)
            .await
            .ok()
            .map(|f| f.join("\n"))
    } else {
        None
    };

    let dep_id = new_id();
    let skip_msg = if force {
        None
    } else {
        should_skip_build(
            &project.root_directory,
            project.ignore_patterns.as_deref(),
            changed.as_deref(),
        )
    };
    let now = Utc::now();
    let mut dep = Deployment {
        id: dep_id.clone(),
        project_id: project.id.clone(),
        commit_sha: sha.to_string(),
        commit_message: if force {
            Some(format!(
                "Scheduled redeploy (every {}m)",
                project.redeploy_interval_mins
            ))
        } else {
            commit_message(repo, "HEAD").await.ok()
        },
        commit_author: commit_author(repo, "HEAD").await.ok(),
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
    if !force {
        if let Ok(m) = commit_message(repo, &dep.commit_sha).await {
            dep.commit_message = Some(m);
        }
        if let Ok(a) = commit_author(repo, &dep.commit_sha).await {
            dep.commit_author = Some(a);
        }
    }
    state.insert_deployment(&dep).await?;

    if skip_msg.is_some() {
        let mut project = project.clone();
        project.last_commit_sha = Some(sha.to_string());
        project.updated_at = Utc::now();
        state.update_project(&project).await?;
        let _ = state
            .append_log(&dep.id, dep.error_message.as_deref().unwrap_or("Skipped"))
            .await;
        tracing::info!(
            "skipped build for {} (no changes under root_directory)",
            project.owner_repo
        );
    } else {
        crate::services::webhooks::dispatch_event(
            state.clone(),
            project.id.clone(),
            "deployment.queued",
            &dep,
        );
        state.enqueue_build(dep_id);
    }
    Ok(())
}
