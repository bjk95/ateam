use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const USER_AGENT: &str = concat!("ateam/", env!("CARGO_PKG_VERSION"));

/// Resolve a branch/tag/sha into a definite commit SHA.
///
/// Calls `GET /repos/{owner}/{repo}/commits/{ref}` and returns the `sha` field.
pub fn resolve_ref(owner: &str, repo: &str, git_ref: &str) -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/commits/{}",
        owner, repo, git_ref
    );
    let resp = github_get_json(&url)?;
    let sha = resp
        .get("sha")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("response missing `sha` field for {}", url))?;
    Ok(sha.to_string())
}

/// Get the recursive tree at the given commit SHA. Returns the GitHub API
/// response object — caller walks `tree[]` to find subtree SHAs.
pub fn get_tree(owner: &str, repo: &str, commit_sha: &str) -> Result<serde_json::Value> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
        owner, repo, commit_sha
    );
    github_get_json(&url)
}

/// Find the tree SHA of a sub-path within a repo at a given ref.
/// Returns `None` if the path doesn't exist in the tree.
pub fn subtree_sha(owner: &str, repo: &str, commit_sha: &str, sub_path: &str) -> Result<Option<String>> {
    let tree = get_tree(owner, repo, commit_sha)?;
    let entries = tree
        .get("tree")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("tree response missing `tree` array"))?;
    let normalized_target = sub_path.trim_matches('/');
    for entry in entries {
        let path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let kind = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if path == normalized_target && kind == "tree" {
            if let Some(sha) = entry.get("sha").and_then(|v| v.as_str()) {
                return Ok(Some(sha.to_string()));
            }
        }
    }
    Ok(None)
}

/// Download the repo tarball at a given ref and extract into a temp dir.
/// Returns the path to the extracted root directory (one level inside the
/// tarball, since GitHub tarballs are wrapped in `{owner}-{repo}-{shortsha}/`).
pub fn fetch_tarball(owner: &str, repo: &str, git_ref: &str, dest: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    let url = format!(
        "https://api.github.com/repos/{}/{}/tarball/{}",
        owner, repo, git_ref
    );
    let mut client_builder = reqwest::blocking::Client::builder().user_agent(USER_AGENT);
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token).parse()
                .map_err(|_| anyhow!("invalid GITHUB_TOKEN value"))?,
        );
        client_builder = client_builder.default_headers(headers);
    }
    let client = client_builder.build().context("building reqwest client")?;
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("requesting {}", url))?;
    if !resp.status().is_success() {
        bail!("GET {} returned {}", url, resp.status());
    }
    let bytes = resp.bytes().with_context(|| format!("downloading {}", url))?;
    let cursor = std::io::Cursor::new(bytes);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dest)
        .with_context(|| format!("extracting tarball to {}", dest.display()))?;

    // Find the single root dir GitHub created.
    let mut iter = fs::read_dir(dest).with_context(|| format!("reading {}", dest.display()))?;
    let entry = iter
        .next()
        .ok_or_else(|| anyhow!("tarball produced no top-level dir"))??;
    Ok(entry.path())
}

fn github_get_json(url: &str) -> Result<serde_json::Value> {
    let mut req = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("building reqwest client")?
        .get(url);
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .with_context(|| format!("requesting {}", url))?;
    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("GET {} returned {}: {}", url, status, truncate(&body_text, 200));
    }
    serde_json::from_str(&body_text).with_context(|| format!("parsing JSON from {}", url))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("{}...", &s[..n])
    } else {
        s.to_string()
    }
}

/// Convenience: assume `default_ref` if caller doesn't pin one.
pub fn default_branch_or(repo_owner: &str, repo: &str, fallback: &str) -> String {
    let url = format!("https://api.github.com/repos/{}/{}", repo_owner, repo);
    if let Ok(resp) = github_get_json(&url) {
        if let Some(default) = resp.get("default_branch").and_then(|v| v.as_str()) {
            return default.to_string();
        }
    }
    fallback.to_string()
}

/// Drop in fallback to "main" without an extra API call. Used during
/// initial install to avoid spending API budget on the metadata call.
pub fn default_branch_fallback() -> &'static str {
    "main"
}

/// Read a single file from a repo at a given ref. Used for spot reads.
#[allow(dead_code)]
pub fn read_file_at_ref(owner: &str, repo: &str, git_ref: &str, path: &str) -> Result<String> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        owner, repo, git_ref, path
    );
    let mut req = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()?
        .get(&url);
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.bearer_auth(token);
    }
    let resp = req.send().with_context(|| format!("requesting {}", url))?;
    if !resp.status().is_success() {
        bail!("GET {} returned {}", url, resp.status());
    }
    let mut text = String::new();
    resp.text()
        .map(|s| {
            text = s;
        })
        .ok();
    Ok(text)
}
