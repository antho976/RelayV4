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
        let project_id = project.id;
        ctx.after_commit(move |engine| enqueue(engine, project_id, id, parent));
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
    // Removing a checkout walks and deletes its build output: seconds to minutes for a large
    // one, so it happens before the transaction opens (D149), which then only records it.
    e.register_staged::<IntegrationDiscard, Option<i64>>(|ctx, p| {
        let (integration, project, in_use) = ctx.read(|conn| {
            let integration = get(conn, p.integration_id)?;
            let project = get_project(conn, integration.project_id)?;
            let in_use = match &integration.worktree {
                Some(path) => runs_in(conn, path)?,
                None => 0,
            };
            Ok((integration, project, in_use))
        })?;
        if in_use > 0 {
            return Err(BusError::conflict("integration.in_use", "a device run is using this integration's checkout")
                .with_hint("stop the run, then discard"));
        }
        if let Some(path) = &integration.worktree {
            remove_checkout(Path::new(&project.path), Path::new(path), integration.id)
                .map_err(|e| BusError::conflict("integration.discard_failed", e.to_string()))?;
        }
        Ok(Some(integration.project_id))
    }, |ctx: &mut Ctx, p, project_id| {
        ctx.tx().execute("UPDATE integrations SET state='discarded',finished_at=COALESCE(finished_at,?1) WHERE id=?2", params![ctx.now,p.integration_id]).bus()?;
        if let Some(project_id) = project_id { ctx.set_project(project_id); }
        ctx.emit("integration.changed", json!({"integration_id": p.integration_id,"state":"discarded"}));
        Ok(Empty {})
    });
}

/// How many finished integrations per project keep their checkout for inspection or a device
/// run (D38). Older ones are discarded as each new one finishes.
const KEEP_FINISHED: usize = 3;

/// Integrations waiting for their project's runner, keyed by engine and project. A key is
/// present while a runner thread is working through that project: one integration at a time
/// per project, so repeated "Test together" clicks queue rather than start concurrent
/// checkouts and builds. The runner holds the engine, so its address cannot be reused while
/// the key exists.
type Queues = std::collections::HashMap<(usize, i64), std::collections::VecDeque<(i64, uuid::Uuid)>>;
static QUEUES: std::sync::Mutex<Option<Queues>> = std::sync::Mutex::new(None);

fn enqueue(engine: std::sync::Arc<Engine>, project_id: i64, id: i64, parent: uuid::Uuid) {
    let key = (std::sync::Arc::as_ptr(&engine) as usize, project_id);
    {
        let mut queues = QUEUES.lock().unwrap_or_else(|p| p.into_inner());
        let queues = queues.get_or_insert_with(Default::default);
        if let Some(waiting) = queues.get_mut(&key) {
            waiting.push_back((id, parent));
            return;
        }
        queues.insert(key, Default::default());
    }
    let spawned = std::thread::Builder::new().name(format!("integration-{project_id}")).spawn(move || {
        let mut next = Some((id, parent));
        while let Some((id, parent)) = next {
            run(engine.clone(), id, parent);
            prune(&engine, project_id);
            let mut queues = QUEUES.lock().unwrap_or_else(|p| p.into_inner());
            let queues = queues.get_or_insert_with(Default::default);
            next = queues.get_mut(&key).and_then(|waiting| waiting.pop_front());
            if next.is_none() {
                queues.remove(&key);
            }
        }
    });
    if spawned.is_err() {
        if let Some(queues) = QUEUES.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            queues.remove(&key);
        }
    }
}

/// Discard every finished integration of `project_id` past the newest [`KEEP_FINISHED`] that
/// still has a checkout, unless a device run is using it. Runs on the integration thread.
fn prune(engine: &Engine, project_id: i64) {
    let doomed: Vec<(i64, String, String)> = {
        let conn = engine.store.lock();
        let rows = conn.prepare_cached(
            "SELECT i.id, i.worktree, p.path FROM integrations i JOIN projects p ON p.id=i.project_id
             WHERE i.project_id=?1 AND i.state IN ('passed','failed','conflict') AND i.worktree IS NOT NULL
             ORDER BY i.id DESC LIMIT -1 OFFSET ?2",
        ).and_then(|mut stmt| stmt.query_map(params![project_id, KEEP_FINISHED as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>());
        match rows {
            Ok(rows) => rows.into_iter().filter(|(_, path, _)| runs_in(&conn, path).is_ok_and(|n| n == 0)).collect(),
            Err(error) => {
                tracing::warn!(project_id, %error, "listing integrations to prune");
                return;
            }
        }
    };
    for (id, path, repo) in doomed {
        if let Err(error) = remove_checkout(Path::new(&repo), Path::new(&path), id) {
            tracing::warn!(integration = id, %error, "pruning an old integration");
            continue;
        }
        let _ = engine.system_write("integration.prune", None, Some(project_id), None, json!({"integration_id": id}), |tx, now| {
            tx.execute("UPDATE integrations SET state='discarded',finished_at=COALESCE(finished_at,?1) WHERE id=?2", params![now, id]).bus()?;
            Ok(((), vec![("integration.changed".into(), json!({"integration_id": id, "state": "discarded"}))]))
        });
    }
}

/// Live device runs whose checkout is `path`.
fn runs_in(conn: &rusqlite::Connection, path: &str) -> Result<i64, BusError> {
    conn.prepare_cached("SELECT COUNT(*) FROM device_runs WHERE worktree=?1 AND state IN ('building','running')").bus()?
        .query_row([path], |r| r.get(0)).bus()
}

/// An integration's checkout, its build output and its `relay/integration-<id>` branch.
pub(crate) fn remove_checkout(repo: &Path, path: &Path, id: i64) -> anyhow::Result<()> {
    if path.exists() {
        worktree::remove(repo, path, true)?;
    }
    let _ = worktree::git_mutate(repo, &["branch", "-D", &format!("relay/integration-{id}")]);
    Ok(())
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
