use chrono::Utc;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::db::AppState;
use crate::services::framework::detect_framework;

pub struct BuildWorker {
    state: Arc<AppState>,
}

impl BuildWorker {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub fn spawn(self) {
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut rx = state
                .build_rx
                .lock()
                .take()
                .expect("build receiver already taken");
            while let Some(dep_id) = rx.recv().await {
                if let Err(e) = run_build(state.clone(), &dep_id).await {
                    tracing::error!("build {dep_id} failed: {e:#}");
                }
            }
        });
    }
}

async fn log_line(state: &AppState, dep_id: &str, line: &str) {
    tracing::debug!("[{dep_id}] {line}");
    let _ = state.append_log(dep_id, line).await;
}

async fn is_cancelled(state: &AppState, dep_id: &str) -> bool {
    matches!(
        state.get_deployment(dep_id).await,
        Ok(Some(d)) if d.status == "cancelled"
    )
}

async fn run_build(state: Arc<AppState>, dep_id: &str) -> anyhow::Result<()> {
    let mut dep = state
        .get_deployment(dep_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("deployment not found"))?;

    // Prefer marking skipped at enqueue time and not enqueuing; still guard the worker.
    if dep.status == "skipped" {
        log_line(
            &state,
            dep_id,
            dep.error_message
                .as_deref()
                .unwrap_or("Build skipped — no install/build run"),
        )
        .await;
        return Ok(());
    }

    if dep.status == "cancelled" {
        log_line(&state, dep_id, "Build cancelled before start").await;
        return Ok(());
    }

    let mut project = state
        .get_project(&dep.project_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project not found"))?;

    // Secondary check if changed_files became available after enqueue.
    if let Some(msg) = crate::services::git::should_skip_build(
        &project.root_directory,
        dep.changed_files.as_deref(),
    ) {
        dep.status = "skipped".into();
        dep.error_message = Some(msg.clone());
        dep.finished_at = Some(Utc::now());
        state.update_deployment(&dep).await?;
        log_line(&state, dep_id, &msg).await;
        project.last_commit_sha = Some(dep.commit_sha.clone());
        project.updated_at = Utc::now();
        state.update_project(&project).await?;
        return Ok(());
    }

    if is_cancelled(&state, dep_id).await {
        log_line(&state, dep_id, "Build cancelled before start").await;
        return Ok(());
    }

    dep.status = "building".into();
    state.update_deployment(&dep).await?;
    log_line(&state, dep_id, "Starting build…").await;

    let repo = state.data_dir.join("repos").join(&project.id);
    let work = if project.root_directory == "." {
        repo.clone()
    } else {
        repo.join(&project.root_directory)
    };

    // checkout commit
    let _ = Command::new("git")
        .args(["-C", &repo.to_string_lossy(), "checkout", &dep.commit_sha])
        .status()
        .await;

    if is_cancelled(&state, dep_id).await {
        log_line(&state, dep_id, "Build cancelled after checkout").await;
        return Ok(());
    }

    let fw = detect_framework(&work);
    if project.framework.is_none() {
        project.framework = fw.framework.clone();
    }
    dep.framework = fw.framework.clone();

    let install = project
        .install_command
        .clone()
        .or(fw.install_command.clone());
    let build = project.build_command.clone().or(fw.build_command.clone());
    let output = project
        .output_directory
        .clone()
        .or(fw.output_directory.clone())
        .unwrap_or_else(|| "dist".into());

    let env_vars = state.list_env(&project.id).await.unwrap_or_default();

    if let Some(cmd) = install {
        if is_cancelled(&state, dep_id).await {
            log_line(&state, dep_id, "Build cancelled before install").await;
            return Ok(());
        }
        log_line(&state, dep_id, &format!("$ {cmd}")).await;
        if let Err(e) = run_shell(state.clone(), dep_id, &work, &cmd, &env_vars).await {
            fail(state.clone(), &mut dep, &e.to_string()).await?;
            return Ok(());
        }
    }

    if let Some(cmd) = build {
        if is_cancelled(&state, dep_id).await {
            log_line(&state, dep_id, "Build cancelled before build step").await;
            return Ok(());
        }
        log_line(&state, dep_id, &format!("$ {cmd}")).await;
        if let Err(e) = run_shell(state.clone(), dep_id, &work, &cmd, &env_vars).await {
            fail(state.clone(), &mut dep, &e.to_string()).await?;
            return Ok(());
        }
    }

    if is_cancelled(&state, dep_id).await {
        log_line(&state, dep_id, "Build cancelled before publish").await;
        return Ok(());
    }

    let out_src = if output == "." {
        work.clone()
    } else {
        work.join(&output)
    };

    let deploy_dir = state.data_dir.join("deployments").join(dep_id);
    let _ = tokio::fs::remove_dir_all(&deploy_dir).await;
    tokio::fs::create_dir_all(&deploy_dir).await?;

    if out_src.exists() {
        copy_dir(&out_src, &deploy_dir)?;
        log_line(
            &state,
            dep_id,
            &format!("Copied output from {}", out_src.display()),
        )
        .await;
    } else if work.join("index.html").exists() {
        copy_dir(&work, &deploy_dir)?;
        log_line(&state, dep_id, "Copied project root (static)").await;
    } else {
        // write a placeholder success page
        let html = format!(
            r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Flare Deploy</title>
<style>body{{font-family:system-ui;background:#0a0a0f;color:#e8e8f0;display:flex;min-height:100vh;align-items:center;justify-content:center}}
.card{{background:#14141f;padding:2rem 3rem;border-radius:12px;border:1px solid #2a2a3a}}
h1{{margin:0 0 .5rem;background:linear-gradient(90deg,#a78bfa,#22d3ee);-webkit-background-clip:text;color:transparent}}
p{{color:#888;margin:0}}</style></head><body><div class="card"><h1>Deployed with Flare</h1>
<p>Project · {} · {}</p><p style="margin-top:.75rem;font-size:.85rem">No static output detected; this is a placeholder.</p></div></body></html>"#,
            project.name,
            &dep.commit_sha[..7.min(dep.commit_sha.len())]
        );
        tokio::fs::write(deploy_dir.join("index.html"), html).await?;
        log_line(
            &state,
            dep_id,
            "No output dir found — wrote placeholder index.html",
        )
        .await;
    }

    // Final cancel check — don't overwrite cancelled status with ready.
    if is_cancelled(&state, dep_id).await {
        log_line(&state, dep_id, "Build cancelled before marking ready").await;
        return Ok(());
    }

    dep.status = "ready".into();
    dep.url_path = Some(format!("/_deploy/{dep_id}/"));
    dep.finished_at = Some(Utc::now());
    state.update_deployment(&dep).await?;

    project.last_commit_sha = Some(dep.commit_sha.clone());
    project.updated_at = Utc::now();
    project.framework = dep.framework.clone();
    state.update_project(&project).await?;

    log_line(&state, dep_id, "Build ready ✓").await;
    crate::services::webhooks::dispatch_event(
        state.clone(),
        project.id.clone(),
        "deployment.ready",
        &dep,
    );
    Ok(())
}

async fn fail(
    state: Arc<AppState>,
    dep: &mut crate::models::Deployment,
    err: &str,
) -> anyhow::Result<()> {
    // Don't overwrite a cancel with error.
    if is_cancelled(&state, &dep.id).await {
        log_line(&state, &dep.id, "Build cancelled (ignoring failure)").await;
        return Ok(());
    }
    log_line(&state, &dep.id, &format!("ERROR: {err}")).await;
    dep.status = "error".into();
    dep.error_message = Some(err.to_string());
    dep.finished_at = Some(Utc::now());
    state.update_deployment(dep).await?;
    crate::services::webhooks::dispatch_event(
        state.clone(),
        dep.project_id.clone(),
        "deployment.error",
        dep,
    );
    Ok(())
}

async fn run_shell(
    state: Arc<AppState>,
    dep_id: &str,
    cwd: &Path,
    cmd: &str,
    env_vars: &[crate::models::EnvVar],
) -> anyhow::Result<()> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .envs(env_vars.iter().map(|e| (e.key.clone(), e.value.clone())))
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let state_o = state.clone();
    let id_o = dep_id.to_string();
    let t1 = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log_line(&state_o, &id_o, &line).await;
        }
    });
    let state_e = state.clone();
    let id_e = dep_id.to_string();
    let t2 = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log_line(&state_e, &id_e, &line).await;
        }
    });

    let status = child.wait().await?;
    let _ = t1.await;
    let _ = t2.await;
    if !status.success() {
        anyhow::bail!("command failed with {status}");
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(src)?;
        // skip node_modules / .git
        if rel.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s == "node_modules" || s == ".git" || s == "target"
        }) {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &target)?;
        }
    }
    Ok(())
}
