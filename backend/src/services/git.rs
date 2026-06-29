use anyhow::{bail, Context, Result};
use std::path::Path;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct GithubRef {
    pub owner: String,
    pub repo: String,
    pub html_url: String,
    pub clone_url: String,
}

/// Parse `https://github.com/owner/repo`, `github.com/owner/repo`, or `owner/repo`.
/// No API keys — public HTTPS clone only.
pub fn parse_github_input(input: &str) -> Result<GithubRef> {
    let s = input.trim().trim_end_matches('/').trim_end_matches(".git");
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        bail!("expected owner/repo or https://github.com/owner/repo");
    }
    let owner = parts[0].to_string();
    let repo = parts[1].to_string();
    if owner.contains(':') || repo.contains(':') {
        bail!("invalid owner/repo");
    }
    let html_url = format!("https://github.com/{owner}/{repo}");
    let clone_url = format!("{html_url}.git");
    Ok(GithubRef {
        owner,
        repo,
        html_url,
        clone_url,
    })
}

pub async fn clone_or_fetch(url_or_html: &str, dest: &Path, branch: &str) -> Result<()> {
    let clone_url = if url_or_html.ends_with(".git") {
        url_or_html.to_string()
    } else {
        format!("{}.git", url_or_html.trim_end_matches('/'))
    };

    if dest.join(".git").exists() {
        let status = Command::new("git")
            .args(["-C"])
            .arg(dest)
            .args(["fetch", "origin", branch])
            .status()
            .await
            .context("git fetch")?;
        if !status.success() {
            bail!("git fetch failed");
        }
        let status = Command::new("git")
            .args(["-C"])
            .arg(dest)
            .args(["checkout", branch])
            .status()
            .await
            .context("git checkout")?;
        if !status.success() {
            // try origin/branch
            let status = Command::new("git")
                .args(["-C"])
                .arg(dest)
                .args(["checkout", "-B", branch, &format!("origin/{branch}")])
                .status()
                .await
                .context("git checkout origin/branch")?;
            if !status.success() {
                bail!("git checkout failed");
            }
        }
        let _ = Command::new("git")
            .args(["-C"])
            .arg(dest)
            .args(["reset", "--hard", &format!("origin/{branch}")])
            .status()
            .await;
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let status = Command::new("git")
        .args(["clone", "--depth", "50", "--branch", branch, &clone_url])
        .arg(dest)
        .status()
        .await
        .context("git clone")?;
    if !status.success() {
        // retry without branch (default branch)
        let status = Command::new("git")
            .args(["clone", "--depth", "50", &clone_url])
            .arg(dest)
            .status()
            .await
            .context("git clone default")?;
        if !status.success() {
            bail!("git clone failed for {clone_url} (public repos only, no credentials)");
        }
    }
    Ok(())
}

pub async fn remote_head(repo: &Path, branch: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", &format!("origin/{branch}")])
        .output()
        .await
        .context("rev-parse origin/branch")?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .context("rev-parse HEAD")?;
    if !out.status.success() {
        bail!("cannot resolve HEAD");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn commit_message(repo: &Path, sha: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["log", "-1", "--format=%s", sha])
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn commit_author(repo: &Path, sha: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["log", "-1", "--format=%an", sha])
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn changed_files(repo: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["diff", "--name-only", &format!("{from}..{to}")])
        .output()
        .await?;
    if !out.status.success() {
        bail!("git diff failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect())
}

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub date: String,
}

/// List recent commits from the local clone via `git log` (no GitHub API).
pub async fn list_commits(repo: &Path, limit: usize) -> Result<Vec<CommitInfo>> {
    let n = limit.clamp(1, 100).to_string();
    // %x1f = unit separator between fields; %x1e = record separator between commits
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args([
            "log",
            &format!("-n{n}"),
            "--format=%H%x1f%s%x1f%an%x1f%aI%x1e",
        ])
        .output()
        .await
        .context("git log")?;
    if !out.status.success() {
        bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut commits = Vec::new();
    for record in text.split('\u{1e}') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let parts: Vec<&str> = record.split('\u{1f}').collect();
        if parts.len() < 4 {
            continue;
        }
        commits.push(CommitInfo {
            sha: parts[0].trim().to_string(),
            message: parts[1].trim().to_string(),
            author: parts[2].trim().to_string(),
            date: parts[3].trim().to_string(),
        });
    }
    Ok(commits)
}

/// Returns a skip message when `changed_files` is present and none of the paths
/// fall under `root_directory`. When root is `.` (whole repo), never skip.
pub fn should_skip_build(root_directory: &str, changed_files: Option<&str>) -> Option<String> {
    let files_str = changed_files?;
    let files: Vec<&str> = files_str
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if files.is_empty() {
        return None;
    }
    let root = root_directory.trim().trim_end_matches('/');
    if root.is_empty() || root == "." {
        return None;
    }
    let prefix = format!("{root}/");
    let any_relevant = files.iter().any(|f| *f == root || f.starts_with(&prefix));
    if any_relevant {
        None
    } else {
        Some(format!("Skipped: no changes under root_directory '{root}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_owner_repo() {
        let r = parse_github_input("vercel/next.js").unwrap();
        assert_eq!(r.owner, "vercel");
        assert_eq!(r.repo, "next.js");
        assert!(r.clone_url.ends_with(".git"));
    }

    #[test]
    fn parse_url() {
        let r = parse_github_input("https://github.com/rust-lang/mdBook/").unwrap();
        assert_eq!(r.owner, "rust-lang");
        assert_eq!(r.repo, "mdBook");
    }

    #[test]
    fn skip_when_no_files_under_root() {
        let msg = should_skip_build(
            "apps/web",
            Some("README.md\ndocs/guide.md\napps/api/main.rs"),
        );
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("apps/web"));
    }

    #[test]
    fn no_skip_when_file_under_root() {
        assert!(should_skip_build("apps/web", Some("apps/web/src/index.js\nREADME.md")).is_none());
        assert!(should_skip_build("apps/web", Some("apps/web")).is_none());
    }

    #[test]
    fn no_skip_for_repo_root() {
        assert!(should_skip_build(".", Some("README.md")).is_none());
        assert!(should_skip_build("", Some("README.md")).is_none());
    }

    #[test]
    fn no_skip_without_changed_files() {
        assert!(should_skip_build("apps/web", None).is_none());
        assert!(should_skip_build("apps/web", Some("")).is_none());
    }
}
