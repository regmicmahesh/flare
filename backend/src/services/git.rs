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

/// Match a single gitignore-style glob against a repo-relative path.
/// Patterns without `/` also match basenames in any directory (e.g. `*.md` → `docs/a.md`).
pub fn path_matches_glob(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim().trim_start_matches("./");
    let path = path.trim().trim_start_matches("./");
    if pattern.is_empty() || path.is_empty() {
        return false;
    }
    if glob_match(pattern, path) {
        return true;
    }
    // Bare patterns (no slash) also match against the file name only / any depth.
    if !pattern.contains('/') {
        if let Some(name) = path.rsplit('/').next() {
            if glob_match(pattern, name) {
                return true;
            }
        }
        // `**/pattern` style for nested paths
        let deep = format!("**/{pattern}");
        return glob_match(&deep, path);
    }
    false
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = path.chars().collect();
    glob_match_rec(&p, 0, &s, 0)
}

fn glob_match_rec(pat: &[char], pi: usize, s: &[char], si: usize) -> bool {
    let mut pi = pi;
    let mut si = si;
    while pi < pat.len() {
        if pat[pi] == '*' {
            // `**` — match across `/`
            if pi + 1 < pat.len() && pat[pi + 1] == '*' {
                let mut npi = pi + 2;
                if npi < pat.len() && pat[npi] == '/' {
                    npi += 1;
                }
                // Try matching rest at every position (including current)
                let mut i = si;
                loop {
                    if glob_match_rec(pat, npi, s, i) {
                        return true;
                    }
                    if i >= s.len() {
                        break;
                    }
                    i += 1;
                }
                return false;
            }
            // `*` — match within one path segment (no `/`)
            let npi = pi + 1;
            let mut i = si;
            loop {
                if glob_match_rec(pat, npi, s, i) {
                    return true;
                }
                if i >= s.len() || s[i] == '/' {
                    break;
                }
                i += 1;
            }
            return false;
        }
        if pat[pi] == '?' {
            if si >= s.len() || s[si] == '/' {
                return false;
            }
            pi += 1;
            si += 1;
            continue;
        }
        if si >= s.len() || pat[pi] != s[si] {
            return false;
        }
        pi += 1;
        si += 1;
    }
    si == s.len()
}

/// Parse newline-separated ignore patterns (blank lines and `#` comments ignored).
pub fn parse_ignore_patterns(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect()
}

/// Returns a skip message when `changed_files` is present and either:
/// - none of the paths fall under `root_directory` (when root is not `.`), or
/// - every relevant changed file matches at least one `ignore_patterns` glob.
pub fn should_skip_build(
    root_directory: &str,
    ignore_patterns: Option<&str>,
    changed_files: Option<&str>,
) -> Option<String> {
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
    let relevant: Vec<&str> = if root.is_empty() || root == "." {
        files.clone()
    } else {
        let prefix = format!("{root}/");
        files
            .iter()
            .copied()
            .filter(|f| *f == root || f.starts_with(&prefix))
            .collect()
    };
    if relevant.is_empty() {
        return Some(format!("Skipped: no changes under root_directory '{root}'"));
    }
    let patterns = parse_ignore_patterns(ignore_patterns);
    if !patterns.is_empty() {
        let all_ignored = relevant
            .iter()
            .all(|f| patterns.iter().any(|p| path_matches_glob(p, f)));
        if all_ignored {
            return Some("Skipped: all changed files match ignore_patterns".into());
        }
    }
    None
}

/// True when `path` (repo-relative) did not change between `from` and `to`.
pub async fn path_unchanged_between(repo: &Path, from: &str, to: &str, path: &str) -> bool {
    if from == to {
        return true;
    }
    let out = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["diff", "--name-only", &format!("{from}..{to}"), "--", path])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .all(|l| l.trim().is_empty()),
        _ => false,
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
            None,
            Some("README.md\ndocs/guide.md\napps/api/main.rs"),
        );
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("apps/web"));
    }

    #[test]
    fn no_skip_when_file_under_root() {
        assert!(
            should_skip_build("apps/web", None, Some("apps/web/src/index.js\nREADME.md")).is_none()
        );
        assert!(should_skip_build("apps/web", None, Some("apps/web")).is_none());
    }

    #[test]
    fn no_skip_for_repo_root() {
        assert!(should_skip_build(".", None, Some("README.md")).is_none());
        assert!(should_skip_build("", None, Some("README.md")).is_none());
    }

    #[test]
    fn no_skip_without_changed_files() {
        assert!(should_skip_build("apps/web", None, None).is_none());
        assert!(should_skip_build("apps/web", None, Some("")).is_none());
    }

    #[test]
    fn skip_when_all_match_ignore_patterns() {
        let msg = should_skip_build(".", Some("*.md\ndocs/**"), Some("README.md\ndocs/guide.md"));
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("ignore_patterns"));
    }

    #[test]
    fn no_skip_when_some_not_ignored() {
        assert!(should_skip_build(".", Some("*.md"), Some("README.md\nsrc/index.js")).is_none());
    }

    #[test]
    fn ignore_with_root_directory() {
        // Only docs under apps/web — all ignored
        let msg = should_skip_build(
            "apps/web",
            Some("**/*.md"),
            Some("apps/web/README.md\napps/api/main.rs"),
        );
        assert!(msg.is_some());
        // Real source change under root — do not skip
        assert!(should_skip_build(
            "apps/web",
            Some("*.md"),
            Some("apps/web/src/app.js\napps/web/README.md"),
        )
        .is_none());
    }

    #[test]
    fn glob_basics() {
        assert!(path_matches_glob("*.md", "README.md"));
        assert!(path_matches_glob("*.md", "docs/guide.md"));
        assert!(path_matches_glob("docs/**", "docs/guide.md"));
        assert!(path_matches_glob("docs/**", "docs/a/b.txt"));
        assert!(!path_matches_glob("docs/**", "src/a.txt"));
        assert!(path_matches_glob("**/*.md", "apps/web/README.md"));
    }
}
