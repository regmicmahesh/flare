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
}
