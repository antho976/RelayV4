//! Explicit GitHub integration. Relay delegates authentication to `gh`, so credentials remain
//! in the user's system credential store instead of Relay's SQLite store.

use relay_bus::error::BusError;
use relay_bus::types::{GitHubRepo, GitHubStatus};
use serde::Deserialize;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct DownloadedSkill {
    pub name: String,
    pub body: String,
    pub source_url: String,
    pub source_path: String,
    pub revision: String,
    /// The skill's whole folder, staged outside the clone: reference documents and scripts a
    /// `SKILL.md` points at are part of the skill, not decoration (D147). `None` when the
    /// caller asked for no staging area, or the folder was past the copy bounds.
    pub assets: Option<PathBuf>,
}

#[derive(Deserialize)]
struct RepoJson {
    name: String,
    full_name: String,
    description: Option<String>,
    clone_url: String,
    ssh_url: String,
    private: bool,
    archived: bool,
    updated_at: Option<String>,
}

pub fn gh_path() -> Result<PathBuf, BusError> {
    which::which("gh").map_err(|_| BusError::unavailable("github.gh_missing", "GitHub CLI is not installed or not on PATH")
        .with_hint("install GitHub CLI, then reopen GitHub setup"))
}

pub fn status() -> GitHubStatus {
    let Ok(gh) = which::which("gh") else {
        return GitHubStatus { installed: false, connected: false, login: None };
    };
    let output = Command::new(gh).args(["api", "user", "--jq", ".login"]).output();
    match output {
        Ok(output) if output.status.success() => GitHubStatus {
            installed: true,
            connected: true,
            login: Some(String::from_utf8_lossy(&output.stdout).trim().to_string()).filter(|value| !value.is_empty()),
        },
        _ => GitHubStatus { installed: true, connected: false, login: None },
    }
}

pub fn connect(gh: PathBuf) -> std::io::Result<bool> {
    Command::new(gh)
        .args(["auth", "login", "--hostname", "github.com", "--web", "--clipboard", "--git-protocol", "https"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
}

pub fn repositories() -> Result<Vec<GitHubRepo>, BusError> {
    let gh = gh_path()?;
    let output = Command::new(gh)
        .args(["api", "--paginate", "user/repos?per_page=100&sort=updated&direction=desc"])
        .output()
        .map_err(|error| BusError::unavailable("github.list_failed", error.to_string()))?;
    if !output.status.success() {
        return Err(BusError::unavailable(
            "github.not_connected",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ).with_hint("connect GitHub first"));
    }
    let mut repositories = Vec::new();
    let stream = serde_json::Deserializer::from_slice(&output.stdout).into_iter::<Vec<RepoJson>>();
    for page in stream {
        for repo in page.map_err(|error| BusError::internal(format!("decoding GitHub repositories: {error}")))? {
            repositories.push(GitHubRepo {
                name: repo.name,
                full_name: repo.full_name,
                description: repo.description,
                clone_url: repo.clone_url,
                ssh_url: repo.ssh_url,
                private: repo.private,
                archived: repo.archived,
                updated_at: repo.updated_at,
            });
        }
    }
    repositories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.full_name.cmp(&b.full_name)));
    Ok(repositories)
}

pub fn download_skills(
    url: &str,
    requested_subdir: Option<&str>,
    stage: Option<&Path>,
) -> Result<Vec<DownloadedSkill>, BusError> {
    let (source_url, branch, url_subdir) = parse_github_url(url)?;
    let subdir = requested_subdir.filter(|value| !value.trim().is_empty()).map(str::trim).or(url_subdir.as_deref());
    let temp = std::env::temp_dir().join(format!("relay-skill-{}", uuid::Uuid::new_v4()));
    let mut command = Command::new("git");
    command.args(["clone", "--depth", "1"]);
    if let Some(branch) = branch.as_deref() { command.args(["--branch", branch]); }
    let output = command.arg(&source_url).arg(&temp).output()
        .map_err(|error| BusError::unavailable("skill.git_missing", format!("cannot start git: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = fs::remove_dir_all(&temp);
        return Err(BusError::unavailable("skill.clone_failed", message));
    }
    let result = collect_skills(&temp, &source_url, subdir, stage);
    let _ = fs::remove_dir_all(&temp);
    result
}

fn parse_github_url(value: &str) -> Result<(String, Option<String>, Option<String>), BusError> {
    let value = value.trim().trim_end_matches('/');
    let path = if let Some(path) = value.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = value.strip_prefix("http://github.com/") {
        path
    } else if let Some(path) = value.strip_prefix("git@github.com:") {
        path
    } else {
        return Err(BusError::invalid("skill.github_url", "enter a github.com repository or SKILL.md URL"));
    };
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(BusError::invalid("skill.github_url", "GitHub URL must include owner and repository"));
    }
    let repo = parts[1].trim_end_matches(".git");
    let source = format!("https://github.com/{}/{}.git", parts[0], repo);
    if parts.get(2) == Some(&"tree") || parts.get(2) == Some(&"blob") {
        let branch = parts.get(3).map(|value| value.to_string());
        let mut subdir = parts.get(4..).unwrap_or_default().join("/");
        if subdir.ends_with("/SKILL.md") { subdir.truncate(subdir.len() - "/SKILL.md".len()); }
        else if subdir == "SKILL.md" { subdir.clear(); }
        Ok((source, branch, (!subdir.is_empty()).then_some(subdir)))
    } else {
        Ok((source, None, None))
    }
}

fn collect_skills(
    root: &Path,
    source_url: &str,
    subdir: Option<&str>,
    stage: Option<&Path>,
) -> Result<Vec<DownloadedSkill>, BusError> {
    let revision = Command::new("git").args(["-C", root.to_string_lossy().as_ref(), "rev-parse", "HEAD"])
        .output().ok().filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string()).unwrap_or_default();
    let start = if let Some(subdir) = subdir {
        let relative = safe_relative(subdir)?;
        root.join(relative)
    } else { root.to_path_buf() };
    if !start.exists() {
        return Err(BusError::not_found("skill.path_not_found", format!("{} does not exist in the repository", start.strip_prefix(root).unwrap_or(&start).display())));
    }
    let mut files = Vec::new();
    find_skill_files(&start, 0, &mut files).map_err(|error| BusError::unavailable("skill.read_failed", error.to_string()))?;
    if files.is_empty() {
        return Err(BusError::not_found("skill.none_found", "no SKILL.md files were found at that GitHub source"));
    }
    files.sort();
    files.into_iter().enumerate().map(|(index, path)| {
        let body = fs::read_to_string(&path).map_err(|error| BusError::unavailable("skill.read_failed", error.to_string()))?;
        if body.len() > 256 * 1024 { return Err(BusError::invalid("skill.body", format!("{} exceeds 256 KiB", path.display()))); }
        let source_path = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let fallback = path.parent().and_then(Path::file_name).map(|value| value.to_string_lossy().to_string()).unwrap_or_else(|| "skill".into());
        let name = frontmatter_name(&body).unwrap_or(fallback);
        let assets = stage.zip(path.parent()).and_then(|(stage, folder)| {
            let staged = stage.join(index.to_string());
            // An oversized folder is not a failed install: the instructions still work, only
            // the extra files are dropped.
            match crate::skills::copy_bounded(folder, &staged) {
                Ok(()) => Some(staged),
                Err(error) => {
                    tracing::warn!(folder = %folder.display(), error = %error, "staging skill folder");
                    let _ = fs::remove_dir_all(&staged);
                    None
                }
            }
        });
        Ok(DownloadedSkill { name, body, source_url: source_url.to_string(), source_path, revision: revision.clone(), assets })
    }).collect()
}

fn safe_relative(value: &str) -> Result<PathBuf, BusError> {
    let path = Path::new(value);
    if path.is_absolute() || path.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(BusError::invalid("skill.subdir", "skill subdirectory must be a simple relative path"));
    }
    Ok(path.to_path_buf())
}

fn find_skill_files(path: &Path, depth: usize, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        if path.file_name().is_some_and(|name| name == "SKILL.md") { out.push(path.to_path_buf()); }
        return Ok(());
    }
    if depth > 8 { return Ok(()); }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || entry.file_name() == ".git" { continue; }
        if file_type.is_dir() { find_skill_files(&entry.path(), depth + 1, out)?; }
        else if entry.file_name() == "SKILL.md" { out.push(entry.path()); }
    }
    Ok(())
}

fn frontmatter_name(body: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" { return None; }
    for line in lines {
        if line.trim() == "---" { break; }
        if let Some(name) = line.trim().strip_prefix("name:") {
            let name = name.trim().trim_matches(['\'', '"']);
            if !name.is_empty() { return Some(name.chars().take(120).collect()); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_urls_and_skill_files_are_resolved() {
        let (url, branch, subdir) = parse_github_url("https://github.com/openai/skills/tree/main/skills/docs/SKILL.md").unwrap();
        assert_eq!(url, "https://github.com/openai/skills.git");
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(subdir.as_deref(), Some("skills/docs"));

        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("skills/review");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("SKILL.md"), "---\nname: Review carefully\n---\nCheck the diff.\n").unwrap();
        fs::create_dir_all(folder.join("reference")).unwrap();
        fs::write(folder.join("reference/audit.md"), "audit").unwrap();
        let stage = tempfile::tempdir().unwrap();
        let skills = collect_skills(root.path(), "https://github.com/example/skills.git", Some("skills/review"), Some(stage.path())).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Review carefully");
        assert_eq!(skills[0].source_path, "skills/review/SKILL.md");
        // The folder travels with the instructions, not just the SKILL.md the row stores.
        let assets = skills[0].assets.as_deref().expect("skill folder staged");
        assert_eq!(fs::read_to_string(assets.join("reference/audit.md")).unwrap(), "audit");
        assert!(assets.join("SKILL.md").is_file());
    }
}
