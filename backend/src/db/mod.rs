use anyhow::Context;
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::sync::mpsc;

use crate::models::{BuildLog, Deployment, Domain, EnvVar, Project, Webhook};

pub struct AppState {
    pub pool: SqlitePool,
    pub data_dir: PathBuf,
    pub build_tx: mpsc::UnboundedSender<String>,
    pub build_rx: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<String>>>,
}

impl AppState {
    pub async fn new(db_path: &Path, data_dir: PathBuf) -> anyhow::Result<Self> {
        let options =
            SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=rwc", db_path.display()))?
                .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("connect sqlite")?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                github_url TEXT NOT NULL,
                owner_repo TEXT NOT NULL,
                default_branch TEXT NOT NULL DEFAULT 'main',
                framework TEXT,
                root_directory TEXT NOT NULL DEFAULT '.',
                build_command TEXT,
                output_directory TEXT,
                install_command TEXT,
                last_commit_sha TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                poll_enabled INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS deployments (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                commit_sha TEXT NOT NULL,
                commit_message TEXT,
                commit_author TEXT,
                branch TEXT NOT NULL,
                status TEXT NOT NULL,
                framework TEXT,
                url_path TEXT,
                error_message TEXT,
                changed_files TEXT,
                created_at TEXT NOT NULL,
                finished_at TEXT,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS build_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                deployment_id TEXT NOT NULL,
                line TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(deployment_id) REFERENCES deployments(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS env_vars (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(project_id, key),
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS webhooks (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                url TEXT NOT NULL,
                events TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS domains (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                host TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_deployments_project ON deployments(project_id);
            CREATE INDEX IF NOT EXISTS idx_logs_deployment ON build_logs(deployment_id);
            CREATE INDEX IF NOT EXISTS idx_webhooks_project ON webhooks(project_id);
            CREATE INDEX IF NOT EXISTS idx_domains_host ON domains(host);
            "#,
        )
        .execute(&pool)
        .await?;

        // Seed defaults if missing
        sqlx::query(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('poll_interval_secs', '60')",
        )
        .execute(&pool)
        .await?;

        let (build_tx, build_rx) = mpsc::unbounded_channel();

        Ok(Self {
            pool,
            data_dir,
            build_tx,
            build_rx: parking_lot::Mutex::new(Some(build_rx)),
        })
    }

    pub async fn get_all_settings(
        &self,
    ) -> sqlx::Result<std::collections::HashMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM settings ORDER BY key")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn get_setting(&self, key: &str) -> sqlx::Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn poll_interval_secs(&self) -> u64 {
        if let Ok(Some(v)) = self.get_setting("poll_interval_secs").await {
            if let Ok(n) = v.parse::<u64>() {
                return n.max(5);
            }
        }
        std::env::var("FLARE_POLL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60)
            .max(5)
    }

    pub async fn list_projects(&self) -> sqlx::Result<Vec<Project>> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, github_url, owner_repo, default_branch, framework,
                    root_directory, build_command, output_directory, install_command,
                    last_commit_sha, created_at, updated_at,
                    poll_enabled != 0 as poll_enabled
             FROM projects ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_project(&self, id: &str) -> sqlx::Result<Option<Project>> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, github_url, owner_repo, default_branch, framework,
                    root_directory, build_command, output_directory, install_command,
                    last_commit_sha, created_at, updated_at,
                    poll_enabled != 0 as poll_enabled
             FROM projects WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn insert_project(&self, p: &Project) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO projects (id, name, github_url, owner_repo, default_branch, framework,
             root_directory, build_command, output_directory, install_command, last_commit_sha,
             created_at, updated_at, poll_enabled)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(&p.github_url)
        .bind(&p.owner_repo)
        .bind(&p.default_branch)
        .bind(&p.framework)
        .bind(&p.root_directory)
        .bind(&p.build_command)
        .bind(&p.output_directory)
        .bind(&p.install_command)
        .bind(&p.last_commit_sha)
        .bind(p.created_at)
        .bind(p.updated_at)
        .bind(p.poll_enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_project(&self, p: &Project) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE projects SET name=?, default_branch=?, framework=?, root_directory=?,
             build_command=?, output_directory=?, install_command=?, last_commit_sha=?,
             updated_at=?, poll_enabled=? WHERE id=?",
        )
        .bind(&p.name)
        .bind(&p.default_branch)
        .bind(&p.framework)
        .bind(&p.root_directory)
        .bind(&p.build_command)
        .bind(&p.output_directory)
        .bind(&p.install_command)
        .bind(&p.last_commit_sha)
        .bind(p.updated_at)
        .bind(p.poll_enabled)
        .bind(&p.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_project(&self, id: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_deployment(&self, d: &Deployment) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO deployments (id, project_id, commit_sha, commit_message, commit_author,
             branch, status, framework, url_path, error_message, changed_files, created_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&d.id)
        .bind(&d.project_id)
        .bind(&d.commit_sha)
        .bind(&d.commit_message)
        .bind(&d.commit_author)
        .bind(&d.branch)
        .bind(&d.status)
        .bind(&d.framework)
        .bind(&d.url_path)
        .bind(&d.error_message)
        .bind(&d.changed_files)
        .bind(d.created_at)
        .bind(d.finished_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_deployment(&self, d: &Deployment) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE deployments SET status=?, framework=?, url_path=?, error_message=?,
             changed_files=?, finished_at=? WHERE id=?",
        )
        .bind(&d.status)
        .bind(&d.framework)
        .bind(&d.url_path)
        .bind(&d.error_message)
        .bind(&d.changed_files)
        .bind(d.finished_at)
        .bind(&d.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_deployment(&self, id: &str) -> sqlx::Result<Option<Deployment>> {
        sqlx::query_as::<_, Deployment>("SELECT * FROM deployments WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_deployments(&self, project_id: &str) -> sqlx::Result<Vec<Deployment>> {
        sqlx::query_as::<_, Deployment>(
            "SELECT * FROM deployments WHERE project_id = ? ORDER BY created_at DESC LIMIT 50",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn append_log(&self, deployment_id: &str, line: &str) -> sqlx::Result<()> {
        sqlx::query("INSERT INTO build_logs (deployment_id, line, created_at) VALUES (?, ?, ?)")
            .bind(deployment_id)
            .bind(line)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_logs(&self, deployment_id: &str) -> sqlx::Result<Vec<BuildLog>> {
        sqlx::query_as::<_, BuildLog>(
            "SELECT id, deployment_id, line, created_at FROM build_logs
             WHERE deployment_id = ? ORDER BY id ASC",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_env(&self, project_id: &str) -> sqlx::Result<Vec<EnvVar>> {
        sqlx::query_as::<_, EnvVar>(
            "SELECT id, project_id, key, value, created_at FROM env_vars WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn upsert_env(&self, v: &EnvVar) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO env_vars (id, project_id, key, value, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(project_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(&v.id)
        .bind(&v.project_id)
        .bind(&v.key)
        .bind(&v.value)
        .bind(v.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_env(&self, project_id: &str, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM env_vars WHERE project_id = ? AND key = ?")
            .bind(project_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_pollable_projects(&self) -> sqlx::Result<Vec<Project>> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, github_url, owner_repo, default_branch, framework,
                    root_directory, build_command, output_directory, install_command,
                    last_commit_sha, created_at, updated_at,
                    poll_enabled != 0 as poll_enabled
             FROM projects WHERE poll_enabled != 0",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub fn enqueue_build(&self, deployment_id: String) {
        let _ = self.build_tx.send(deployment_id);
    }

    // --- webhooks ---

    pub async fn list_webhooks(&self, project_id: &str) -> sqlx::Result<Vec<Webhook>> {
        sqlx::query_as::<_, Webhook>(
            "SELECT id, project_id, url, events, created_at FROM webhooks
             WHERE project_id = ? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn insert_webhook(&self, w: &Webhook) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO webhooks (id, project_id, url, events, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&w.id)
        .bind(&w.project_id)
        .bind(&w.url)
        .bind(&w.events)
        .bind(w.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_webhook(&self, project_id: &str, webhook_id: &str) -> sqlx::Result<u64> {
        let res = sqlx::query("DELETE FROM webhooks WHERE id = ? AND project_id = ?")
            .bind(webhook_id)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    // --- domains ---

    pub async fn list_domains(&self, project_id: &str) -> sqlx::Result<Vec<Domain>> {
        sqlx::query_as::<_, Domain>(
            "SELECT id, project_id, host, created_at FROM domains
             WHERE project_id = ? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn insert_domain(&self, d: &Domain) -> sqlx::Result<()> {
        sqlx::query("INSERT INTO domains (id, project_id, host, created_at) VALUES (?, ?, ?, ?)")
            .bind(&d.id)
            .bind(&d.project_id)
            .bind(&d.host)
            .bind(d.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_domain_by_host(&self, host: &str) -> sqlx::Result<Option<Domain>> {
        sqlx::query_as::<_, Domain>(
            "SELECT id, project_id, host, created_at FROM domains WHERE host = ?",
        )
        .bind(host)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn delete_domain(&self, project_id: &str, domain_id: &str) -> sqlx::Result<u64> {
        let res = sqlx::query("DELETE FROM domains WHERE id = ? AND project_id = ?")
            .bind(domain_id)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// Latest ready deployment for a project (production mapping for custom domains).
    pub async fn latest_ready_deployment(
        &self,
        project_id: &str,
    ) -> sqlx::Result<Option<Deployment>> {
        sqlx::query_as::<_, Deployment>(
            "SELECT * FROM deployments WHERE project_id = ? AND status = 'ready'
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
    }
}
