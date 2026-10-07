//! `provider.*` — explicit, cached CLI discovery. No polling and no background subprocesses.

use crate::engine::{Ctx, Engine, IntoBus};
use crate::github::DownloadedSkill;
use relay_bus::ops::provider::*;
use relay_bus::types::{Id, PluginDoc, Skill, Usage};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::json;
use std::collections::BTreeMap;

pub fn register(e: &mut Engine) {
    crate::provider_updates::register(e);
    e.register_unlocked::<List>(|ctx, _| {
        Ok(ListOut {
            providers: ctx.read(crate::providers::list)?,
        })
    });
    // Each probe is a subprocess (`--version`, auth status) with a deadline of seconds; they
    // run before the transaction opens and only the cache rows are written under it (D149).
    e.register_staged::<Refresh, _>(
        |ctx, _| {
            let paths = ctx.read(|conn| Ok(crate::providers::paths(conn)))?;
            Ok(crate::providers::probe(paths))
        },
        |ctx: &mut Ctx, _, probes| {
        let discovered = crate::providers::record(ctx.tx(), &ctx.now, probes)?;
        for item in &discovered {
            if item.version_changed {
                let name = crate::sessions::provider_str(item.info.provider);
                let current = item.info.version.as_deref().unwrap_or("not installed");
                let previous = item.previous_version.as_deref().unwrap_or("unknown");
                ctx.tx().execute(
                    "INSERT INTO notifications(project_id,category,title,body,link,read,created_at)
                     VALUES (NULL,'provider',?1,?2,NULL,0,?3)",
                    params![format!("{name} version changed"), format!("{previous} → {current}"), ctx.now],
                ).bus()?;
                ctx.emit("notify.new", json!({"category":"provider", "provider":name}));
            }
            ctx.emit("provider.version", json!({
                "provider": crate::sessions::provider_str(item.info.provider),
                "installed": item.info.installed,
                "previous": item.previous_version,
                "current": item.info.version,
                "changed": item.version_changed,
            }));
        }
        Ok(ListOut { providers: discovered.into_iter().map(|item| item.info).collect() })
        },
    );
    // The reported half is three columns of SQLite; the discovered half walks each provider's
    // session directory and tails JSONL files. Only the first belongs on the lock (D144).
    e.register_unlocked::<UsageGet>(|ctx, payload| {
        let reported = ctx.read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT provider,usage,updated_at FROM sessions WHERE usage IS NOT NULL ORDER BY updated_at DESC,id DESC",
            ).bus()?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))).bus()?;
            rows.collect::<rusqlite::Result<Vec<_>>>().bus()
        })?;
        let mut latest = BTreeMap::new();
        for (provider, windows, taken_at) in reported {
            latest.entry(provider.clone()).or_insert_with(|| Usage {
                provider: crate::sessions::parse_provider(&provider),
                windows: serde_json::from_str(&windows).unwrap_or(serde_json::Value::Null),
                taken_at,
            });
        }
        // What a provider left behind wins over a report, whatever their ages. A test engine
        // never reads the developer's own provider state; `tests/usage_source.rs` drives this
        // path on a dev instance with the provider homes in a temp dir.
        if ctx.engine().instance != crate::Instance::Test {
            for provider in [
                relay_bus::types::Provider::Claude,
                relay_bus::types::Provider::Codex,
            ] {
                if payload.provider.is_none_or(|wanted| wanted == provider) {
                    if let Some(item) = crate::usage::read(provider) {
                        latest.insert(crate::sessions::provider_str(provider).to_string(), item);
                    }
                }
            }
        }
        let usage = latest.into_values().filter(|item| payload.provider.is_none_or(|provider| provider == item.provider)).collect();
        Ok(UsageGetOut { usage })
    });
    e.register::<UsageReport>(|ctx: &mut Ctx, payload| {
        let row = crate::sessions::by_name(ctx.tx(), &payload.session)?;
        if ctx.actor.session_name() != Some(payload.session.as_str()) || row.session.provider != payload.provider {
            return Err(relay_bus::BusError::refused("usage.session", "usage reports must match the bound session and provider"));
        }
        ctx.tx().execute("UPDATE sessions SET usage=?1,updated_at=?2 WHERE id=?3",
            params![serde_json::to_string(&payload.payload).bus()?,ctx.now,row.session.id]).bus()?;
        ctx.set_project(row.session.project_id);
        ctx.emit("usage.changed", json!({"session":payload.session,"provider":crate::sessions::provider_str(payload.provider),"usage":payload.payload}));
        Ok(relay_bus::Empty {})
    });
    e.register::<SkillList>(|ctx, payload| {
        if let Some(project_id) = payload.project_id {
            crate::handlers::workspace::get_project(ctx.tx(), project_id)?;
        }
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "SELECT * FROM skills WHERE deleted_at IS NULL ORDER BY name COLLATE NOCASE,id",
            )
            .bus()?;
        let mut skills = stmt
            .query_map([], |row| skill_row(ctx.tx(), row))
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        if let Some(enabled) = payload.enabled {
            skills.retain(|skill| match payload.project_id {
                Some(project_id) => skill.enabled_in.contains(&project_id) == enabled,
                None => skill.enabled_in.is_empty() != enabled,
            });
        }
        if payload.summary == Some(true) {
            skills.iter_mut().for_each(summarize);
        }
        Ok(SkillListOut { skills })
    });
    e.register::<SkillGet>(|ctx, payload| get_skill(ctx.tx(), payload.skill_id));
    e.register::<SkillCreate>(|ctx: &mut Ctx, payload| {
        let name = valid_skill_name(&payload.name)?;
        valid_skill_body(&payload.body)?;
        let existing: Option<(Id, Option<String>, String)> = ctx
            .tx()
            .query_row(
                "SELECT id,deleted_at,body FROM skills WHERE name=?1 COLLATE NOCASE",
                [&name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .bus()?;
        // A removed row keeps its name, so a new skill of that name reuses it. Only the undo of
        // `skill.delete` — which recreates the exact body it removed — gets the old skill back
        // whole; anything else is a new local skill, and inheriting the old GitHub source would
        // let "Update from GitHub" overwrite it, and the old folder would ride along to agents.
        let mut restored = false;
        let id =
            match existing {
                Some((id, Some(_), body)) if body == payload.body => {
                    restored = true;
                    ctx.tx().execute(
                    "UPDATE skills SET name=?1,deleted_at=NULL,updated_at=?2 WHERE id=?3",
                    params![name,ctx.now,id],
                ).bus()?;
                    id
                }
                Some((id, Some(_), _)) => {
                    ctx.tx().execute(
                    "UPDATE skills SET name=?1,body=?2,source_url=NULL,source_path=NULL,source_ref=NULL,revision=NULL,deleted_at=NULL,updated_at=?3 WHERE id=?4",
                    params![name,payload.body,ctx.now,id],
                ).bus()?;
                    id
                }
                Some((id, None, _)) => {
                    return Err(relay_bus::BusError::conflict(
                        "skill.name_exists",
                        format!("a skill named {name:?} already exists as {id}"),
                    ))
                }
                None => {
                    ctx.tx().execute(
                    "INSERT INTO skills(name,body,created_at,updated_at) VALUES (?1,?2,?3,?3)",
                    params![name,payload.body,ctx.now],
                ).bus()?;
                    ctx.tx().last_insert_rowid()
                }
            };
        enable_everywhere(ctx.tx(), id)?;
        let skill = get_skill(ctx.tx(), id)?;
        if !restored {
            // Also clears an orphan a rolled-back install left under a fresh id.
            ctx.after_commit(move |engine| {
                let _ = std::fs::remove_dir_all(crate::skills::library_dir(&engine.store, id));
            });
        }
        ctx.set_undo(
            "skill.delete",
            json!({"skill_id":id}),
            Some(json!({"updated_at":skill.updated_at})),
        );
        ctx.emit("skill.changed", serde_json::to_value(&skill).bus()?);
        refresh_checkouts(ctx);
        Ok(skill)
    });
    e.register::<SkillUpdate>(|ctx: &mut Ctx, payload| {
        let before = get_skill(ctx.tx(), payload.skill_id)?;
        let name = match payload.name { Some(name) => valid_skill_name(&name)?, None => before.name.clone() };
        let body = payload.body.unwrap_or_else(|| before.body.clone());
        valid_skill_body(&body)?;
        let duplicate: Option<Id> = ctx.tx().query_row(
            "SELECT id FROM skills WHERE name=?1 COLLATE NOCASE AND id!=?2",
            params![name,before.id], |row| row.get(0),
        ).optional().bus()?;
        if let Some(id) = duplicate {
            return Err(relay_bus::BusError::conflict("skill.name_exists", format!("that name is used by skill {id}")));
        }
        ctx.tx().execute("UPDATE skills SET name=?1,body=?2,updated_at=?3 WHERE id=?4 AND deleted_at IS NULL",
            params![name,body,ctx.now,before.id]).bus()?;
        let skill = get_skill(ctx.tx(), before.id)?;
        ctx.set_undo("skill.update", json!({"skill_id":before.id,"name":before.name,"body":before.body}), Some(json!({"updated_at":skill.updated_at})));
        ctx.emit("skill.changed", serde_json::to_value(&skill).bus()?);
        refresh_checkouts(ctx);
        Ok(skill)
    });
    e.register::<SkillDelete>(|ctx: &mut Ctx, payload| {
        let before = get_skill(ctx.tx(), payload.skill_id)?;
        ctx.tx()
            .execute(
                "UPDATE skills SET deleted_at=?1,updated_at=?1 WHERE id=?2",
                params![ctx.now, before.id],
            )
            .bus()?;
        ctx.set_undo(
            "skill.create",
            json!({"name":before.name,"body":before.body}),
            None,
        );
        ctx.emit("skill.deleted", json!({"id":before.id}));
        refresh_checkouts(ctx);
        Ok(relay_bus::Empty {})
    });
    e.register::<SkillEnable>(|ctx: &mut Ctx, payload| {
        crate::handlers::workspace::get_project(ctx.tx(), payload.project_id)?;
        let before = get_skill(ctx.tx(), payload.skill_id)?;
        let was_enabled = before.enabled_in.contains(&payload.project_id);
        if payload.enabled {
            ctx.tx().execute("INSERT OR IGNORE INTO skill_projects(skill_id,project_id) VALUES (?1,?2)", params![payload.skill_id,payload.project_id]).bus()?;
        } else {
            ctx.tx().execute("DELETE FROM skill_projects WHERE skill_id=?1 AND project_id=?2", params![payload.skill_id,payload.project_id]).bus()?;
        }
        ctx.tx().execute("UPDATE skills SET updated_at=?1 WHERE id=?2", params![ctx.now,payload.skill_id]).bus()?;
        ctx.set_project(payload.project_id);
        let skill = get_skill(ctx.tx(), payload.skill_id)?;
        ctx.set_undo("skill.enable", json!({"skill_id":payload.skill_id,"project_id":payload.project_id,"enabled":was_enabled}), Some(json!({"updated_at":skill.updated_at})));
        ctx.emit("skill.changed", serde_json::to_value(&skill).bus()?);
        refresh_checkouts(ctx);
        Ok(skill)
    });
    // The download is `gh`/network plus a tree copy; only the adoption into the library is a
    // write. Downloading first, unlocked, keeps a slow clone off every other op (D144).
    e.register_staged::<SkillInstall, _>(
        |ctx, payload| {
            // A skill is its folder, not only its SKILL.md: reference documents and scripts are
            // staged beside the store and adopted into the library once the row has an id (D147).
            let reference = install_ref(ctx, payload)?;
            let staging = Staging(ctx.engine().store.skills_dir().join(".staging").join(uuid::Uuid::new_v4().to_string()));
            let items = crate::github::download_skills(&payload.url, payload.subdir.as_deref(), reference.as_deref(), Some(&staging.0))
                .and_then(deduplicate_downloaded_skills)?;
            Ok((staging, items))
        },
        |ctx: &mut Ctx, payload, (staging, downloaded)| {
        let mut skills = Vec::with_capacity(downloaded.len());
        let mut folders = Vec::with_capacity(downloaded.len());
        for mut item in downloaded {
            let assets = item.assets.take();
            let skill = install_downloaded_skill(ctx.tx(), &ctx.now, item, payload.replace_skill_id)?;
            folders.push((skill.id, assets));
            ctx.emit("skill.changed", serde_json::to_value(&skill).bus()?);
            skills.push(skill);
        }
        // The library is written only once the rows are committed (a folder adopted for a
        // rolled-back row would be inherited by the next skill given its id), and it then holds
        // exactly what this install brought. A rollback drops this closure, and `staging` with it.
        ctx.after_commit(move |engine| {
            for (id, assets) in folders {
                let library = crate::skills::library_dir(&engine.store, id);
                match assets {
                    Some(assets) => if let Err(error) = crate::skills::adopt(&assets, &library) {
                        tracing::warn!(skill = id, error = %error, "storing skill folder");
                    },
                    None => { let _ = std::fs::remove_dir_all(&library); }
                }
            }
            drop(staging);
        });
        refresh_checkouts(ctx);
        // A whole repository's bodies on one reply line can outgrow a client's line limit.
        skills.iter_mut().for_each(summarize);
        Ok(SkillInstallOut { skills })
        },
    );
    e.register_unlocked::<GitHubStatusOp>(|_, _| Ok(crate::github::status()));
    e.register::<GitHubConnect>(|ctx: &mut Ctx, _| {
        let gh = crate::github::gh_path()?;
        ctx.after_commit(move |engine| {
            std::thread::spawn(move || {
                let connected = crate::github::connect(gh).unwrap_or(false);
                let status = crate::github::status();
                engine.emit_system(
                    "github.changed",
                    json!({"connected":connected && status.connected,"login":status.login}),
                );
            });
        });
        Ok(GitHubConnectOut { started: true })
    });
    e.register_unlocked::<GitHubRepoList>(|_, _| {
        Ok(GitHubRepoListOut {
            repositories: crate::github::repositories()?,
        })
    });
    // Detection lists each project root once; that is filesystem work, so it runs with the
    // store lock released and only the enable edges are read under it (D144).
    e.register_unlocked::<PluginList>(|ctx, payload| {
        let (projects, edges) = ctx.read(|conn| {
            let mut stmt = conn.prepare_cached("SELECT id,path FROM projects ORDER BY id").bus()?;
            let projects = stmt
                .query_map([], |row| Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?)))
                .bus()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .bus()?;
            let mut edges = BTreeMap::<String, Vec<Id>>::new();
            for plugin in crate::plugins::all() {
                edges.insert(plugin.id().to_string(), crate::plugins::projects_for(conn, plugin.id()).bus()?);
            }
            Ok((projects, edges))
        })?;
        let plugins = crate::plugins::all()
            .iter()
            .map(|plugin| {
                let enabled_in = edges.get(plugin.id()).cloned().unwrap_or_default();
                let suggested_for = projects
                    .iter()
                    .filter(|(id, _)| !enabled_in.contains(id))
                    .filter(|(id, _)| payload.project_id.is_none_or(|only| only == *id))
                    .filter(|(_, path)| crate::plugins::detect(plugin, std::path::Path::new(path)))
                    .map(|(id, _)| *id)
                    .collect();
                crate::plugins::to_bus(plugin, enabled_in, suggested_for)
            })
            .collect();
        Ok(PluginListOut { plugins })
    });
    e.register_unlocked::<PluginGet>(|ctx, payload| {
        let plugin = find_plugin(&payload.plugin_id)?;
        let enabled_in = ctx.read(|conn| crate::plugins::projects_for(conn, plugin.id()).bus())?;
        let skill = match payload.skill.as_deref() {
            None => None,
            Some(name) => {
                let skill = plugin.skills.iter().find(|skill| skill.dir == name).ok_or_else(|| {
                    relay_bus::BusError::not_found("plugin.skill_not_found", format!("{} has no skill {name:?}", plugin.id()))
                })?;
                Some(PluginDoc { path: format!("skills/{}/SKILL.md", skill.dir), body: skill.body.clone() })
            }
        };
        Ok(PluginGetOut {
            plugin: crate::plugins::to_bus(plugin, enabled_in, Vec::new()),
            instructions: plugin.instructions(),
            docs: crate::plugins::docs(plugin),
            skill,
        })
    });
    e.register::<PluginEnable>(|ctx: &mut Ctx, payload| {
        let plugin = find_plugin(&payload.plugin_id)?;
        crate::handlers::workspace::get_project(ctx.tx(), payload.project_id)?;
        let was_enabled = crate::plugins::projects_for(ctx.tx(), plugin.id()).bus()?.contains(&payload.project_id);
        if payload.enabled {
            ctx.tx().execute(
                "INSERT OR IGNORE INTO plugin_projects(plugin_id,project_id,enabled_at) VALUES (?1,?2,?3)",
                params![plugin.id(), payload.project_id, ctx.now],
            ).bus()?;
        } else {
            ctx.tx().execute(
                "DELETE FROM plugin_projects WHERE plugin_id=?1 AND project_id=?2",
                params![plugin.id(), payload.project_id],
            ).bus()?;
        }
        ctx.set_project(payload.project_id);
        let enabled_in = crate::plugins::projects_for(ctx.tx(), plugin.id()).bus()?;
        let out = crate::plugins::to_bus(plugin, enabled_in, Vec::new());
        ctx.set_undo(
            "plugin.enable",
            json!({"plugin_id":plugin.id(),"project_id":payload.project_id,"enabled":was_enabled}),
            None,
        );
        ctx.emit("plugin.changed", json!({"id":plugin.id(),"project_id":payload.project_id,"enabled":payload.enabled}));
        // Skill folders reach existing checkouts now; MCP servers and the brief reach each agent
        // on its next start or resume, which is when a provider reads them.
        refresh_checkouts(ctx);
        Ok(out)
    });
}

/// A staging folder for one `skill.install`, removed when the install is done with it —
/// committed, refused or rolled back.
struct Staging(std::path::PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn find_plugin(id: &str) -> Result<&'static crate::plugins::Loaded, relay_bus::BusError> {
    crate::plugins::get(id).ok_or_else(|| {
        relay_bus::BusError::not_found("plugin.not_found", format!("no bundled plugin {id:?}"))
    })
}

/// Repositories commonly publish the same skill under both `.agents/skills` and
/// provider-specific folders. Treat byte-identical copies as one install. Two different
/// instruction bodies with the same public name remain an explicit conflict.
fn deduplicate_downloaded_skills(items: Vec<DownloadedSkill>) -> Result<Vec<DownloadedSkill>, relay_bus::BusError> {
    let mut unique = BTreeMap::<String, DownloadedSkill>::new();
    for item in items {
        let key = item.name.trim().to_lowercase();
        if let Some(existing) = unique.get(&key) {
            if existing.body == item.body {
                continue;
            }
            let existing_rank = skill_path_rank(&existing.source_path);
            let incoming_rank = skill_path_rank(&item.source_path);
            if incoming_rank < existing_rank {
                unique.insert(key, item);
                continue;
            }
            if incoming_rank > existing_rank {
                continue;
            }
            return Err(relay_bus::BusError::conflict(
                "skill.duplicate_name",
                format!("the repository contains different SKILL.md files named {:?}", item.name),
            ).with_details(json!({"paths":[existing.source_path,item.source_path]}))
             .with_hint("install one of the folders directly so Relay knows which instructions to use"));
        }
        unique.insert(key, item);
    }
    Ok(unique.into_values().collect())
}

/// The ref an install clones: the one asked for, else the one a `tree/<ref>` URL names, else
/// the ref the installed skill at `subdir` came from (see [`recorded_ref`]). `None` clones the
/// default branch.
fn install_ref(ctx: &crate::engine::Unlocked, payload: &SkillInstallIn) -> Result<Option<String>, relay_bus::BusError> {
    let (source_url, url_ref, _) = crate::github::parse_github_url(&payload.url)?;
    let asked = payload.source_ref.as_deref().map(str::trim).filter(|value| !value.is_empty());
    if let Some(reference) = asked.map(str::to_string).or(url_ref) {
        return Ok(Some(reference));
    }
    let Some(path) = payload.subdir.as_deref().map(str::trim).filter(|value| !value.is_empty()) else { return Ok(None) };
    ctx.read(|conn| recorded_ref(conn, &source_url, path))
}

/// "Update from GitHub" sends a skill's stored URL and `source_path` and no ref; without this
/// the refresh would quietly switch a skill installed from a branch to the default branch. Only
/// an unambiguous answer is inherited: when that folder is installed from the default branch
/// too, or from two refs, the request means the default branch, as it says.
fn recorded_ref(conn: &Connection, source_url: &str, source_path: &str) -> Result<Option<String>, relay_bus::BusError> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT source_ref FROM skills WHERE source_url=?1 AND source_path=?2 AND deleted_at IS NULL",
    ).bus()?;
    let refs = stmt.query_map(params![source_url, source_path], |row| row.get::<_, Option<String>>(0)).bus()?
        .collect::<rusqlite::Result<Vec<_>>>().bus()?;
    Ok(match refs.as_slice() { [only] => only.clone(), _ => None })
}

fn skill_path_rank(path: &str) -> u8 {
    if path.starts_with("skills/") { 0 }
    else if path.starts_with(".agents/skills/") { 1 }
    else if path.split('/').any(|part| part.starts_with('.')) { 3 }
    else { 2 }
}

#[derive(Debug)]
struct SkillIdentity {
    id: Id,
    name: String,
    source_url: Option<String>,
    source_path: Option<String>,
    source_ref: Option<String>,
    revision: Option<String>,
    deleted_at: Option<String>,
}

fn skill_identity(row: &Row) -> rusqlite::Result<SkillIdentity> {
    Ok(SkillIdentity {
        id: row.get("id")?,
        name: row.get("name")?,
        source_url: row.get("source_url")?,
        source_path: row.get("source_path")?,
        source_ref: row.get("source_ref")?,
        revision: row.get("revision")?,
        deleted_at: row.get("deleted_at")?,
    })
}

/// Install one downloaded skill without making invisible soft-deleted rows look like live
/// conflicts. A genuine live name collision requires permission for that exact row id. Only the
/// row is written: its folder is the caller's to adopt once the transaction commits.
fn install_downloaded_skill(
    conn: &Connection,
    now: &str,
    item: DownloadedSkill,
    replace_skill_id: Option<Id>,
) -> Result<Skill, relay_bus::BusError> {
    let name = valid_skill_name(&item.name)?;
    valid_skill_body(&item.body)?;
    let source_match: Option<SkillIdentity> = conn
        .query_row(
            // A branch is part of the source: the same folder from another ref is another skill.
            "SELECT * FROM skills WHERE source_url=?1 AND source_path=?2 AND source_ref IS ?3",
            params![item.source_url, item.source_path, item.source_ref],
            skill_identity,
        )
        .optional()
        .bus()?;
    let name_match: Option<SkillIdentity> = conn
        .query_row(
            "SELECT * FROM skills WHERE name=?1 COLLATE NOCASE",
            [&name],
            skill_identity,
        )
        .optional()
        .bus()?;

    let source_id = source_match.as_ref().map(|skill| skill.id);
    let target_id = match name_match.as_ref() {
        Some(named) if Some(named.id) == source_id => Some(named.id),
        // Removed skills are absent from skill.list, so reclaim the row instead of reporting a
        // conflict the user cannot see. The same id keeps audit history and project enables.
        Some(named) if named.deleted_at.is_some() && source_match.is_none() => Some(named.id),
        Some(named)
            if named.deleted_at.is_none()
                && replace_skill_id == Some(named.id)
                && source_match.is_none() =>
        {
            Some(named.id)
        }
        Some(named) => {
            let source = named.source_url.as_deref().unwrap_or("a local skill");
            let mut error = relay_bus::BusError::conflict(
                "skill.name_exists",
                format!(
                    "GitHub skill {name:?} conflicts with visible installed skill {} from {source}",
                    named.id
                ),
            )
            .with_details(json!({
                "installed_skill": {
                    "id": named.id,
                    "name": named.name,
                    "source_url": named.source_url,
                    "source_path": named.source_path,
                    "source_ref": named.source_ref,
                    "revision": named.revision,
                    "visible": named.deleted_at.is_none()
                },
                "incoming_skill": {
                    "name": name,
                    "source_url": item.source_url,
                    "source_path": item.source_path,
                    "source_ref": item.source_ref,
                    "revision": item.revision
                },
                "can_replace": named.deleted_at.is_none() && source_match.is_none()
            }));
            error = if source_match.is_some() {
                error.with_hint("the source and name belong to different installed rows; remove one explicitly before refreshing")
            } else {
                error.with_hint("refresh the installed skill, install a narrower folder, or replace it explicitly")
            };
            return Err(error);
        }
        None => source_id,
    };

    let id = if let Some(id) = target_id {
        conn.execute(
            "UPDATE skills SET name=?1,body=?2,source_url=?3,source_path=?4,source_ref=?5,revision=?6,deleted_at=NULL,updated_at=?7 WHERE id=?8",
            params![name,item.body,item.source_url,item.source_path,item.source_ref,item.revision,now,id],
        ).bus()?;
        id
    } else {
        conn.execute(
            "INSERT INTO skills(name,body,source_url,source_path,source_ref,revision,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
            params![name,item.body,item.source_url,item.source_path,item.source_ref,item.revision,now],
        ).bus()?;
        conn.last_insert_rowid()
    };
    enable_everywhere(conn, id)?;
    get_skill(conn, id)
}

/// A skill belongs to the app, not to one project (D147): a row nobody has enabled anywhere
/// is enabled in every project that exists, so installing it once covers the workspaces that
/// are already there. An explicit per-project switch is still honoured — this only fills in
/// the empty set.
fn enable_everywhere(conn: &Connection, id: Id) -> Result<(), relay_bus::BusError> {
    let edges: i64 = conn
        .query_row("SELECT COUNT(*) FROM skill_projects WHERE skill_id=?1", [id], |row| row.get(0))
        .bus()?;
    if edges == 0 {
        conn.execute(
            "INSERT OR IGNORE INTO skill_projects(skill_id,project_id) SELECT ?1,id FROM projects",
            [id],
        ).bus()?;
    }
    Ok(())
}

/// Push the new skill set into every project root and session worktree once the transaction
/// is committed, off the request thread: existing checkouts pick it up without relaunching.
fn refresh_checkouts(ctx: &mut Ctx) {
    ctx.after_commit(|engine| {
        std::thread::Builder::new()
            .name("skill-materialize".into())
            .spawn(move || crate::skills::refresh_all(&engine))
            .ok();
    });
}

fn skill_row(conn: &rusqlite::Connection, row: &Row) -> rusqlite::Result<Skill> {
    let id: Id = row.get("id")?;
    let mut stmt = conn
        .prepare_cached("SELECT project_id FROM skill_projects WHERE skill_id=?1 ORDER BY project_id")?;
    let enabled_in = stmt
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Skill {
        id,
        name: row.get("name")?,
        body: row.get("body")?,
        source_url: row.get("source_url")?,
        source_path: row.get("source_path")?,
        source_ref: row.get("source_ref")?,
        revision: row.get("revision")?,
        enabled_in,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// How much of a body a summary keeps: the frontmatter and the opening lines, enough to
/// describe a skill without every body of a large library riding on one reply.
const SKILL_SUMMARY_BYTES: usize = 4096;

fn summarize(skill: &mut Skill) {
    if skill.body.len() > SKILL_SUMMARY_BYTES {
        let mut end = SKILL_SUMMARY_BYTES;
        while !skill.body.is_char_boundary(end) {
            end -= 1;
        }
        skill.body.truncate(end);
    }
}

fn get_skill(conn: &rusqlite::Connection, id: Id) -> Result<Skill, relay_bus::BusError> {
    conn.query_row(
        "SELECT * FROM skills WHERE id=?1 AND deleted_at IS NULL",
        [id],
        |row| skill_row(conn, row),
    )
    .optional()
    .bus()?
    .ok_or_else(|| relay_bus::BusError::not_found("skill.not_found", format!("no skill {id}")))
}

fn valid_skill_name(name: &str) -> Result<String, relay_bus::BusError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(relay_bus::BusError::invalid(
            "skill.name",
            "skill name must be 1..=120 characters",
        ));
    }
    Ok(name.to_string())
}

fn valid_skill_body(body: &str) -> Result<(), relay_bus::BusError> {
    if body.len() > 256 * 1024 {
        return Err(relay_bus::BusError::invalid(
            "skill.body",
            "skill body exceeds 256 KiB",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn downloaded(name: &str, path: &str, revision: &str) -> DownloadedSkill {
        DownloadedSkill {
            name: name.into(),
            body: format!("---\nname: {name}\n---\nCurrent instructions.\n"),
            source_url: "https://github.com/example/skills.git".into(),
            source_path: path.into(),
            source_ref: None,
            revision: revision.into(),
            assets: None,
        }
    }

    #[test]
    fn install_reclaims_hidden_name_and_requires_exact_live_replacement() {
        let store = Store::open_memory().unwrap();
        let hidden_id = store.with_tx(|tx| {
            tx.execute(
                "INSERT INTO skills(name,body,created_at,updated_at,deleted_at) VALUES ('ponytail','old','t0','t1','t1')",
                [],
            )?;
            Ok(tx.last_insert_rowid())
        }).unwrap();

        let restored = store
            .with_tx(|tx| {
                Ok(install_downloaded_skill(
                    tx,
                    "t2",
                    downloaded("ponytail", "ponytail/SKILL.md", "abc"),
                    None,
                )?)
            })
            .unwrap();
        assert_eq!(
            restored.id, hidden_id,
            "an invisible deleted row is reused instead of conflicting"
        );
        assert_eq!(restored.source_path.as_deref(), Some("ponytail/SKILL.md"));
        assert_eq!(restored.revision.as_deref(), Some("abc"));

        let error = store
            .with_tx(|tx| {
                Ok(install_downloaded_skill(
                    tx,
                    "t3",
                    downloaded("ponytail", "replacement/SKILL.md", "def"),
                    None,
                )
                .unwrap_err())
            })
            .unwrap();
        assert_eq!(error.code, "skill.name_exists");
        assert_eq!(
            error.details.as_ref().unwrap()["installed_skill"]["id"],
            hidden_id
        );
        assert_eq!(error.details.as_ref().unwrap()["can_replace"], true);

        let replaced = store
            .with_tx(|tx| {
                Ok(install_downloaded_skill(
                    tx,
                    "t4",
                    downloaded("ponytail", "replacement/SKILL.md", "def"),
                    Some(hidden_id),
                )?)
            })
            .unwrap();
        assert_eq!(
            replaced.id, hidden_id,
            "replacement keeps project bindings and history on the same row"
        );
        assert_eq!(
            replaced.source_path.as_deref(),
            Some("replacement/SKILL.md")
        );
        assert_eq!(replaced.revision.as_deref(), Some("def"));
    }

    #[test]
    fn a_ref_is_part_of_a_github_skill_source_and_a_refresh_keeps_it() {
        let store = Store::open_memory().unwrap();
        let url = "https://github.com/example/skills.git";
        let path = "skills/x/SKILL.md";
        let from = |name: &str, reference: Option<&str>, revision: &str| {
            let mut item = downloaded(name, path, revision);
            item.source_ref = reference.map(str::to_string);
            item
        };
        let install = |item: DownloadedSkill| store.with_tx(|tx| Ok(install_downloaded_skill(tx, "t", item, None)));
        let recorded = || store.with_tx(|tx| Ok(recorded_ref(tx, url, path))).unwrap().unwrap();

        let dev = install(from("x", Some("dev"), "a")).unwrap().unwrap();
        assert_eq!(dev.source_ref.as_deref(), Some("dev"));
        assert_eq!(recorded(), Some("dev".into()), "Update from GitHub stays on the branch the skill came from");
        let refreshed = install(from("x", Some("dev"), "b")).unwrap().unwrap();
        assert_eq!((refreshed.id, refreshed.revision.as_deref()), (dev.id, Some("b")));

        // The default branch's copy of the folder no longer overwrites the dev row in place.
        let error = install(from("x", None, "c")).unwrap().unwrap_err();
        assert_eq!(error.code, "skill.name_exists");
        assert_eq!(error.details.as_ref().unwrap()["installed_skill"]["source_ref"], "dev");
        // Under another name both sources are kept, and a refresh without a ref is the default branch.
        let main = install(from("x-main", None, "c")).unwrap().unwrap();
        assert_ne!(main.id, dev.id);
        assert_eq!(main.source_ref, None);
        assert_eq!(recorded(), None);
        let twice = store.with_tx(|tx| {
            Ok(tx.execute(
                "INSERT INTO skills(name,body,source_url,source_path,created_at,updated_at) VALUES ('y','b',?1,?2,'t','t')",
                [url, path],
            ).is_err())
        });
        assert!(twice.unwrap(), "one folder from the default branch is still one row");
    }

    #[test]
    fn create_over_a_removed_github_skill_starts_a_local_one_and_undo_restores_it_whole() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("store/store.db"), false).unwrap();
        let engine = crate::engine::Engine::new(crate::Instance::Test, store);
        let create = |name: &str, body: &str| {
            let request = relay_bus::Request::new(relay_bus::Actor::User, "skill.create", json!({"name":name,"body":body}));
            engine.dispatch(request, crate::engine::Door::InProcess).into_result().unwrap()
        };
        let removed = |name: &str, body: &str| {
            let id = engine.store.with_tx(|tx| {
                tx.execute(
                    "INSERT INTO skills(name,body,source_url,source_path,revision,created_at,updated_at,deleted_at)
                     VALUES (?1,?2,'https://github.com/example/skills.git',?1,'abc','t','t','t')",
                    [name, body],
                )?;
                Ok(tx.last_insert_rowid())
            }).unwrap();
            let library = crate::skills::library_dir(&engine.store, id);
            std::fs::create_dir_all(&library).unwrap();
            std::fs::write(library.join("SKILL.md"), body).unwrap();
            (id, library)
        };

        let (id, library) = removed("ponytail", "from GitHub");
        let mine = create("ponytail", "my own text");
        assert_eq!(mine["id"], id);
        assert_eq!(mine["source_url"], serde_json::Value::Null, "Update from GitHub would overwrite this text");
        assert_eq!(mine["revision"], serde_json::Value::Null);
        assert!(!library.exists(), "the removed skill's folder rode along to the new one");

        // What `skill.delete` records as its undo: the same name and the exact body it removed.
        let (id, library) = removed("braid", "from GitHub");
        let restored = create("braid", "from GitHub");
        assert_eq!(restored["id"], id);
        assert_eq!(restored["source_url"], "https://github.com/example/skills.git");
        assert!(library.join("SKILL.md").is_file());
    }

    #[test]
    fn identical_provider_copies_install_once_but_different_bodies_conflict() {
        let first = downloaded("Review", ".agents/skills/review/SKILL.md", "abc");
        let second = downloaded("Review", ".claude/skills/review/SKILL.md", "abc");
        let unique = deduplicate_downloaded_skills(vec![first, second]).unwrap();
        assert_eq!(unique.len(), 1);

        let first = downloaded("Review", ".claude/skills/review/SKILL.md", "abc");
        let mut second = downloaded("review", ".cursor/skills/review/SKILL.md", "abc");
        second.body.push_str("Provider-specific instructions.\n");
        let error = deduplicate_downloaded_skills(vec![first, second]).unwrap_err();
        assert_eq!(error.code, "skill.duplicate_name");
    }

    #[test]
    fn canonical_skills_folder_wins_over_provider_adapter() {
        let mut adapter = downloaded("Ponytail", ".openclaw/skills/ponytail/SKILL.md", "abc");
        adapter.body.push_str("OpenClaw adapter.\n");
        let canonical = downloaded("Ponytail", "skills/ponytail/SKILL.md", "abc");
        let unique = deduplicate_downloaded_skills(vec![adapter, canonical]).unwrap();
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].source_path, "skills/ponytail/SKILL.md");
    }

    #[test]
    fn relay_agents_skill_wins_over_other_hidden_adapters() {
        let mut generic_agent = downloaded("Impeccable", ".agent/skills/impeccable/SKILL.md", "abc");
        generic_agent.body.push_str("Generic agent commands.\n");
        let relay_agents = downloaded("Impeccable", ".agents/skills/impeccable/SKILL.md", "abc");
        let unique = deduplicate_downloaded_skills(vec![generic_agent, relay_agents]).unwrap();
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].source_path, ".agents/skills/impeccable/SKILL.md");
    }
}
