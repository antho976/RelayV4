//! Installed skills as folders, not just instruction text (D147).
//!
//! A skill installed once belongs to the whole app: its folder is kept beside the store in
//! `<data>/skills/<id>/` and materialized into every project root and every session worktree
//! as a real provider skill (`.claude/skills/<name>/`, `.agents/skills/<name>/`), so an agent
//! in any project can load it. Relay only ever owns folders carrying [`MARKER`]; a skill the
//! repository checks in itself is left exactly as it is.

use anyhow::{Context, Result};
use relay_bus::types::Id;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

/// Written into every skill folder Relay materializes. Its contents are the freshness stamp;
/// its absence means the folder belongs to the repository and must not be touched.
pub const MARKER: &str = ".relay-skill";

/// Provider folders a materialized skill is written into, relative to a checkout root.
pub const TARGETS: [&str; 2] = [".claude/skills", ".agents/skills"];

/// Bounds on one adopted skill folder — a `SKILL.md` at a repository root would otherwise
/// pull the entire repository into the library.
const MAX_FILES: usize = 4_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 12;

/// One enabled skill as the materializer needs it.
struct Row {
    dir: String,
    body: String,
    stamp: String,
    source: Source,
}

/// Where a skill folder's files come from.
enum Source {
    /// An installed skill: its library folder beside the store, if it has one.
    Library(Id),
    /// A skill of a bundled plugin (D159): files relative to the skill folder.
    Plugin(Vec<(&'static str, &'static [u8])>),
}

/// `<store dir>/skills/<skill id>/` — the app-wide copy of one skill's folder.
pub fn library_dir(store: &crate::Store, id: Id) -> PathBuf {
    store.skills_dir().join(id.to_string())
}

/// Copy a downloaded skill folder into the library, replacing whatever was there. Callers
/// treat failure as "no assets": the instruction body alone still makes a usable skill.
pub fn adopt(from: &Path, to: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(to);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    match copy_bounded(from, to) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_dir_all(to);
            Err(error)
        }
    }
}

/// Copy a folder, skipping `.git` and symlinks, refusing anything past the bounds above.
pub fn copy_bounded(from: &Path, to: &Path) -> Result<()> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    copy_into(from, to, 0, &mut files, &mut bytes)
}

fn copy_into(from: &Path, to: &Path, depth: usize, files: &mut usize, bytes: &mut u64) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() || entry.file_name() == ".git" || entry.file_name() == MARKER {
            continue;
        }
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_into(&entry.path(), &target, depth + 1, files, bytes)?;
            continue;
        }
        *files += 1;
        *bytes += entry.metadata()?.len();
        anyhow::ensure!(
            *files <= MAX_FILES && *bytes <= MAX_BYTES,
            "{} exceeds the skill folder bounds ({MAX_FILES} files, {} MiB)",
            from.display(),
            MAX_BYTES / (1024 * 1024)
        );
        fs::copy(entry.path(), &target)
            .with_context(|| format!("copying {}", entry.path().display()))?;
    }
    Ok(())
}

/// Folder name for a skill: the public name, reduced to something every provider can read.
pub fn folder_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars().take(64) {
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "skill".into() } else { out }
}

fn enabled(conn: &Connection, project_id: Id) -> Result<Vec<Row>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.id,s.name,s.body,s.updated_at,COALESCE(s.revision,'') FROM skills s
         JOIN skill_projects sp ON sp.skill_id=s.id
         WHERE sp.project_id=?1 AND s.deleted_at IS NULL
         ORDER BY s.name COLLATE NOCASE,s.id",
    )?;
    let rows = stmt.query_map([project_id], row_of)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(with_plugins(rows, &crate::plugins::enabled_for(conn, project_id)?))
}

/// Add the skills of enabled plugins. An installed skill with the same folder name wins: the
/// user put it there on purpose, and two writers of one folder would rewrite it forever.
fn with_plugins(mut rows: Vec<Row>, plugins: &[&'static crate::plugins::Loaded]) -> Vec<Row> {
    for plugin in plugins {
        for skill in &plugin.skills {
            if rows.iter().any(|row| row.dir == skill.dir) {
                continue;
            }
            rows.push(Row {
                dir: skill.dir.clone(),
                body: skill.body.clone(),
                stamp: format!("plugin {} {} {}\n", plugin.id(), plugin.bundle.digest, skill.dir),
                source: Source::Plugin(skill.files.clone()),
            });
        }
    }
    rows
}

fn row_of(row: &rusqlite::Row) -> rusqlite::Result<Row> {
    let id: Id = row.get(0)?;
    let name: String = row.get(1)?;
    let updated_at: String = row.get(3)?;
    let revision: String = row.get(4)?;
    Ok(Row {
        dir: folder_name(&name),
        body: row.get(2)?,
        stamp: format!("{id} {updated_at} {revision}\n"),
        source: Source::Library(id),
    })
}

/// Every skill enabled in at least one project. The user-scope folders are one per machine,
/// so they cannot carry a per-project answer; a project that switched a skill off still keeps
/// it out of its own checkout, which is where a provider looks first.
fn enabled_anywhere(conn: &Connection) -> Result<Vec<Row>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.id,s.name,s.body,s.updated_at,COALESCE(s.revision,'') FROM skills s
         WHERE s.deleted_at IS NULL AND EXISTS(SELECT 1 FROM skill_projects sp WHERE sp.skill_id=s.id)
         ORDER BY s.name COLLATE NOCASE,s.id",
    )?;
    let rows = stmt.query_map([], row_of)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(with_plugins(rows, &crate::plugins::enabled_anywhere(conn)?))
}

/// The machine-wide skill folders. Codex is the reason this exists: it reads skills only from
/// `$CODEX_HOME/skills` and knows nothing about a checkout's `.claude/` or `.agents/`, so a
/// Codex session would otherwise never see an installed skill as a skill. Claude's user scope
/// is included for the same reach outside Relay's own projects.
pub fn user_bases(instance: crate::Instance) -> Vec<PathBuf> {
    // A test engine must never write into the developer's own home; `RELAY_SKILLS_HOME` gives
    // tests and scratch runs a harmless place to prove the same path end to end.
    let (home, codex) = match std::env::var_os("RELAY_SKILLS_HOME") {
        Some(path) => {
            let home = PathBuf::from(path);
            let codex = home.join(".codex");
            (home, codex)
        }
        None if instance == crate::Instance::Test => return Vec::new(),
        None => {
            let Some(dirs) = directories::BaseDirs::new() else { return Vec::new() };
            let home = dirs.home_dir().to_path_buf();
            let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
            (home, codex)
        }
    };
    vec![home.join(".claude").join("skills"), codex.join("skills")]
}

/// What one set of skill folders should hold, read under the store lock so the copying can
/// happen without it: skill folders are megabytes, and the store mutex is not where megabytes
/// belong (D144).
pub struct Plan {
    /// Directories that hold one folder per skill.
    bases: Vec<PathBuf>,
    /// The checkout those folders sit in, when they do: its `git status` has to stay clean.
    repo: Option<PathBuf>,
    rows: Vec<Row>,
}

/// Read the skills enabled for `project_id` into a plan for the checkout at `root`.
pub fn plan(conn: &Connection, root: &Path, project_id: Id) -> Result<Plan> {
    Ok(Plan {
        bases: TARGETS.iter().map(|target| root.join(target)).collect(),
        repo: Some(root.to_path_buf()),
        rows: enabled(conn, project_id)?,
    })
}

/// Read every enabled skill into a plan for the machine-wide provider folders.
pub fn plan_user(conn: &Connection, instance: crate::Instance) -> Result<Plan> {
    let bases = user_bases(instance);
    let rows = if bases.is_empty() { Vec::new() } else { enabled_anywhere(conn)? };
    Ok(Plan { bases, repo: None, rows })
}

/// Write a plan out: every enabled skill becomes a provider skill folder, folders of skills
/// that are no longer enabled are removed, and results inside a checkout stay out of
/// `git status`. Best-effort per skill — one unwritable folder never costs the caller its
/// session.
pub fn apply(plan: &Plan, store: &crate::Store) -> Result<()> {
    let mut written: Vec<String> = Vec::new();
    for base in &plan.bases {
        prune(base, &plan.rows);
        for row in &plan.rows {
            let dest = base.join(&row.dir);
            match write_skill(store, row, &dest) {
                Ok(true) => {
                    if let Some(relative) = plan.repo.as_ref().and_then(|repo| dest.strip_prefix(repo).ok()) {
                        written.push(format!("{}/", relative.display()));
                    }
                }
                Ok(false) => {}
                Err(error) => tracing::warn!(skill = %row.dir, error = %error, "materializing skill"),
            }
        }
    }
    if let (Some(repo), false) = (plan.repo.as_ref(), written.is_empty()) {
        let entries: Vec<&str> = written.iter().map(String::as_str).collect();
        crate::worktree::exclude_paths(repo, &entries)?;
    }
    Ok(())
}

/// Plan and apply in one step, for callers already holding the transaction that launched.
pub fn materialize(conn: &Connection, store: &crate::Store, root: &Path, project_id: Id) -> Result<()> {
    apply(&plan(conn, root, project_id)?, store)
}

/// The same for the machine-wide folders, so a launching session finds its skills registered
/// with whichever provider it is.
pub fn materialize_user(conn: &Connection, store: &crate::Store, instance: crate::Instance) -> Result<()> {
    apply(&plan_user(conn, instance)?, store)
}

/// Push every enabled skill into every project root and every session worktree that still
/// exists. This is what makes a skill installed today show up in workspaces created yesterday.
pub fn refresh_all(engine: &crate::engine::Engine) {
    let plans = {
        let conn = engine.store.lock();
        let mut roots: Vec<(Id, PathBuf)> = Vec::new();
        let mut read = |sql: &str| {
            let Ok(mut stmt) = conn.prepare(sql) else { return };
            let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, Id>(0)?, PathBuf::from(row.get::<_, String>(1)?)))
            }) else { return };
            roots.extend(rows.flatten());
        };
        read("SELECT id,path FROM projects");
        read("SELECT project_id,worktree FROM sessions WHERE worktree != '' AND state != 'closed'");
        let mut plans = match plan_user(&conn, engine.instance) {
            Ok(plan) => vec![plan],
            Err(error) => {
                tracing::warn!(error = %error, "planning user-scope skills");
                Vec::new()
            }
        };
        plans.extend(roots
            .into_iter()
            .filter(|(_, root)| root.is_dir())
            .filter_map(|(project_id, root)| match plan(&conn, &root, project_id) {
                Ok(plan) => Some(plan),
                Err(error) => {
                    tracing::warn!(root = %root.display(), error = %error, "planning skills");
                    None
                }
            })
            .collect::<Vec<_>>());
        plans
    };
    for plan in plans {
        if let Err(error) = apply(&plan, &engine.store) {
            tracing::warn!(error = %error, "materializing skills");
        }
    }
}

/// Returns whether the folder is Relay's to write (a checked-in skill of the same name wins).
fn write_skill(store: &crate::Store, row: &Row, dest: &Path) -> Result<bool> {
    let marker = dest.join(MARKER);
    if dest.exists() && !marker.is_file() {
        return Ok(false);
    }
    if fs::read_to_string(&marker).is_ok_and(|stamp| stamp == row.stamp) {
        return Ok(true);
    }
    // A half-written folder must not survive: without its marker the next pass would read it
    // as a folder the repository owns and never touch it again.
    if let Err(error) = fill(store, row, dest) {
        let _ = fs::remove_dir_all(dest);
        return Err(error);
    }
    Ok(true)
}

fn fill(store: &crate::Store, row: &Row, dest: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(dest);
    match &row.source {
        Source::Library(id) if library_dir(store, *id).is_dir() => copy_bounded(&library_dir(store, *id), dest)?,
        Source::Library(_) => fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?,
        Source::Plugin(files) => {
            for (relative, bytes) in files {
                let path = dest.join(relative);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
                }
                fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
            }
        }
    }
    let instructions = dest.join("SKILL.md");
    if !instructions.is_file() {
        fs::write(&instructions, &row.body)
            .with_context(|| format!("writing {}", instructions.display()))?;
    }
    let marker = dest.join(MARKER);
    fs::write(&marker, &row.stamp).with_context(|| format!("writing {}", marker.display()))
}

/// Remove skill folders Relay owns that no longer correspond to an enabled skill.
fn prune(base: &Path, rows: &[Row]) {
    let Ok(entries) = fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !path.join(MARKER).is_file() || rows.iter().any(|row| row.dir == name) {
            continue;
        }
        let _ = fs::remove_dir_all(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_provider_safe() {
        assert_eq!(folder_name("  Impeccable  "), "impeccable");
        assert_eq!(folder_name("Review Carefully!"), "review-carefully");
        assert_eq!(folder_name("///"), "skill");
    }

    #[test]
    fn user_scope_covers_both_providers_and_never_touches_a_test_engine_home() {
        assert!(
            user_bases(crate::Instance::Test).is_empty(),
            "a test engine must not write into the developer's real home",
        );
        let home = tempfile::tempdir().unwrap();
        // SAFETY: the only test that touches this variable, restored before it returns.
        unsafe { std::env::set_var("RELAY_SKILLS_HOME", home.path()) };
        let bases = user_bases(crate::Instance::Test);
        assert_eq!(bases, vec![home.path().join(".claude/skills"), home.path().join(".codex/skills")]);

        // Codex reads skills from its own home and nowhere else, so this is the only place a
        // Codex session can see one.
        let store = crate::Store::open_memory().unwrap();
        store.with_tx(|tx| {
            tx.execute("INSERT INTO workspaces(path,name,ord,created_at,updated_at) VALUES ('/w','w',0,'t','t')", [])?;
            tx.execute("INSERT INTO projects(workspace_id,path,name,base_branch,ord,created_at,updated_at) VALUES (1,'/w/p','p','main',0,'t','t')", [])?;
            tx.execute("INSERT INTO skills(name,body,created_at,updated_at) VALUES ('Impeccable','design well','t','t')", [])?;
            tx.execute("INSERT INTO skill_projects(skill_id,project_id) VALUES (1,1)", [])?;
            Ok(())
        }).unwrap();
        let hand_written = home.path().join(".codex/skills/mine");
        fs::create_dir_all(&hand_written).unwrap();
        fs::write(hand_written.join("SKILL.md"), "mine").unwrap();

        {
            let conn = store.lock();
            materialize_user(&conn, &store, crate::Instance::Test).unwrap();
        }
        for base in &bases {
            assert_eq!(fs::read_to_string(base.join("impeccable/SKILL.md")).unwrap(), "design well");
        }
        // Switching it off in the only project that had it takes it out of the shared homes.
        store.with_tx(|tx| { tx.execute("DELETE FROM skill_projects", [])?; Ok(()) }).unwrap();
        {
            let conn = store.lock();
            materialize_user(&conn, &store, crate::Instance::Test).unwrap();
        }
        assert!(!bases[1].join("impeccable").exists(), "a disabled skill stayed in the codex home");
        assert_eq!(
            fs::read_to_string(hand_written.join("SKILL.md")).unwrap(),
            "mine",
            "pruning reached a skill the user wrote themselves",
        );
        unsafe { std::env::remove_var("RELAY_SKILLS_HOME") };
    }

    #[test]
    fn bounded_copy_skips_git_and_symlinks() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join(".git")).unwrap();
        fs::write(src.path().join(".git/config"), "x").unwrap();
        fs::create_dir_all(src.path().join("reference")).unwrap();
        fs::write(src.path().join("reference/polish.md"), "polish").unwrap();
        fs::write(src.path().join("SKILL.md"), "body").unwrap();
        std::os::unix::fs::symlink(src.path().join("SKILL.md"), src.path().join("link.md")).unwrap();
        let into = dest.path().join("out");
        copy_bounded(src.path(), &into).unwrap();
        assert_eq!(fs::read_to_string(into.join("reference/polish.md")).unwrap(), "polish");
        assert!(into.join("SKILL.md").is_file());
        assert!(!into.join(".git").exists());
        assert!(!into.join("link.md").exists());
    }
}
