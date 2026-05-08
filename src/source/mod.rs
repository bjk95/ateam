pub mod git;
pub mod github;
pub mod local;
pub mod skills_sh;

use anyhow::{anyhow, bail, Result};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// A skill-package source. Parsed from user input and stored verbatim
/// (sans path) in the lockfile's `source` field. The `path` field on
/// the lockfile entry handles the optional sub-path within the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Github { owner: String, repo: String },
    Git { url: String },
    Local { path: PathBuf },
}

impl Source {
    /// Parse a Vercel-compatible source spec. Strict: rejects `openclaw/*`
    /// sources unless the caller routes through [`Source::parse_with`].
    pub fn parse(input: &str) -> Result<Self> {
        Self::parse_with(input, false)
    }

    /// Parse a Vercel-compatible source spec.
    ///
    /// Accepts:
    /// - `owner/repo` — GitHub shorthand
    /// - `github:owner/repo` — explicit prefix
    /// - `https://github.com/owner/repo` — full GitHub URL (treated as github)
    /// - `git@github.com:owner/repo.git` — SSH GitHub URL (treated as github)
    /// - `https://...` / `git@...` for non-github → `git:` source
    /// - `./path` or `/abs/path` → `local:`
    ///
    /// `accept_openclaw` mirrors Vercel's `--dangerously-accept-openclaw-risks`
    /// gate: openclaw skills can shell out, so the user must explicitly accept.
    pub fn parse_with(input: &str, accept_openclaw: bool) -> Result<Self> {
        let s = input.trim();
        if s.is_empty() {
            bail!("empty source");
        }

        let parsed = parse_inner(s)?;
        if !accept_openclaw && parsed.is_openclaw() {
            bail!("refusing openclaw/* source `{}` — pass --dangerously-accept-openclaw-risks to install anyway", input);
        }
        Ok(parsed)
    }

    fn is_openclaw(&self) -> bool {
        match self {
            Source::Github { owner, .. } => owner.eq_ignore_ascii_case("openclaw"),
            Source::Git { url } => {
                url.contains("github.com/openclaw/") || url.contains("github.com:openclaw/")
            }
            Source::Local { .. } => false,
        }
    }

    /// Lockfile string form, *without* a path component.
    pub fn lockfile_string(&self) -> String {
        match self {
            Source::Github { owner, repo } => format!("github:{}/{}", owner, repo),
            Source::Git { url } => format!("git:{}", url),
            Source::Local { path } => format!("local:{}", path.display()),
        }
    }

    /// Parse the lockfile-string form back into a Source. Lockfile entries
    /// were already gated at add-time, so the openclaw block is bypassed here.
    pub fn from_lockfile_string(s: &str) -> Result<Self> {
        Self::parse_with(s, true)
    }
}

fn parse_inner(s: &str) -> Result<Source> {
    // Explicit prefix wins.
    if let Some(rest) = s.strip_prefix("github:") {
        return parse_github_owner_repo(rest);
    }
    if let Some(rest) = s.strip_prefix("git:") {
        return Ok(Source::Git {
            url: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("local:") {
        return Ok(Source::Local {
            path: PathBuf::from(rest),
        });
    }

    // Local path detection.
    if s.starts_with("./") || s.starts_with("../") || s.starts_with('/') || s.starts_with("~/") {
        return Ok(Source::Local {
            path: PathBuf::from(s),
        });
    }

    // GitHub HTTPS / SSH detection — normalize to (owner, repo).
    if let Some((owner, repo)) = strip_github_url(s) {
        return Ok(Source::Github { owner, repo });
    }

    // Bare git URL → generic git.
    if s.starts_with("https://")
        || s.starts_with("http://")
        || s.starts_with("ssh://")
        || s.starts_with("git@")
        || s.ends_with(".git")
    {
        return Ok(Source::Git { url: s.to_string() });
    }

    // Last resort: `owner/repo` shorthand.
    parse_github_owner_repo(s)
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lockfile_string())
    }
}

/// Reject any subpath that could escape its package root. Used at parse time
/// (lockfile load) so a malicious `path = "../../etc"` entry can never reach
/// the extraction step where `pkg_root.join(sub_path)` would traverse outside
/// the tarball working dir.
pub fn sanitize_subpath(s: &str) -> Result<()> {
    if s.is_empty() {
        bail!("subpath is empty");
    }
    for comp in Path::new(s).components() {
        match comp {
            Component::ParentDir => bail!("subpath `{}` contains `..` traversal", s),
            Component::RootDir | Component::Prefix(_) => {
                bail!("subpath `{}` must be relative, not absolute", s)
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

fn parse_github_owner_repo(s: &str) -> Result<Source> {
    let trimmed = s.trim_end_matches('/').trim_start_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() < 2 {
        return Err(anyhow!("expected `owner/repo`, got `{}`", s));
    }
    let owner = parts[0].to_string();
    let repo = parts[1].trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return Err(anyhow!("invalid owner/repo in `{}`", s));
    }
    Ok(Source::Github { owner, repo })
}

fn strip_github_url(s: &str) -> Option<(String, String)> {
    // https://github.com/owner/repo[.git][/...]
    if let Some(rest) = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))
    {
        let cleaned = rest
            .split('#')
            .next()
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("");
        let parts: Vec<&str> = cleaned.split('/').collect();
        if parts.len() >= 2 {
            let owner = parts[0].to_string();
            let repo = parts[1].trim_end_matches(".git").to_string();
            if !owner.is_empty() && !repo.is_empty() {
                return Some((owner, repo));
            }
        }
        return None;
    }
    // git@github.com:owner/repo[.git]
    if let Some(rest) = s.strip_prefix("git@github.com:") {
        let cleaned = rest.trim_end_matches(".git");
        let parts: Vec<&str> = cleaned.split('/').collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_repo_shorthand() {
        assert_eq!(
            Source::parse("vercel-labs/agent-skills").unwrap(),
            Source::Github {
                owner: "vercel-labs".into(),
                repo: "agent-skills".into()
            }
        );
    }

    #[test]
    fn parses_explicit_github_prefix() {
        assert_eq!(
            Source::parse("github:foo/bar").unwrap(),
            Source::Github {
                owner: "foo".into(),
                repo: "bar".into()
            }
        );
    }

    #[test]
    fn parses_https_github_url() {
        assert_eq!(
            Source::parse("https://github.com/foo/bar").unwrap(),
            Source::Github {
                owner: "foo".into(),
                repo: "bar".into()
            }
        );
        assert_eq!(
            Source::parse("https://github.com/foo/bar.git").unwrap(),
            Source::Github {
                owner: "foo".into(),
                repo: "bar".into()
            }
        );
    }

    #[test]
    fn parses_local_path() {
        match Source::parse("./skills/foo").unwrap() {
            Source::Local { path } => assert_eq!(path, PathBuf::from("./skills/foo")),
            other => panic!("expected Local, got {:?}", other),
        }
    }

    #[test]
    fn parses_generic_git() {
        match Source::parse("https://gitlab.com/x/y.git").unwrap() {
            Source::Git { url } => assert_eq!(url, "https://gitlab.com/x/y.git"),
            other => panic!("expected Git, got {:?}", other),
        }
    }

    #[test]
    fn rejects_openclaw_owner_shorthand() {
        let err = Source::parse("openclaw/exfil").unwrap_err();
        assert!(err
            .to_string()
            .contains("--dangerously-accept-openclaw-risks"));
    }

    #[test]
    fn rejects_openclaw_https_url() {
        assert!(Source::parse("https://github.com/openclaw/x").is_err());
    }

    #[test]
    fn accepts_openclaw_when_gated() {
        let s = Source::parse_with("openclaw/x", true).unwrap();
        assert_eq!(
            s,
            Source::Github {
                owner: "openclaw".into(),
                repo: "x".into()
            }
        );
    }

    #[test]
    fn lockfile_load_bypasses_openclaw_block() {
        // Lockfile entries were blessed at add-time; reload must not fail.
        let s = Source::from_lockfile_string("github:openclaw/x").unwrap();
        assert_eq!(
            s,
            Source::Github {
                owner: "openclaw".into(),
                repo: "x".into()
            }
        );
    }

    #[test]
    fn sanitize_subpath_accepts_normal_segments() {
        assert!(sanitize_subpath("skills/foo").is_ok());
        assert!(sanitize_subpath("a/b/c").is_ok());
        assert!(sanitize_subpath("./skills/foo").is_ok());
    }

    #[test]
    fn sanitize_subpath_rejects_parent_dir_traversal() {
        assert!(sanitize_subpath("..").is_err());
        assert!(sanitize_subpath("../etc").is_err());
        assert!(sanitize_subpath("skills/../../etc").is_err());
        assert!(sanitize_subpath("a/b/../../..").is_err());
    }

    #[test]
    fn sanitize_subpath_rejects_absolute_paths() {
        assert!(sanitize_subpath("/etc/passwd").is_err());
        assert!(sanitize_subpath("/abs/path").is_err());
    }

    #[test]
    fn sanitize_subpath_rejects_empty() {
        assert!(sanitize_subpath("").is_err());
    }
}
