//! Built-in plugins: a bundle of skills, always-on agent instructions, documentation and MCP
//! servers, switched on per project (D159).
//!
//! A plugin is compiled into the engine from the repository's `plugins/<id>/` folder (see
//! `build.rs`), so its content and the code that delivers it never drift apart. Switching one
//! on for a project stores one `plugin_projects` edge; from then on every agent launched in
//! that project gets the plugin's skills as real provider skill folders (through the same
//! materializer as installed skills), the plugin's instructions in its brief, and the plugin's
//! MCP servers in its provider configuration. Nothing is written for a project that has it off.

use anyhow::Result;
use relay_bus::types::{Id, Plugin, PluginDoc, PluginMcpServer, PluginSkill};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

/// One plugin folder as the build script embedded it.
pub struct BundledPlugin {
    pub id: &'static str,
    /// A digest of every file, so a rebuilt engine with changed content re-materializes.
    pub digest: &'static str,
    /// `(path relative to the plugin root, contents)`, sorted by path.
    pub files: &'static [(&'static str, &'static [u8])],
}

include!(concat!(env!("OUT_DIR"), "/bundled_plugins.rs"));

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    /// Markdown every agent of an enabled project receives in its brief.
    pub instructions: Option<String>,
    #[serde(default)]
    pub docs: Vec<String>,
    /// File-name suffixes at a checkout root that mark a project this plugin is for.
    #[serde(default)]
    pub detect: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    /// Where a new agent of an enabled project works when `session.create` names no checkout:
    /// `"primary"` for tools that act on one shared checkout (a running Unreal editor has
    /// exactly one project open). Absent means Relay's default, a new worktree.
    #[serde(default)]
    pub default_checkout: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServer {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `relay` means this Relay binary; anything else is looked up on `PATH` by the provider.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

/// One skill folder inside a plugin.
pub struct SkillEntry {
    /// Folder name, which is also the skill's provider-visible name.
    pub dir: String,
    pub description: String,
    pub body: String,
    /// Files relative to the skill folder.
    pub files: Vec<(&'static str, &'static [u8])>,
}

pub struct Loaded {
    pub bundle: &'static BundledPlugin,
    pub manifest: Manifest,
    pub skills: Vec<SkillEntry>,
}

impl Loaded {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn file(&self, path: &str) -> Option<&'static str> {
        self.bundle
            .files
            .iter()
            .find(|(name, _)| *name == path)
            .and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
    }

    pub fn instructions(&self) -> String {
        self.manifest
            .instructions
            .as_deref()
            .and_then(|path| self.file(path))
            .unwrap_or("")
            .trim()
            .to_string()
    }
}

/// Every bundled plugin, parsed once. A folder with an unreadable manifest is skipped with a
/// warning rather than taking the engine down.
pub fn all() -> &'static [Loaded] {
    static LOADED: OnceLock<Vec<Loaded>> = OnceLock::new();
    LOADED.get_or_init(|| BUNDLED.iter().filter_map(|bundle| match load(bundle) {
        Ok(loaded) => Some(loaded),
        Err(error) => {
            tracing::warn!(plugin = bundle.id, error = %error, "bundled plugin manifest");
            None
        }
    }).collect())
}

pub fn get(id: &str) -> Option<&'static Loaded> {
    all().iter().find(|plugin| plugin.id() == id)
}

fn load(bundle: &'static BundledPlugin) -> Result<Loaded> {
    let raw = bundle
        .files
        .iter()
        .find(|(name, _)| *name == "plugin.json")
        .map(|(_, bytes)| *bytes)
        .ok_or_else(|| anyhow::anyhow!("no plugin.json"))?;
    let manifest: Manifest = serde_json::from_slice(raw)?;
    anyhow::ensure!(manifest.id == bundle.id, "manifest id {:?} is not its folder {:?}", manifest.id, bundle.id);
    let mut skills: BTreeMap<String, SkillEntry> = BTreeMap::new();
    for (path, bytes) in bundle.files {
        let Some(rest) = path.strip_prefix("skills/") else { continue };
        let Some((dir, inner)) = rest.split_once('/') else { continue };
        let entry = skills.entry(dir.to_string()).or_insert_with(|| SkillEntry {
            dir: dir.to_string(),
            description: String::new(),
            body: String::new(),
            files: Vec::new(),
        });
        entry.files.push((inner, bytes));
        if inner == "SKILL.md" {
            entry.body = String::from_utf8_lossy(bytes).into_owned();
            entry.description = frontmatter(&entry.body, "description").unwrap_or_default();
        }
    }
    Ok(Loaded {
        bundle,
        manifest,
        skills: skills.into_values().filter(|skill| !skill.body.is_empty()).collect(),
    })
}

/// One single-line value from a `SKILL.md` YAML front matter block.
pub fn frontmatter(body: &str, key: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim_end();
        if line.trim() == "---" {
            return None;
        }
        if let Some(value) = line.strip_prefix(key).and_then(|rest| rest.strip_prefix(':')) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            return Some(value.to_string());
        }
    }
    None
}

/// Ids of the bundled plugins switched on for a project, in id order.
pub fn enabled_ids(conn: &Connection, project_id: Id) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT plugin_id FROM plugin_projects WHERE project_id=?1 ORDER BY plugin_id",
    )?;
    let ids = stmt.query_map([project_id], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

pub fn enabled_for(conn: &Connection, project_id: Id) -> Result<Vec<&'static Loaded>> {
    Ok(enabled_ids(conn, project_id)?.iter().filter_map(|id| get(id)).collect())
}

/// Plugins on for at least one project — what the machine-wide skill folders carry.
pub fn enabled_anywhere(conn: &Connection) -> Result<Vec<&'static Loaded>> {
    let mut stmt = conn.prepare_cached("SELECT DISTINCT plugin_id FROM plugin_projects ORDER BY plugin_id")?;
    let ids = stmt.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids.iter().filter_map(|id| get(id)).collect())
}

pub fn projects_for(conn: &Connection, plugin_id: &str) -> Result<Vec<Id>> {
    let mut stmt = conn.prepare_cached(
        "SELECT project_id FROM plugin_projects WHERE plugin_id=?1 ORDER BY project_id",
    )?;
    let ids = stmt.query_map([plugin_id], |row| row.get(0))?.collect::<rusqlite::Result<Vec<Id>>>()?;
    Ok(ids)
}

/// The checkout a new session of this project gets when the request names none: `"primary"`
/// when an enabled plugin asks for it (D160), otherwise a new worktree.
pub fn default_checkout(conn: &Connection, project_id: Id) -> Result<&'static str> {
    let primary = enabled_for(conn, project_id)?
        .iter()
        .any(|plugin| plugin.manifest.default_checkout.as_deref() == Some("primary"));
    Ok(if primary { "primary" } else { "new" })
}

/// Whether a checkout root looks like this plugin's kind of project: one directory listing,
/// never a tree walk.
pub fn detect(plugin: &Loaded, root: &Path) -> bool {
    if plugin.manifest.detect.is_empty() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return false };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        plugin.manifest.detect.iter().any(|suffix| name.ends_with(suffix.as_str()))
    })
}

pub fn to_bus(plugin: &Loaded, enabled_in: Vec<Id>, suggested_for: Vec<Id>) -> Plugin {
    let manifest = &plugin.manifest;
    Plugin {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        category: manifest.category.clone(),
        summary: manifest.summary.clone(),
        description: manifest.description.clone(),
        skills: plugin
            .skills
            .iter()
            .map(|skill| PluginSkill {
                name: skill.dir.clone(),
                description: skill.description.clone(),
                files: skill.files.len() as u32,
            })
            .collect(),
        mcp_servers: manifest
            .mcp_servers
            .iter()
            .map(|server| PluginMcpServer {
                name: server.name.clone(),
                description: server.description.clone(),
                tools: server.tools.clone(),
            })
            .collect(),
        docs: manifest.docs.clone(),
        enabled_in,
        suggested_for,
    }
}

pub fn docs(plugin: &Loaded) -> Vec<PluginDoc> {
    plugin
        .manifest
        .docs
        .iter()
        .filter_map(|path| plugin.file(path).map(|body| PluginDoc { path: path.clone(), body: body.to_string() }))
        .collect()
}

/// One plugin MCP server ready for a provider configuration: name, command, args, env.
pub type LaunchServer = (String, String, Vec<String>, BTreeMap<String, String>);

/// The MCP servers every enabled plugin contributes to a project, as `(name, command, args,
/// env)` ready for a provider configuration. `relay` resolves to this Relay binary.
pub fn mcp_servers(
    conn: &Connection,
    project_id: Id,
    relay: &Path,
) -> Result<Vec<LaunchServer>> {
    let mut out: Vec<LaunchServer> = Vec::new();
    for plugin in enabled_for(conn, project_id)? {
        for server in &plugin.manifest.mcp_servers {
            // `relay` is reserved for the bus's own server; a plugin cannot shadow it.
            if server.name == "relay" || out.iter().any(|(name, ..)| name == &server.name) {
                continue;
            }
            let command = if server.command == "relay" {
                relay.display().to_string()
            } else {
                server.command.clone()
            };
            out.push((server.name.clone(), command, server.args.clone(), server.env.clone()));
        }
    }
    Ok(out)
}

/// A server name that is a bare TOML key, so Codex's `--config mcp_servers.<name>.*` reads
/// it as one key, and a single settings path segment.
fn valid_server_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

/// One settings entry as a launch server, or why it cannot be one.
fn custom_server(name: &str, spec: &serde_json::Value, relay: &Path) -> std::result::Result<LaunchServer, String> {
    let object = spec.as_object().ok_or("is not an object")?;
    let command = object.get("command").and_then(|v| v.as_str()).map(str::trim).filter(|c| !c.is_empty())
        .ok_or("has no command")?;
    let args = match object.get("args") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.iter()
            .map(|item| item.as_str().map(str::to_owned).ok_or("has a non-string arg"))
            .collect::<std::result::Result<_, _>>()?,
        Some(_) => return Err("args is not an array".into()),
    };
    let mut env = BTreeMap::new();
    match object.get("env") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Object(pairs)) => for (key, value) in pairs {
            if !valid_server_name(key) { return Err(format!("env key {key:?} is not [A-Za-z0-9_-]")); }
            env.insert(key.clone(), value.as_str().ok_or("has a non-string env value")?.to_owned());
        },
        Some(_) => return Err("env is not an object".into()),
    }
    let command = if command == "relay" { relay.display().to_string() } else { command.to_owned() };
    Ok((name.to_owned(), command, args, env))
}

/// The MCP servers the user registered in settings (D162): `mcp.servers` for every project,
/// then `mcp.projects.<project_id>.servers`, where an entry replaces a global one of the same
/// name and `null` switches a global one off for that project. Each entry is `{"command",
/// "args", "env"}`, delivered exactly like a plugin's server. A name that is not
/// `[A-Za-z0-9_-]`, the reserved `relay`, a name an enabled plugin already serves (`taken`),
/// or a malformed entry is skipped with a warning: a bad setting never fails a launch.
pub fn custom_mcp_servers(
    tx: &rusqlite::Transaction,
    project_id: Id,
    relay: &Path,
    taken: &[LaunchServer],
) -> Vec<LaunchServer> {
    let mut merged: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for path in ["mcp.servers".to_string(), format!("mcp.projects.{project_id}.servers")] {
        match crate::handlers::settings::get(tx, Some(&path)) {
            Ok(serde_json::Value::Object(entries)) => merged.extend(entries),
            Ok(serde_json::Value::Null) => {}
            Ok(_) => tracing::warn!(%path, "custom MCP servers: not an object of name -> server"),
            Err(error) => tracing::warn!(%path, error = %error.message, "custom MCP servers: unreadable"),
        }
    }
    let mut out: Vec<LaunchServer> = Vec::new();
    for (name, spec) in merged {
        if spec.is_null() {
            continue;
        }
        if !valid_server_name(&name) || name == "relay" {
            tracing::warn!(server = %name, "custom MCP server skipped: the name must be [A-Za-z0-9_-] and not `relay`");
            continue;
        }
        if taken.iter().any(|(plugin, ..)| plugin == &name) {
            tracing::warn!(server = %name, "custom MCP server skipped: an enabled plugin serves that name");
            continue;
        }
        match custom_server(&name, &spec, relay) {
            Ok(server) => out.push(server),
            Err(why) => tracing::warn!(server = %name, %why, "custom MCP server skipped"),
        }
    }
    out
}

/// The brief section that makes an enabled plugin's rules part of every agent's standing
/// context: its instructions verbatim, then what it installed and where.
pub fn brief_section(conn: &Connection, project_id: Id) -> Result<String> {
    let plugins = enabled_for(conn, project_id)?;
    if plugins.is_empty() {
        return Ok("No plugins are on for this project.".into());
    }
    let mut out = String::new();
    for plugin in plugins {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("### {} plugin (on for this project)\n\n", plugin.manifest.name));
        let instructions = plugin.instructions();
        if !instructions.is_empty() {
            out.push_str(&instructions);
            out.push_str("\n\n");
        }
        if !plugin.skills.is_empty() {
            out.push_str(&format!(
                "Skills from this plugin, installed as folders under {}/ and {}/ — load the matching one before the work it covers:\n",
                crate::skills::TARGETS[0],
                crate::skills::TARGETS[1]
            ));
            for skill in &plugin.skills {
                out.push_str(&format!("- {} — {}\n", skill.dir, skill.description));
            }
        }
        for server in &plugin.manifest.mcp_servers {
            out.push_str(&format!(
                "\nMCP server `{}`: {}\nTools: {}\n",
                server.name,
                server.description,
                server.tools.join(", ")
            ));
        }
    }
    Ok(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unreal_engine_plugin_is_bundled_and_complete() {
        let plugin = get("unreal-engine").expect("unreal-engine is bundled");
        assert_eq!(plugin.manifest.name, "Unreal Engine");
        assert!(!plugin.instructions().is_empty(), "no always-on instructions");
        for doc in &plugin.manifest.docs {
            assert!(plugin.file(doc).is_some(), "manifest names a missing doc {doc}");
        }
        assert!(plugin.skills.len() >= 10, "only {} skills", plugin.skills.len());
        for skill in &plugin.skills {
            assert_eq!(
                frontmatter(&skill.body, "name").as_deref(),
                Some(skill.dir.as_str()),
                "{}: front matter name must equal its folder",
                skill.dir
            );
            assert!(!skill.description.is_empty(), "{} has no description", skill.dir);
            assert_eq!(crate::skills::folder_name(&skill.dir), skill.dir, "{} is not provider-safe", skill.dir);
        }
        let server = &plugin.manifest.mcp_servers[0];
        assert_eq!(server.name, "unreal");
        assert_eq!(server.command, "relay");
    }

    #[test]
    fn the_blender_plugin_is_bundled_and_complete() {
        let plugin = get("blender").expect("blender is bundled");
        assert!(!plugin.instructions().is_empty());
        for doc in &plugin.manifest.docs {
            assert!(plugin.file(doc).is_some(), "manifest names a missing doc {doc}");
        }
        assert_eq!(plugin.manifest.mcp_servers[0].args, vec!["blender-mcp".to_string()]);
        for skill in &plugin.skills {
            assert_eq!(frontmatter(&skill.body, "name").as_deref(), Some(skill.dir.as_str()));
            assert!(!skill.description.is_empty(), "{} has no description", skill.dir);
        }
        // Two plugins on at once must not collide on skill folders or server names.
        let unreal = get("unreal-engine").unwrap();
        assert!(plugin.skills.iter().all(|s| unreal.skills.iter().all(|u| u.dir != s.dir)));
        assert_ne!(plugin.manifest.mcp_servers[0].name, unreal.manifest.mcp_servers[0].name);
    }

    #[test]
    fn frontmatter_reads_single_line_values_only_inside_the_block() {
        let body = "---\nname: unreal-ai\ndescription: \"Behavior trees, EQS\"\n---\n\ndescription: not this\n";
        assert_eq!(frontmatter(body, "name").as_deref(), Some("unreal-ai"));
        assert_eq!(frontmatter(body, "description").as_deref(), Some("Behavior trees, EQS"));
        assert_eq!(frontmatter("# no block\ndescription: x", "description"), None);
    }

    #[test]
    fn detection_reads_one_listing() {
        let plugin = get("unreal-engine").unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(!detect(plugin, root.path()));
        std::fs::create_dir_all(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/Deep.uproject"), "{}").unwrap();
        assert!(!detect(plugin, root.path()), "detection must not walk the tree");
        std::fs::write(root.path().join("Shooter.uproject"), "{}").unwrap();
        assert!(detect(plugin, root.path()));
    }
}
