use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;

use crate::db::AppState;
use crate::models::{new_id, Deployment};
use crate::services::git::{
    changed_files, clone_or_fetch, commit_author, commit_message, remote_head,
};

/// Poll public GitHub remotes for new commits (no webhooks / OAuth required).
/// Interval is read from the `settings` table (`poll_interval_secs`) each loop.
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
        let dep = Deployment {
            id: dep_id.clone(),
            project_id: project.id.clone(),
            commit_sha: sha,
            commit_message: commit_message(&repo, "")
                .await
                .ok()
                .or(commit_message(&repo, "HEAD").await.ok()),
            commit_author: commit_author(&repo, "HEAD").await.ok(),
            branch: project.default_branch.clone(),
            status: "queued".into(),
            framework: project.framework.clone(),
            url_path: None,
            error_message: None,
            changed_files: changed,
            created_at: Utc::now(),
            finished_at: None,
        };
        // fix commit message for sha
        let mut dep = dep;
        if let Ok(m) = commit_message(&repo, &dep.commit_sha).await {
            dep.commit_message = Some(m);
        }
        if let Ok(a) = commit_author(&repo, &dep.commit_sha).await {
            dep.commit_author = Some(a);
        }
        state.insert_deployment(&dep).await?;
        state.enqueue_build(dep_id);
    }
    Ok(())
}
