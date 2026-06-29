use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;

use crate::db::AppState;
use crate::models::{new_id, Deployment};
use crate::services::git::{
    changed_files, clone_or_fetch, commit_author, commit_message, remote_head, should_skip_build,
};

/// Poll public GitHub remotes for new commits (no webhooks / OAuth required).
pub fn start_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(
            std::env::var("FLARE_POLL_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60),
        );
        loop {
            if let Err(e) = tick(state.clone()).await {
                tracing::warn!("poller tick error: {e:#}");
            }
            tokio::time::sleep(interval).await;
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
        if project.last_commit_sha.as_deref() == Some(sha.as_str()) {
            continue;
        }
        tracing::info!(
            "new commit on {} : {} -> {}",
            project.owner_repo,
            project.last_commit_sha.as_deref().unwrap_or("-"),
            &sha[..7.min(sha.len())]
        );

        let changed = if let Some(prev) = &project.last_commit_sha {
            changed_files(&repo, prev, &sha)
                .await
                .ok()
                .map(|f| f.join("\n"))
        } else {
            None
        };

        let dep_id = new_id();
        let skip_msg = should_skip_build(&project.root_directory, changed.as_deref());
        let now = Utc::now();
        let mut dep = Deployment {
            id: dep_id.clone(),
            project_id: project.id.clone(),
            commit_sha: sha.clone(),
            commit_message: commit_message(&repo, "HEAD").await.ok(),
            commit_author: commit_author(&repo, "HEAD").await.ok(),
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
        if let Ok(m) = commit_message(&repo, &dep.commit_sha).await {
            dep.commit_message = Some(m);
        }
        if let Ok(a) = commit_author(&repo, &dep.commit_sha).await {
            dep.commit_author = Some(a);
        }
        state.insert_deployment(&dep).await?;

        if skip_msg.is_some() {
            let mut project = project;
            project.last_commit_sha = Some(sha);
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
            state.enqueue_build(dep_id);
        }
    }
    Ok(())
}
