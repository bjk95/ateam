use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const USER_AGENT: &str = concat!("agents/", env!("CARGO_PKG_VERSION"));
const FALLBACK_BRANCH: &str = "main";

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
pub fn subtree_sha(
    owner: &str,
    repo: &str,
    commit_sha: &str,
    sub_path: &str,
) -> Result<Option<String>> {
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
            format!("Bearer {}", token)
                .parse()
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
    let bytes = resp
        .bytes()
        .with_context(|| format!("downloading {}", url))?;
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
    let resp = req.send().with_context(|| format!("requesting {}", url))?;
    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!(
            "GET {} returned {}: {}",
            url,
            status,
            truncate(&body_text, 200)
        );
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

/// Resolve the repo's default branch via the GitHub API.
///
/// Result is cached per `(owner, repo)` for the lifetime of the process so
/// repeated callers within a single command don't burn API budget. On API
/// failure, emits a loud warning and falls back to `"main"`.
pub fn default_branch(owner: &str, repo: &str) -> String {
    default_branch_with(
        owner,
        repo,
        default_branch_cache(),
        &fetch_default_branch_via_api,
    )
}

fn default_branch_cache() -> &'static Mutex<HashMap<(String, String), String>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fetch_default_branch_via_api(owner: &str, repo: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let resp = github_get_json(&url).ok()?;
    resp.get("default_branch")
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn default_branch_with(
    owner: &str,
    repo: &str,
    cache: &Mutex<HashMap<(String, String), String>>,
    fetch: &dyn Fn(&str, &str) -> Option<String>,
) -> String {
    let key = (owner.to_string(), repo.to_string());
    {
        let guard = cache.lock().expect("default_branch cache poisoned");
        if let Some(v) = guard.get(&key) {
            return v.clone();
        }
    }
    match fetch(owner, repo) {
        Some(branch) => {
            cache
                .lock()
                .expect("default_branch cache poisoned")
                .insert(key, branch.clone());
            branch
        }
        None => {
            crate::ui::warn(format!(
                "GitHub API: could not resolve default branch for {}/{} — falling back to `{}`. Pass --ref to pin a branch explicitly.",
                owner, repo, FALLBACK_BRANCH
            ));
            FALLBACK_BRANCH.to_string()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn default_branch_caches_per_command() {
        let cache = Mutex::new(HashMap::new());
        let calls = Cell::new(0usize);
        let fetch = |_: &str, _: &str| -> Option<String> {
            calls.set(calls.get() + 1);
            Some("trunk".to_string())
        };
        let b1 = default_branch_with("acme", "widget", &cache, &fetch);
        let b2 = default_branch_with("acme", "widget", &cache, &fetch);
        assert_eq!(b1, "trunk");
        assert_eq!(b2, "trunk");
        assert_eq!(calls.get(), 1, "second call should hit the cache");
    }

    #[test]
    fn default_branch_falls_back_on_api_failure() {
        let cache = Mutex::new(HashMap::new());
        let fetch = |_: &str, _: &str| -> Option<String> { None };
        let b = default_branch_with("acme", "widget", &cache, &fetch);
        assert_eq!(b, FALLBACK_BRANCH);
    }
}
