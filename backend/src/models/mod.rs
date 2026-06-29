use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
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
    /// Newline-separated glob patterns (e.g. `*.md`, `docs/**`). Optional.
    pub ignore_patterns: Option<String>,
    /// sha256(password) hex when deployment protection is enabled. Serialized as `password_protect` bool.
    #[serde(
        skip_deserializing,
        rename = "password_protect",
        serialize_with = "serialize_password_protect"
    )]
    pub protect_secret: Option<String>,
    /// Minutes between forced redeploys (0 = off). In addition to commit polling.
    pub redeploy_interval_mins: i64,
    pub last_commit_sha: Option<String>,
    pub production_deployment_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub poll_enabled: bool,
}

fn serialize_password_protect<S>(secret: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_bool(secret.is_some())
}

/// sha256 hex digest of `password` (MVP — not production-grade password storage).
pub fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// Access token derived from stored protect_secret (sha256 of the secret).
pub fn access_token_from_secret(protect_secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(protect_secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Cookie value: `{project_id}:{token}`.
pub fn flare_access_cookie_value(project_id: &str, token: &str) -> String {
    format!("{project_id}:{token}")
}

/// Returns true if the request is authorized for a protected project.
pub fn check_project_access(
    project_id: &str,
    protect_secret: &str,
    authorization: Option<&str>,
    cookie_header: Option<&str>,
) -> bool {
    let expected = access_token_from_secret(protect_secret);
    if let Some(auth) = authorization {
        let bearer = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .map(str::trim)
            .unwrap_or(auth.trim());
        if !bearer.is_empty() && (bearer == expected || hash_password(bearer) == protect_secret) {
            return true;
        }
    }
    if let Some(cookie) = cookie_header {
        let needle = format!("flare_access={}", flare_access_cookie_value(project_id, &expected));
        // Also accept legacy/simple form where token == protect_secret for local demos.
        let needle_secret = format!(
            "flare_access={}",
            flare_access_cookie_value(project_id, protect_secret)
        );
        for part in cookie.split(';') {
            let p = part.trim();
            if p == needle || p == needle_secret {
                return true;
            }
            // Allow any flare_access for this project_id whose token matches.
            if let Some(rest) = p.strip_prefix("flare_access=") {
                if let Some((pid, tok)) = rest.split_once(':') {
                    if pid == project_id && (tok == expected || tok == protect_secret) {
                        return true;
                    }
                }
            }
        }
    }
    false
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

/// Redacted project export (no secrets / env values / protect passwords).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectExport {
    /// Schema version for future compatibility.
    pub version: u32,
    pub name: String,
    pub github_url: String,
    pub owner_repo: String,
    pub default_branch: String,
    pub framework: Option<String>,
    pub root_directory: String,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
    /// Newline-separated ignore globs (no secrets).
    pub ignore_patterns: Option<String>,
    pub poll_enabled: bool,
    pub redeploy_interval_mins: i64,
    /// Whether password protection was enabled (password never exported).
    pub password_protect: bool,
    /// Env **keys only** — values are never included by default.
    pub env_keys: Vec<String>,
    /// Custom domain hosts only.
    pub domain_hosts: Vec<String>,
    /// Webhook URLs and event subscriptions (no signing secrets for MVP).
    pub webhooks: Vec<ProjectExportWebhook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectExportWebhook {
    pub url: String,
    pub events: String,
}

/// Import a project from GitHub plus optional overrides from an export (no secrets).
#[derive(Debug, Deserialize)]
pub struct ImportProjectRequest {
    /// Public GitHub URL or `owner/repo` (required).
    pub github: String,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub root_directory: Option<String>,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
    pub ignore_patterns: Option<String>,
    pub poll_enabled: Option<bool>,
    pub redeploy_interval_mins: Option<u32>,
    /// Domain hosts to register after create (no secrets).
    pub domain_hosts: Option<Vec<String>>,
    /// Webhook URLs (+ optional events) to register after create.
    pub webhooks: Option<Vec<ImportWebhook>>,
    /// Env **keys only** — recorded in response as placeholders; values must be set separately.
    pub env_keys: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ImportWebhook {
    pub url: String,
    pub events: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub default_branch: Option<String>,
    pub root_directory: Option<String>,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub install_command: Option<String>,
    /// Newline-separated glob patterns to ignore for smart skip.
    pub ignore_patterns: Option<String>,
    pub poll_enabled: Option<bool>,
    /// Minutes between forced redeploys (0 = off).
    pub redeploy_interval_mins: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct SetProtectionRequest {
    /// Set password to enable protection; `null` or omit empty to clear.
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProtectionResponse {
    pub password_protect: bool,
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: &'static str,
    pub features: Vec<&'static str>,
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

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub settings: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    /// Poll interval for public GitHub remotes (seconds, min 5).
    pub poll_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: String,
    pub project_id: String,
    pub url: String,
    /// Comma-separated event names (e.g. deployment.ready,deployment.error)
    pub events: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub url: String,
    /// Events to subscribe to. Defaults to all known events if omitted/empty.
    pub events: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct WebhookListResponse {
    pub webhooks: Vec<Webhook>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Domain {
    pub id: String,
    pub project_id: String,
    pub host: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDomainRequest {
    pub host: String,
}

#[derive(Debug, Serialize)]
pub struct DomainListResponse {
    pub domains: Vec<Domain>,
}

/// Known deploy-hook event names.
pub const WEBHOOK_EVENTS: &[&str] = &["deployment.ready", "deployment.error", "deployment.queued"];

#[derive(Debug, Serialize)]
pub struct DeploymentStatsResponse {
    pub deployment_id: String,
    pub hits: i64,
    pub last_hit: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct DeploymentHitRow {
    pub deployment_id: String,
    pub hits: i64,
    pub last_hit: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProjectStatsResponse {
    pub project_id: String,
    pub total_hits: i64,
    pub deployments: Vec<DeploymentHitRow>,
}

#[derive(Debug, Serialize)]
pub struct DeploymentDiffResponse {
    pub a: String,
    pub b: String,
    pub commit_sha_a: String,
    pub commit_sha_b: String,
    pub files: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RollbackRequest {
    /// Optional explicit deployment to roll back to (must be ready).
    pub deployment_id: Option<String>,
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
    use super::{
        access_token_from_secret, check_project_access, hash_password, slugify,
    };

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool App"), "my-cool-app");
        assert_eq!(slugify("  hello!!world  "), "hello-world");
        assert_eq!(slugify("---"), "project");
        assert_eq!(slugify("API_v2"), "api-v2");
    }

    #[test]
    fn password_hash_and_token() {
        let secret = hash_password("s3cret");
        assert_eq!(secret.len(), 64);
        assert_ne!(secret, hash_password("other"));
        let token = access_token_from_secret(&secret);
        assert_eq!(token.len(), 64);
        assert_ne!(token, secret);
    }

    #[test]
    fn access_check_bearer_token_and_password() {
        let pid = "proj-1";
        let secret = hash_password("hunter2");
        let token = access_token_from_secret(&secret);
        assert!(check_project_access(
            pid,
            &secret,
            Some(&format!("Bearer {token}")),
            None
        ));
        assert!(check_project_access(
            pid,
            &secret,
            Some("Bearer hunter2"),
            None
        ));
        assert!(!check_project_access(
            pid,
            &secret,
            Some("Bearer wrong"),
            None
        ));
    }

    #[test]
    fn access_check_cookie() {
        let pid = "proj-1";
        let secret = hash_password("hunter2");
        let token = access_token_from_secret(&secret);
        let cookie = format!("session=abc; flare_access={pid}:{token}; other=1");
        assert!(check_project_access(pid, &secret, None, Some(&cookie)));
        assert!(!check_project_access(
            "other-proj",
            &secret,
            None,
            Some(&cookie)
        ));
    }
}
