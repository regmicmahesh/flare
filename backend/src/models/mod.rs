use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub github_url: String,
    pub owner_repo: String,
    pub default_branch: String,
    pub framework: Option<String>,
    pub root_directory: String,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
    pub last_commit_sha: Option<String>,
    pub production_deployment_id: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct PromoteRequest {
    pub deployment_id: String,
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

/// Derive a URL-safe slug from a project name.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "project".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool App"), "my-cool-app");
        assert_eq!(slugify("  hello!!world  "), "hello-world");
        assert_eq!(slugify("---"), "project");
        assert_eq!(slugify("API_v2"), "api-v2");
    }
}
