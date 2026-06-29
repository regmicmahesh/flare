use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub github_url: String,
    pub owner_repo: String,
    pub default_branch: String,
    pub framework: Option<String>,
    pub root_directory: String,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
    pub last_commit_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub poll_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deployment {
    pub id: String,
    pub project_id: String,
    pub commit_sha: String,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub branch: String,
    pub status: String,
    pub framework: Option<String>,
    pub url_path: Option<String>,
    pub error_message: Option<String>,
    pub changed_files: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BuildLog {
    pub id: i64,
    pub deployment_id: String,
    pub line: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EnvVar {
    pub id: String,
    pub project_id: String,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    /// Public GitHub URL or `owner/repo`
    pub github: String,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub root_directory: Option<String>,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub default_branch: Option<String>,
    pub root_directory: Option<String>,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
    pub poll_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertEnvRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct DeployRequest {
    /// Optional commit SHA to deploy; defaults to branch HEAD.
    pub commit_sha: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CommitEntry {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub date: String,
}

#[derive(Debug, Serialize)]
pub struct CommitsResponse {
    pub commits: Vec<CommitEntry>,
}

#[derive(Debug, Serialize)]
pub struct ActivityEntry {
    pub id: String,
    pub status: String,
    pub commit_sha: String,
    pub commit_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub url_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActivityResponse {
    pub activity: Vec<ActivityEntry>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ProjectListResponse {
    pub projects: Vec<Project>,
}

#[derive(Debug, Serialize)]
pub struct DeploymentListResponse {
    pub deployments: Vec<Deployment>,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub logs: Vec<BuildLog>,
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}
