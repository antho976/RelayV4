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
    e.register::<Refresh>(|ctx: &mut Ctx, _| {
        let discovered = crate::providers::refresh(ctx.tx(), &ctx.now)?;
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
    });
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
        Ok(SkillListOut { skills })
    });
    e.register::<SkillCreate>(|ctx: &mut Ctx, payload| {
        let name = valid_skill_name(&payload.name)?;
        valid_skill_body(&payload.body)?;
        let existing: Option<(Id, Option<String>)> = ctx
            .tx()
            .query_row(
                "SELECT id,deleted_at FROM skills WHERE name=?1 COLLATE NOCASE",
                [&name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .bus()?;
        let id =
            match existing {
                Some((id, Some(_))) => {
                    ctx.tx().execute(
                    "UPDATE skills SET name=?1,body=?2,deleted_at=NULL,updated_at=?3 WHERE id=?4",
                    params![name,payload.body,ctx.now,id],
                ).bus()?;
                    id
                }
                Some((id, None)) => {
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
            let staging = ctx.engine().store.skills_dir().join(".staging").join(uuid::Uuid::new_v4().to_string());
            let downloaded = crate::github::download_skills(&payload.url, payload.subdir.as_deref(), Some(&staging))
                .and_then(deduplicate_downloaded_skills);
            match downloaded {
                Ok(items) => Ok((staging, items)),
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&staging);
                    Err(error)
                }
            }
        },
        |ctx: &mut Ctx, payload, (staging, downloaded)| {
        let mut skills = Vec::with_capacity(downloaded.len());
        for item in downloaded {
            let skill = install_downloaded_skill(
                ctx.tx(), &ctx.engine().store, &ctx.now, item, payload.replace_skill_id,
            );
            let skill = match skill {
                Ok(skill) => skill,
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&staging);
                    return Err(error);
                }
            };
            ctx.emit("skill.changed", serde_json::to_value(&skill).bus()?);
            skills.push(skill);
        }
        let _ = std::fs::remove_dir_all(&staging);
        refresh_checkouts(ctx);
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
    revision: Option<String>,
    deleted_at: Option<String>,
}

fn skill_identity(row: &Row) -> rusqlite::Result<SkillIdentity> {
    Ok(SkillIdentity {
        id: row.get("id")?,
        name: row.get("name")?,
        source_url: row.get("source_url")?,
        source_path: row.get("source_path")?,
        revision: row.get("revision")?,
        deleted_at: row.get("deleted_at")?,
    })
}

/// Install one downloaded skill without making invisible soft-deleted rows look like live
/// conflicts. A genuine live name collision requires permission for that exact row id.
fn install_downloaded_skill(
    conn: &Connection,
    store: &crate::Store,
    now: &str,
    item: DownloadedSkill,
    replace_skill_id: Option<Id>,
) -> Result<Skill, relay_bus::BusError> {
    let name = valid_skill_name(&item.name)?;
    valid_skill_body(&item.body)?;
    let source_match: Option<SkillIdentity> = conn
        .query_row(
            "SELECT * FROM skills WHERE source_url=?1 AND source_path=?2",
            params![item.source_url, item.source_path],
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
                    "revision": named.revision,
                    "visible": named.deleted_at.is_none()
                },
                "incoming_skill": {
                    "name": name,
                    "source_url": item.source_url,
                    "source_path": item.source_path,
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
            "UPDATE skills SET name=?1,body=?2,source_url=?3,source_path=?4,revision=?5,deleted_at=NULL,updated_at=?6 WHERE id=?7",
            params![name,item.body,item.source_url,item.source_path,item.revision,now,id],
        ).bus()?;
        id
    } else {
        conn.execute(
            "INSERT INTO skills(name,body,source_url,source_path,revision,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![name,item.body,item.source_url,item.source_path,item.revision,now],
        ).bus()?;
        conn.last_insert_rowid()
    };
    if let Some(assets) = item.assets.as_deref() {
        if let Err(error) = crate::skills::adopt(assets, &crate::skills::library_dir(store, id)) {
            tracing::warn!(skill = id, error = %error, "storing skill folder");
        }
    }
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
        revision: row.get("revision")?,
        enabled_in,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
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
                    &store,
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
                    &store,
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
                    &store,
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
