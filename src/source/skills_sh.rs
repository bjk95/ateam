//! skills.sh registry — fallback path resolver for skills that don't appear
//! in a repo's current tree but still exist in the public registry's blob
//! storage (e.g., a skill that was renamed or moved upstream but is still
//! published under its old slug). Mirrors what Vercel's `npx skills` CLI
//! does: when local discovery misses, hit
//! `https://skills.sh/api/download/<owner>/<repo>/<slug>` and use the
//! cached snapshot content.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Component, Path};
use std::time::Duration;

const BASE_URL: &str = "https://skills.sh";
const FETCH_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Deserialize)]
pub struct DownloadResponse {
    pub files: Vec<DownloadFile>,
    #[serde(default)]
    pub hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DownloadFile {
    pub path: String,
    pub contents: String,
}

impl DownloadFile {
    pub fn relative_path(&self) -> Result<&Path> {
        validate_download_path(&self.path)?;
        Ok(Path::new(&self.path))
    }
}

fn validate_download_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("download file path is empty");
    }
    if has_windows_prefix(path) {
        bail!("download file path `{}` must be relative", path);
    }
    let mut has_normal_component = false;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir => {}
            Component::ParentDir => bail!("download file path `{}` contains `..`", path),
            Component::RootDir | Component::Prefix(_) => {
                bail!("download file path `{}` must be relative", path)
            }
        }
    }
    if !has_normal_component {
        bail!("download file path `{}` has no file name", path);
    }
    Ok(())
}

fn has_windows_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || path.starts_with("\\\\")
}

/// Vercel-style slug: lowercase, runs of whitespace/underscore become hyphens,
/// non-alphanumeric stripped, leading/trailing hyphens trimmed.
pub fn to_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        let mapped = match ch {
            ' ' | '_' | '-' => Some('-'),
            c if c.is_ascii_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match mapped {
            Some('-') if !prev_dash && !out.is_empty() => {
                out.push('-');
                prev_dash = true;
            }
            Some('-') => {}
            Some(c) => {
                out.push(c);
                prev_dash = false;
            }
            None => {}
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Look up a skill by `<owner>/<repo>/<slug>` in the skills.sh registry's
/// blob-download endpoint. Returns `Ok(None)` when the registry has no entry
/// (404), `Err` only on network/parse failures.
pub fn fetch(owner: &str, repo: &str, slug: &str) -> Result<Option<DownloadResponse>> {
    let url = format!(
        "{}/api/download/{}/{}/{}",
        BASE_URL,
        urlencode(owner),
        urlencode(repo),
        urlencode(slug),
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .user_agent("agents-cli")
        .build()
        .context("building http client")?;
    let response = client.get(&url).send().context("calling skills.sh")?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    if !response.status().is_success() {
        anyhow::bail!("skills.sh returned {}", response.status());
    }
    let body = response.text().context("reading skills.sh response")?;
    // skills.sh sometimes returns Next.js HTML for unknown routes — make sure
    // we only accept JSON-shaped responses.
    if body.starts_with('<') {
        return Ok(None);
    }
    let parsed: DownloadResponse = serde_json::from_str(&body)
        .with_context(|| format!("parsing skills.sh response from {}", url))?;
    Ok(Some(parsed))
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_matches_vercel_rules() {
        assert_eq!(to_slug("Azure Observability"), "azure-observability");
        assert_eq!(to_slug("frontend-design"), "frontend-design");
        assert_eq!(to_slug("My_Skill"), "my-skill");
        assert_eq!(to_slug("--leading-trailing--"), "leading-trailing");
    }

    #[test]
    fn urlencode_handles_special_chars() {
        assert_eq!(urlencode("foo bar"), "foo%20bar");
        assert_eq!(urlencode("plain"), "plain");
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }

    #[test]
    fn relative_path_rejects_escape_paths() {
        for path in [
            "",
            "../SKILL.md",
            "skills/../../SKILL.md",
            "/tmp/SKILL.md",
            "C:\\SKILL.md",
            "\\\\server\\share\\SKILL.md",
        ] {
            let file = DownloadFile {
                path: path.into(),
                contents: String::new(),
            };
            assert!(
                file.relative_path().is_err(),
                "expected `{}` to be rejected",
                path
            );
        }
    }

    #[test]
    fn relative_path_accepts_nested_files() {
        let file = DownloadFile {
            path: "assets/icon.png".into(),
            contents: String::new(),
        };

        assert_eq!(file.relative_path().unwrap(), Path::new("assets/icon.png"));
    }
}
