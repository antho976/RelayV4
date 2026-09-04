//! `integration.*` (phase 8): durable, request-driven throwaway merge/build worktrees.

use crate::engine::{Ctx, Engine, IntoBus};
use crate::handlers::workspace::get_project;
use crate::worktree;
use relay_bus::error::BusError;
use relay_bus::ops::git::*;
use relay_bus::types::{Integration, IntegrationState};
use relay_bus::Empty;
use rusqlite::{params, OptionalExtension, Row};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn register(e: &mut Engine) {
    e.register::<IntegrationRequest>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let mut branches = p.branches.unwrap_or_default();
        for session in p.sessions.unwrap_or_default() {
            let branch: Option<String> = ctx.tx().query_row(
                "SELECT branch FROM sessions WHERE project_id=?1 AND name=?2 AND state!='closed'",
                params![project.id, session], |r| r.get(0),
            ).optional().bus()?;
            branches.push(branch.ok_or_else(|| BusError::not_found("integration.session_not_found", format!("no open session {session}")))?);
        }
        branches.retain(|b| !b.trim().is_empty()); branches.sort(); branches.dedup();
        if branches.len() < 2 { return Err(BusError::invalid("integration.branches", "select at least two distinct session branches")); }
        let repo = gix::open(&project.path).map_err(|e| BusError::unavailable("git.open_failed", e.to_string()))?;
        for branch in &branches { repo.rev_parse_single(branch.as_str()).map_err(|e| BusError::invalid("integration.branch", format!("{branch}: {e}")))?; }
        ctx.tx().execute(
            "INSERT INTO integrations(project_id,branches,state,build,deploy,created_at) VALUES (?1,?2,'queued',?3,?4,?5)",
            params![project.id, serde_json::to_string(&branches).bus()?, p.build.unwrap_or(true) as i64, p.deploy, ctx.now],
        ).bus()?;
        let id = ctx.tx().last_insert_rowid();
        let integration = get(ctx.tx(), id)?;
        let parent = ctx.req_id;
        ctx.set_project(project.id);
        ctx.emit("integration.changed", serde_json::to_value(&integration).bus()?);
        ctx.after_commit(move |engine| {
            std::thread::spawn(move || run(engine, id, parent));
        });
        Ok(integration)
    });
    e.register::<IntegrationGet>(|ctx, p| get(ctx.tx(), p.integration_id));
    e.register::<IntegrationList>(|ctx, p| {
        get_project(ctx.tx(), p.project_id)?;
        let mut st = ctx
            .tx()
            .prepare_cached("SELECT * FROM integrations WHERE project_id=?1 ORDER BY id DESC LIMIT 100")
            .bus()?;
        let rows = st
            .query_map([p.project_id], row)
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        Ok(IntegrationListOut { integrations: rows })
    });
    e.register::<IntegrationDiscard>(|ctx: &mut Ctx, p| {
        let integration = get(ctx.tx(), p.integration_id)?;
        if let Some(path) = &integration.worktree {
            let project = get_project(ctx.tx(), integration.project_id)?;
            worktree::remove(Path::new(&project.path), Path::new(path), true)
                .map_err(|e| BusError::conflict("integration.discard_failed", e.to_string()))?;
            let _ = worktree::git_mutate(Path::new(&project.path), &["branch", "-D", &format!("relay/integration-{}", integration.id)]);
        }
        ctx.tx().execute("UPDATE integrations SET state='discarded',finished_at=COALESCE(finished_at,?1) WHERE id=?2", params![ctx.now,p.integration_id]).bus()?;
        ctx.set_project(integration.project_id); ctx.emit("integration.changed", json!({"integration_id": integration.id,"state":"discarded"}));
        Ok(Empty {})
    });
}

fn run(engine: std::sync::Arc<Engine>, id: i64, parent: uuid::Uuid) {
    let loaded = {
        let conn = engine.store.lock();
        let integration = get(&conn, id);
        let project = integration
            .as_ref()
            .ok()
            .and_then(|i| get_project(&conn, i.project_id).ok());
        integration.ok().zip(project)
    };
    let Some((integration, project)) = loaded else {
        return;
    };
    let repo = PathBuf::from(&project.path);
    let path = repo
        .join(".relay")
        .join("integrations")
        .join(id.to_string());
    let branch = format!("relay/integration-{id}");
    let _ = set_state(
        &engine,
        id,
        parent,
        IntegrationState::Merging,
        Some(&path),
        "Preparing integration worktree",
        false,
        None,
    );
    let created = worktree::create(&repo, &path, &branch, Some(&project.base_branch));
    if let Err(error) = created {
        fail(
            &engine,
            id,
            parent,
            IntegrationState::Failed,
            &format!("worktree: {error}"),
            None,
        );
        return;
    }
    let discarded = {
        let conn = engine.store.lock();
        get(&conn, id)
            .map(|run| run.state == IntegrationState::Discarded)
            .unwrap_or(true)
    };
    if discarded {
        let _ = worktree::remove(&repo, &path, true);
        let _ = worktree::git_mutate(&repo, &["branch", "-D", &branch]);
        return;
    }
    let merge_args: Vec<&str> = std::iter::once("merge")
        .chain(["--no-ff", "--no-edit"])
        .chain(integration.branches.iter().map(String::as_str))
        .collect();
    if let Err(error) = worktree::git_mutate(&path, &merge_args) {
        let pair = integration
            .branches
            .first()
            .cloned()
            .zip(integration.branches.get(1).cloned());
        fail(
            &engine,
            id,
            parent,
            IntegrationState::Conflict,
            &format!("merge: {error}"),
            pair,
        );
        return;
    }
    let build: bool = {
        let conn = engine.store.lock();
        conn.query_row("SELECT build FROM integrations WHERE id=?1", [id], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0)
            != 0
    };
    let mut log = String::from("Merge passed.\n");
    if build {
        if let Some(command) = project.build_cmd.as_deref() {
            let _ = set_state(
                &engine,
                id,
                parent,
                IntegrationState::Building,
                Some(&path),
                &log,
                false,
                None,
            );
            match shell(&path, command, None) {
                Ok(text) => log.push_str(&text),
                Err(text) => {
                    log.push_str(&text);
                    fail(&engine, id, parent, IntegrationState::Failed, &log, None);
                    return;
                }
            }
        } else {
            log.push_str("No build command configured.\n");
        }
    }
    let deploy: Option<String> = {
        let conn = engine.store.lock();
        conn.query_row("SELECT deploy FROM integrations WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .ok()
        .flatten()
    };
    if let Some(device) = deploy {
        let Some(command) = project.run_cmd.as_deref() else {
            fail(
                &engine,
                id,
                parent,
                IntegrationState::Failed,
                "Deploy requested but no run command is configured.",
                None,
            );
            return;
        };
        let _ = set_state(
            &engine,
            id,
            parent,
            IntegrationState::Deploying,
            Some(&path),
            &log,
            false,
            None,
        );
        match shell(&path, command, Some(&device)) {
            Ok(text) => log.push_str(&text),
            Err(text) => {
                log.push_str(&text);
                fail(&engine, id, parent, IntegrationState::Failed, &log, None);
                return;
            }
        }
    }
    let _ = set_state(
        &engine,
        id,
        parent,
        IntegrationState::Passed,
        Some(&path),
        &log,
        true,
        None,
    );
}

fn shell(cwd: &Path, command: &str, device: Option<&str>) -> Result<String, String> {
    let mut cmd = Command::new("fish");
    cmd.current_dir(cwd).args(["-lc", command]);
    if let Some(device) = device {
        cmd.env("RELAY_DEVICE", device);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

fn fail(
    engine: &Engine,
    id: i64,
    parent: uuid::Uuid,
    state: IntegrationState,
    log: &str,
    conflict: Option<(String, String)>,
) {
    let _ = set_state(engine, id, parent, state, None, log, true, conflict);
}

#[allow(clippy::too_many_arguments)] // one transition record; a builder would hide required audit fields
fn set_state(
    engine: &Engine,
    id: i64,
    parent: uuid::Uuid,
    state: IntegrationState,
    path: Option<&Path>,
    log: &str,
    finished: bool,
    conflict: Option<(String, String)>,
) -> Result<(), BusError> {
    {
        let conn = engine.store.lock();
        if get(&conn, id)?.state == IntegrationState::Discarded {
            return Ok(());
        }
    }
    let state_s = state_name(state);
    let tail = tail(log, 12_000);
    let project_id = {
        let conn = engine.store.lock();
        conn.query_row(
            "SELECT project_id FROM integrations WHERE id=?1",
            [id],
            |r| r.get::<_, i64>(0),
        )
        .bus()?
    };
    engine.system_write("integration.advance",Some(parent),Some(project_id),None,json!({"integration_id":id,"state":state_s}),|tx,now|{
        tx.execute("UPDATE integrations SET state=?1,worktree=COALESCE(?2,worktree),log_tail=?3,conflict=?4,started_at=COALESCE(started_at,?5),finished_at=CASE WHEN ?6 THEN ?5 ELSE finished_at END WHERE id=?7",params![state_s,path.map(|p|p.display().to_string()),tail,conflict.as_ref().map(|v|serde_json::to_string(v).unwrap()),now,finished as i64,id]).bus()?;
        if finished { tx.execute("INSERT INTO notifications(project_id,category,title,body,link,read,created_at) VALUES (?1,'integration',?2,?3,NULL,0,?4)",params![project_id,format!("Integration {state_s}"),format!("Integration {id}: {}",tail.lines().last().unwrap_or(state_s)),now]).bus()?; }
        let value=serde_json::to_value(get(tx,id)?).bus()?;let mut events=vec![("integration.changed".into(),value.clone())];if finished{events.push(("integration.result".into(),value));events.push(("notify.new".into(),json!({"category":"integration","project_id":project_id,"integration_id":id})));}Ok(((),events))
    })
}

fn get(conn: &rusqlite::Connection, id: i64) -> Result<Integration, BusError> {
    conn.query_row("SELECT * FROM integrations WHERE id=?1", [id], row)
        .optional()
        .bus()?
        .ok_or_else(|| BusError::not_found("integration.not_found", format!("no integration {id}")))
}
fn row(r: &Row) -> rusqlite::Result<Integration> {
    let state = match r.get::<_, String>("state")?.as_str() {
        "queued" => IntegrationState::Queued,
        "merging" => IntegrationState::Merging,
        "building" => IntegrationState::Building,
        "deploying" => IntegrationState::Deploying,
        "passed" => IntegrationState::Passed,
        "failed" => IntegrationState::Failed,
        "conflict" => IntegrationState::Conflict,
        "discarded" => IntegrationState::Discarded,
        _ => IntegrationState::Failed,
    };
    let conflict = r
        .get::<_, Option<String>>("conflict")?
        .and_then(|v| serde_json::from_str(&v).ok());
    Ok(Integration {
        id: r.get("id")?,
        project_id: r.get("project_id")?,
        branches: serde_json::from_str(&r.get::<_, String>("branches")?).unwrap_or_default(),
        worktree: r.get("worktree")?,
        state,
        conflict,
        log_tail: r.get("log_tail")?,
        started_at: r.get("started_at")?,
        finished_at: r.get("finished_at")?,
    })
}
fn state_name(state: IntegrationState) -> &'static str {
    match state {
        IntegrationState::Queued => "queued",
        IntegrationState::Merging => "merging",
        IntegrationState::Building => "building",
        IntegrationState::Deploying => "deploying",
        IntegrationState::Passed => "passed",
        IntegrationState::Failed => "failed",
        IntegrationState::Conflict => "conflict",
        IntegrationState::Discarded => "discarded",
    }
}
fn tail(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut start = value.len() - max;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    value[start..].to_string()
}
