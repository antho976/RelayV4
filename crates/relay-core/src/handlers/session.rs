//! `session.*` — worktree + PTY lifecycle: create/spawn/resume/park/wake/close,
//! attach/input/resize/scrollback, recovery, PAIR ownership, get/list/update.

use crate::awareness::SESSION_SKILLS_PATH;
use crate::engine::{Ctx, Engine, IntoBus};
use crate::pty::{Pty, SpawnSpec};
use crate::sessions::{self, Row_};
use crate::worktree;
use relay_bus::error::BusError;
use relay_bus::ops::session::*;
use relay_bus::types::{Id, Provider, Role, Session, SessionState};
use relay_bus::Empty;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Where a completion notification points: the task if the session carries one, else the
/// session itself. `session.done` and the Stop-hook fallback must agree, so both call this.
fn done_link(s: &Session) -> String {
    match s.task_id {
        Some(task_id) => json!({"op":"task.get", "payload":{"task_id":task_id}}).to_string(),
        None => json!({"op":"session.get", "payload":{"session":s.name}}).to_string(),
    }
}

fn next_queued_task(
    conn: &Connection,
    session_id: Id,
    completed_task_id: Id,
) -> Result<Option<(Id, Option<Id>)>, BusError> {
    conn.query_row(
        "SELECT t.id,t.module_id
         FROM task_sessions ts JOIN tasks t ON t.id=ts.task_id
         WHERE ts.session_id=?1 AND t.id!=?2 AND t.deleted_at IS NULL AND t.col='active' AND ts.completed_at IS NULL
         ORDER BY ts.queue_ord,t.position,t.id LIMIT 1",
        params![session_id, completed_task_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .bus()
}

fn assignment_text(task_id: Id, review: bool) -> String {
    if review {
        format!("All builders finished Task #{task_id}. Review the current task in the shared worktree; report your review outcome before the group advances.")
    } else {
        format!("Your current assignment is Task #{task_id}. Read session.bootstrap for the task and group context before continuing.")
    }
}

pub(crate) fn announce_assignment(ctx: &mut Ctx, session: &Session, task_id: Id, review: bool) -> Result<(), BusError> {
    let text = assignment_text(task_id, review);
    if let Some(message) = crate::handlers::notes::send_system(ctx.tx(), session.project_id, &session.name, &text, Some(task_id), &ctx.now)? {
        ctx.emit("mailbox.new", serde_json::to_value(message).bus()?);
    }
    deliver_assignment(ctx, session, task_id, review)
}

fn deliver_assignment(ctx: &mut Ctx, session: &Session, task_id: Id, review: bool) -> Result<(), BusError> {
    // Done runs inside the provider's old turn. Its trailing Stop is the safe handoff
    // edge; hookless providers retain the assignment in durable mail instead.
    let awaiting_stop: bool = ctx.tx().query_row(
        "SELECT done_pending_stop IS NOT NULL FROM sessions WHERE id=?1", [session.id], |row| row.get(0),
    ).bus()?;
    if awaiting_stop { return Ok(()); }
    let text = assignment_text(task_id, review);
    let (id, project) = (session.id, session.project_id);
    ctx.after_commit(move |engine| {
        let Some(pty) = engine.pty(id) else { return; };
        if !pty.claim_idle_edge() { return; }
        let changed = engine.system_write("session.assignment.ready", None, Some(project), Some(id), json!({"task_id":task_id}), |tx, now| {
            let changed = tx.execute("UPDATE sessions SET state='running',updated_at=?1 WHERE id=?2 AND task_id=?3 AND state='idle' AND done_pending_stop IS NULL", params![now,id,task_id]).bus()?;
            let mut events = Vec::new();
            if changed > 0 {
                if let Some(row) = sessions::by_id(tx,id)? { events.push(("session.changed".into(),serde_json::to_value(row.session).bus()?)); }
            }
            Ok((changed > 0, events))
        });
        if matches!(changed, Ok(true)) {
            if let Err(error) = pty.write(format!("{text}\r").as_bytes()) {
                tracing::warn!(session_id=id, %error, "assignment remains available in Relay mailbox");
            }
        }
    });
    Ok(())
}

fn complete_group_assignment(ctx: &mut Ctx, session: &Session, task_id: Id, sha: Option<&str>) -> Result<(), BusError> {
    if session.role == Role::Reviewer {
        let ready: bool = ctx.tx().query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND col IN ('in_review','done') AND deleted_at IS NULL)",[task_id],|row|row.get(0)).bus()?;
        if !ready { return Err(BusError::conflict("session.review_not_ready", "Builders must finish the current task before its review can complete")); }
    }
    ctx.tx().execute("UPDATE task_sessions SET completed_at=COALESCE(completed_at,?1) WHERE task_id=?2 AND session_id=?3",params![ctx.now,task_id,session.id]).bus()?;
    if let Some(sha) = sha.filter(|sha| !sha.trim().is_empty()) {
        ctx.tx().execute("INSERT OR IGNORE INTO task_commits(task_id,sha,branch,linked_at) VALUES (?1,?2,?3,?4)",params![task_id,sha,session.branch,ctx.now]).bus()?;
    }
    let builders_left: i64 = ctx.tx().query_row(
        "SELECT COUNT(*) FROM task_sessions ts JOIN sessions s ON s.id=ts.session_id WHERE ts.task_id=?1 AND s.worktree=?2 AND s.role='builder' AND s.state!='closed' AND ts.completed_at IS NULL",
        params![task_id,session.worktree],|row|row.get(0)).bus()?;
    if builders_left != 0 { return Ok(()); }
    let changed = ctx.tx().execute("UPDATE tasks SET col='in_review',state='awaiting_review',updated_at=?1 WHERE id=?2 AND col='active' AND deleted_at IS NULL",params![ctx.now,task_id]).bus()?;
    let mut stmt = ctx.tx().prepare_cached("SELECT s.id FROM sessions s JOIN task_sessions ts ON ts.session_id=s.id WHERE ts.task_id=?1 AND s.worktree=?2 AND s.state!='closed' ORDER BY s.id").bus()?;
    let ids = stmt.query_map(params![task_id,session.worktree],|row|row.get::<_,Id>(0)).bus()?.collect::<rusqlite::Result<Vec<_>>>().bus()?;
    drop(stmt);
    if changed > 0 {
        ctx.emit("task.changed",json!({"task_id":task_id,"col":"in_review","state":"awaiting_review"}));
        for id in &ids {
            if let Some(row) = sessions::by_id(ctx.tx(),*id)? {
                if row.session.role == Role::Reviewer { announce_assignment(ctx,&row.session,task_id,true)?; }
            }
        }
    }
    let reviewers_left: i64 = ctx.tx().query_row(
        "SELECT COUNT(*) FROM task_sessions ts JOIN sessions s ON s.id=ts.session_id WHERE ts.task_id=?1 AND s.worktree=?2 AND s.role='reviewer' AND s.state!='closed' AND ts.completed_at IS NULL",
        params![task_id,session.worktree],|row|row.get(0)).bus()?;
    if reviewers_left != 0 { return Ok(()); }
    let next = next_queued_task(ctx.tx(),session.id,task_id)?;
    for id in ids {
        let changed = ctx.tx().execute("UPDATE sessions SET task_id=?1,module_id=?2,updated_at=?3 WHERE id=?4 AND task_id=?5",params![next.map(|v|v.0),next.and_then(|v|v.1),ctx.now,id,task_id]).bus()?;
        if changed > 0 {
            if let Some(row) = sessions::by_id(ctx.tx(),id)? {
                emit_session(ctx,&row.session);
                if let Some((next_id,_)) = next { announce_assignment(ctx,&row.session,next_id,false)?; }
            }
        }
    }
    Ok(())

}

fn launch_nudge(row: &Row_) -> Option<String> {
    if let Some(assignment) = row
        .launch_prompt
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        return Some(format!(
            "Call session.bootstrap first, then begin this assignment: {}",
            assignment.trim(),
        ));
    }
    row.session.task_id.map(|task_id| match row.session.role {
        Role::Builder => format!(
            "Call session.bootstrap first, then begin current Task #{task_id}. Work through queued tasks one at a time; session.done completes only the current task."
        ),
        Role::Reviewer => format!(
            "Call session.bootstrap first, then begin the review-group work for current Task #{task_id}. Watch Relay mailbox for file-ready messages from the builders."
        ),
        Role::Docs => format!(
            "Call session.bootstrap first, then begin the documentation work for current Task #{task_id}."
        ),
    })
}

/// Agents act on their own session (or their PAIR partner for reads); the user on any.
fn assert_own(ctx: &Ctx, row: &Row_, reads_ok_for_pair: bool) -> Result<(), BusError> {
    if let Some(sid) = ctx.actor_session_id() {
        if sid == row.session.id {
            return Ok(());
        }
        if reads_ok_for_pair {
            if let Some(pair) = &row.session.pair_with {
                if Some(pair.as_str()) == ctx.actor.session_name() {
                    return Ok(());
                }
            }
        }
        return Err(BusError::not_own("session"));
    }
    if ctx.actor.is_agent() {
        // an agent through the in-process door without a bound session: only its own name
        if ctx.actor.session_name() != Some(row.session.name.as_str()) {
            return Err(BusError::not_own("session"));
        }
    }
    Ok(())
}

/// The project an agent actor is confined to, or `None` for the user.
fn actor_project(ctx: &Ctx) -> Result<Option<Id>, BusError> {
    let Some(sid) = ctx.actor_session_id() else {
        if ctx.actor.is_agent() {
            return Err(BusError::actor(
                "agent actor is not bound to a live session",
            ));
        }
        return Ok(None);
    };
    let own =
        sessions::by_id(ctx.tx(), sid)?.ok_or_else(|| BusError::actor("bound session vanished"))?;
    Ok(Some(own.session.project_id))
}

/// Metadata reads are project-scoped for agents; the private surfaces (scrollback, brief)
/// keep using [`assert_own`].
fn assert_same_project(ctx: &Ctx, row: &Row_) -> Result<(), BusError> {
    match actor_project(ctx)? {
        Some(project_id) if project_id != row.session.project_id => {
            Err(BusError::not_own("project"))
        }
        _ => Ok(()),
    }
}

/// A running PTY knows when its child last spoke even when the provider files no lifecycle
/// report; without this, an unhooked provider reads as a session that has never done anything.
fn overlay_live_output(ctx: &Ctx, sessions: &mut [Session]) {
    for session in sessions {
        if let Some(at) = ctx
            .engine()
            .pty(session.id)
            .and_then(|pty| pty.last_output_at())
        {
            session.last_output_at = Some(at);
        }
    }
}

/// Claims name worktree-relative paths, like every other path on the bus.
fn claim_path(raw: &str) -> Result<String, BusError> {
    let path = Path::new(raw.trim());
    if raw.trim().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(BusError::invalid(
            "session.claim_path",
            format!("{raw:?} must be worktree-relative and contain no .."),
        ));
    }
    Ok(path.display().to_string())
}

fn emit_session(ctx: &mut Ctx, s: &Session) {
    ctx.set_project(s.project_id);
    ctx.set_session(s.id);
    ctx.emit(
        "session.changed",
        serde_json::to_value(s).unwrap_or(Value::Null),
    );
}

fn release_claims(ctx: &mut Ctx, session: &Session) -> Result<(), BusError> {
    ctx.tx().execute("DELETE FROM claims WHERE session_id=?1", [session.id]).bus()?;
    ctx.tx().execute(
        "UPDATE overlaps SET active=0,last_seen=?1 WHERE project_id=?2 AND active=1
         AND EXISTS (SELECT 1 FROM json_each(overlaps.sessions) WHERE value=?3)",
        params![ctx.now, session.project_id, session.name],
    ).bus()?;
    ctx.emit("overlap.changed", json!({"project_id":session.project_id,"session":session.name}));
    Ok(())
}

// Hooks and explicit reports can describe the same result more than once.
// Keep one unread card per result, without replaying its sound or peer broadcast.
fn record_agent_notification(ctx: &mut Ctx, s: &Session, category: &str, title: &str, body: &str, link: &str) -> Result<bool, BusError> {
    let existing = ctx.tx().query_row(
        "SELECT id FROM notifications WHERE project_id=?1 AND category=?2 AND title=?3 AND link=?4 AND read=0 ORDER BY id DESC LIMIT 1",
        params![s.project_id, category, title, link], |row| row.get::<_, i64>(0),
    ).optional().bus()?;
    if let Some(id) = existing {
        if body != "Agent turn completed" {
            ctx.tx().execute("UPDATE notifications SET body=?1 WHERE id=?2", params![body, id]).bus()?;
            ctx.emit("notify.changed", json!({"notification_id":id,"project_id":s.project_id}));
        }
        return Ok(false);
    }
    ctx.tx().execute(
        "INSERT INTO notifications(project_id,category,title,body,link,read,created_at) VALUES (?1,?2,?3,?4,?5,0,?6)",
        params![s.project_id, category, title, body, link, ctx.now],
    ).bus()?;
    Ok(true)
}

fn notify_completion(ctx: &mut Ctx, s: &Session, summary: &str) -> Result<(), BusError> {
    let link = done_link(s);
    if !record_agent_notification(ctx, s, "agent_done", &format!("{} finished", s.name), summary, &link)? {
        return Ok(());
    }
    if let Some(message) = crate::handlers::notes::send_system(
        ctx.tx(),
        s.project_id,
        "*",
        &format!("{} finished: {}", s.name, summary),
        s.task_id,
        &ctx.now,
    )? {
        ctx.emit("mailbox.new", serde_json::to_value(message).bus()?);
    }
    ctx.emit(
        "notify.new",
        json!({"category":"agent_done", "project_id":s.project_id, "session":s.name}),
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum LaunchKind {
    Fresh,
    Resume,
}

fn write_session_file(
    worktree: &Path,
    relative: &str,
    text: &str,
    code: &'static str,
) -> Result<(), BusError> {
    let path = worktree.join(relative);
    let parent = path
        .parent()
        .ok_or_else(|| BusError::internal("session brief path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        BusError::unavailable(code, format!("cannot create {}: {error}", parent.display()))
    })?;
    fs::write(&path, text).map_err(|error| {
        BusError::unavailable(code, format!("cannot write {}: {error}", path.display()))
    })
}

fn uninstall_git_chain(tx: &Transaction, repo: &Path, worktree: &Path) -> Result<(), BusError> {
    let mut stmt = tx
        .prepare_cached("SELECT name FROM sessions WHERE worktree=?1 ORDER BY id DESC")
        .bus()?;
    let names = stmt
        .query_map([worktree.display().to_string()], |row| {
            row.get::<_, String>(0)
        })
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    for name in names {
        crate::hooks::uninstall_git(repo, worktree, &name).bus()?;
    }
    Ok(())
}

fn launch(ctx: &mut Ctx, row: &Row_, kind: LaunchKind) -> Result<Session, BusError> {
    let cmd = crate::providers::executable(ctx.tx(), row.session.provider)?;
    let cwd = PathBuf::from(&row.session.worktree);
    if !cwd.is_dir() {
        return Err(BusError::conflict(
            "session.worktree_missing",
            format!("worktree {} is gone", cwd.display()),
        ));
    }
    let relay_bin = crate::hooks::relay_bin();
    let project = crate::handlers::workspace::get_project(ctx.tx(), row.session.project_id)?;
    crate::hooks::install_git(
        Path::new(&project.path),
        &cwd,
        &row.session.name,
        ctx.instance(),
        &relay_bin,
    )
    .bus()?;
    // A plugin that is on brings its MCP servers to every agent of the project (D159).
    let plugin_servers = crate::plugins::mcp_servers(ctx.tx(), row.session.project_id, &relay_bin).bus()?;
    if row.session.provider == Provider::Claude {
        crate::hooks::install_claude(&cwd, ctx.instance(), &relay_bin).bus()?;
        crate::hooks::add_claude_mcp_servers(&cwd, &plugin_servers).bus()?;
    } else if row.session.provider == Provider::Codex {
        crate::hooks::install_codex(&cwd, ctx.instance(), &relay_bin).bus()?;
    }
    // Every enabled skill becomes a real provider skill folder in this worktree, whatever
    // project it belongs to (D147). A skill folder that cannot be written is never worth
    // failing a launch over, so this reports and continues.
    if let Err(error) =
        crate::skills::materialize(ctx.tx(), &ctx.engine().store, &cwd, row.session.project_id)
    {
        tracing::warn!(session = %row.session.name, error = %error, "materializing skills");
    }
    // Codex reads skills only from its own home, so the checkout alone would leave a Codex
    // session with nothing registered (D147).
    if let Err(error) =
        crate::skills::materialize_user(ctx.tx(), &ctx.engine().store, ctx.instance())
    {
        tracing::warn!(session = %row.session.name, error = %error, "materializing user skills");
    }
    // The brief is delivered on *every* spawn, not only with a dispatched task: an agent with
    // no assignment is exactly the one that most needs to know who its peers are (D101). It
    // travels inside the role instruction file because that is the channel both providers
    // already read; the skills half stays on disk, referenced by path.
    let brief = crate::awareness::brief(ctx.tx(), &row.session.name, Some(ctx.engine()))?;
    // The file lives inside the private worktree, so it may carry the launch assignment. The
    // injected copy may not: for Codex it ends up in argv, which anyone can read with `ps`.
    let mut on_disk = brief.compact.clone();
    if let Some(assignment) = row
        .launch_prompt
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        on_disk.push_str(&format!("\n\nLaunch assignment:\n{assignment}"));
    }
    let brief_path = crate::awareness::session_brief_path(&row.session.name);
    let role_path = crate::providers::role_instructions_relative(&row.session.name);
    write_session_file(&cwd, &brief_path, &on_disk, "session.brief_write_failed")?;
    write_session_file(
        &cwd,
        SESSION_SKILLS_PATH,
        &brief.parts.skills,
        "session.brief_write_failed",
    )?;
    let instructions = format!(
        "{}\n\n{}",
        crate::providers::role_instructions(row.session.role),
        brief.compact,
    );
    write_session_file(
        &cwd,
        &role_path,
        &instructions,
        "session.role_instructions_write_failed",
    )?;
    let mut args = match kind {
        LaunchKind::Fresh => crate::providers::driver(row.session.provider).args(
            &row.session,
            crate::providers::Launch::Fresh,
            Some(&brief.compact),
        ),
        LaunchKind::Resume => crate::providers::driver(row.session.provider).args(
            &row.session,
            crate::providers::Launch::Resume {
                provider_ref: row.session.provider_ref.as_deref(),
            },
            Some(&brief.compact),
        ),
    };
    if row.session.provider == Provider::Codex {
        args.extend([
            "--config".into(),
            crate::providers::codex_notify_config(&relay_bin),
        ]);
        for (name, command, server_args, env) in &plugin_servers {
            args.extend(crate::providers::codex_mcp_config(name, command, server_args, env));
        }
    }
    let cfg = crate::guardrail::config(ctx.tx(), Some(row.session.project_id))?;
    for root in crate::guardrail::write_roots(&cfg, &cwd).into_iter().filter(|root| root != &cwd) {
        // Provider directory grants need a real directory. Test engines never
        // create directories in the user's provider homes.
        if ctx.instance() != crate::Instance::Test && !root.is_dir() {
            fs::create_dir_all(&root).map_err(|error| BusError::unavailable(
                "session.write_root", format!("cannot prepare {}: {error}", root.display()),
            ))?;
        }
        args.extend(["--add-dir".into(), root.display().to_string()]);
    }
    let initial_scrollback = match kind {
        LaunchKind::Fresh => Vec::new(),
        LaunchKind::Resume => sessions::load_scrollback(ctx.tx(), row.session.id)?
            .map(|saved| saved.0.into_bytes())
            .unwrap_or_default(),
    };
    let epoch = row.epoch + 1;
    let spec = SpawnSpec {
        cmd: cmd.display().to_string(),
        args,
        env: vec![
            ("RELAY_SESSION".into(), row.session.name.clone()),
            ("RELAY_TOKEN".into(), row.token.clone()),
            ("RELAY_INSTANCE".into(), ctx.instance().as_str().to_string()),
            ("RELAY_PROJECT".into(), row.session.project_id.to_string()),
            ("RELAY_WORKTREE".into(), row.session.worktree.clone()),
            ("RELAY_BRIEF".into(), brief_path),
            ("RELAY_BIN".into(), relay_bin.display().to_string()),
            (
                "RELAY_STORE".into(),
                ctx.engine().store.path().display().to_string(),
            ),
        ],
        cwd,
        cols: 120,
        rows: 40,
        epoch,
        initial_scrollback,
    };
    let engine = ctx.engine().arc().map(|engine| Arc::downgrade(&engine));
    let sid = row.session.id;
    let project_id = row.session.project_id;
    let req_id = ctx.req_id;
    let pty = Pty::spawn(spec, move |code| {
        let Some(engine) = engine.and_then(|weak| weak.upgrade()) else { return };
        let exited = engine.system_write(
            "session.spawn.completed", Some(req_id), Some(project_id), Some(sid), json!({ "exit_code": code }),
            |tx, now| {
                let mut events = Vec::new();
                let changed = tx.execute(
                    "UPDATE sessions SET state='exited',exit_code=?1,pid=NULL,updated_at=?2
                     WHERE id=?3 AND epoch=?4 AND state IN ('spawning','running','idle','blocked')",
                    params![code, now, sid, epoch as i64],
                ).map_err(crate::engine::internal)?;
                if changed > 0 {
                    if let Some(row) = sessions::by_id(tx, sid)? {
                        if row.session.role == Role::Builder {
                          if let Some(task_id) = row.session.task_id {
                            let changed = tx.execute(
                                "UPDATE tasks SET state='failed',updated_at=?1 WHERE id=?2 AND col='active' AND deleted_at IS NULL",
                                params![now, task_id],
                            ).map_err(crate::engine::internal)?;
                            if changed > 0 {
                                events.push(("task.changed".into(), json!({"task_id":task_id,"state":"failed"})));
                            }
                          }
                        }
                        events.push(("session.changed".into(), serde_json::to_value(&row.session).unwrap_or(Value::Null)));
                    }
                }
                Ok((changed > 0, events))
            },
        );
        if exited.unwrap_or(false) {
            crate::handlers::device_lease::release_session(&engine, sid);
        }
    }).map_err(|error| BusError::unavailable("session.spawn_failed", error.to_string()))?;
    ctx.tx().execute(
        "UPDATE sessions SET state='running',pid=?1,epoch=?2,spawned_at=COALESCE(spawned_at,?3),
         exit_code=NULL,restore_reason=NULL,done_pending_stop=NULL,updated_at=?3 WHERE id=?4",
        params![pty.pid() as i64, epoch as i64, ctx.now, sid],
    ).bus()?;
    if let Some(old) = ctx.engine().set_pty(sid, &row.session.name, pty) {
        old.kill(Duration::from_millis(500));
    }
    if matches!(kind, LaunchKind::Fresh) {
        if let Some(prompt) = launch_nudge(row) {
            ctx.after_commit(move |engine| {
                if let Some(pty) = engine.pty(sid) {
                    if let Err(error) = pty.write(format!("{prompt}\r").as_bytes()) {
                        tracing::warn!(session_id = sid, error = %error, "could not send fresh-session start prompt");
                    }
                }
            });
        }
    }
    let updated =
        sessions::by_id(ctx.tx(), sid)?.ok_or_else(|| BusError::internal("session vanished"))?;
    emit_session(ctx, &updated.session);
    Ok(updated.session)
}

/// Mirror the session state we just persisted onto its live PTY. `session.input` reads this
/// instead of the session row, so the idle→running edge still produces exactly one write while
/// every other keystroke stays off the store entirely (D148).
fn note_pty_state(ctx: &Ctx, session_id: Id, state: &str) {
    if let Some(pty) = ctx.engine().pty(session_id) {
        pty.set_idle(state == "idle");
    }
}

/// Reserve the public name across the unlocked checkout phase. A failed checkout
/// releases the name reservation but preserves any files Git already created.
struct PreparedCreate {
    engine: std::sync::Weak<Engine>,
    name: String,
    project: relay_bus::types::Project,
    worktree: Option<relay_bus::types::Worktree>,
    /// Read once in the prepare phase, so both phases agree even if a plugin is switched
    /// meanwhile (D160).
    default_checkout: &'static str,
}

impl Drop for PreparedCreate {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.upgrade() {
            engine.creating_sessions.lock().unwrap().remove(&self.name);
        }
    }
}

fn validate_create(conn: &Connection, p: &CreateIn) -> Result<relay_bus::types::Project, BusError> {
    let project = crate::handlers::workspace::get_project(conn, p.project_id)?;
    crate::providers::validate_options(p.provider, p.model.as_deref(), p.effort.as_deref())?;
    if let Some(task_id) = p.task_id {
        let valid: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND project_id=?2 AND deleted_at IS NULL)",
            params![task_id, project.id], |row| row.get(0),
        ).bus()?;
        if !valid { return Err(BusError::not_found("task.not_found", format!("no task {task_id} in project {}", project.id))); }
    }
    if let Some(module_id) = p.module_id {
        let valid: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM modules WHERE id=?1 AND project_id=?2 AND deleted_at IS NULL)",
            params![module_id, project.id], |row| row.get(0),
        ).bus()?;
        if !valid { return Err(BusError::not_found("module.not_found", format!("no module {module_id} in project {}", project.id))); }
    }
    Ok(project)
}

pub fn register(e: &mut Engine) {
    e.register_staged::<Create, _>(|ctx, p| {
        let mut prepared = ctx.read(|conn| {
            let project = validate_create(conn, p)?;
            let default_checkout = crate::plugins::default_checkout(conn, project.id).bus()?;
            let mut reservations = ctx.engine().creating_sessions.lock().unwrap();
            for _ in 0..200 {
                let name = sessions::new_name(conn).bus()?;
                if worktree::pooled_path(Path::new(&project.path), &name).exists()
                    || !reservations.insert(name.clone()) { continue; }
                return Ok(PreparedCreate {
                    engine: Arc::downgrade(&ctx.engine().arc().ok_or_else(|| BusError::internal("engine unavailable"))?),
                    name, project, worktree: None, default_checkout,
                });
            }
            Err(BusError::conflict("session.names_busy", "Could not reserve a free session name"))
        })?;
        // Fetch AND checkout can invoke slow network/LFS filters. Neither belongs
        // under the global store mutex: existing sessions must remain responsive.
        if p.pair_with.is_none() && p.worktree.as_deref().unwrap_or(prepared.default_checkout) == "new" {
            let repo = Path::new(&prepared.project.path);
            super::git::refresh_new_worktree(repo, p.branch.as_deref())?;
            let branch = p.branch.clone().unwrap_or_else(|| worktree::branch_for(&prepared.name));
            let path = worktree::pooled_path(repo, &prepared.name);
            let from = if super::git::existing_worktree_branch(repo, Some(&branch))? {
                None
            } else {
                super::git::new_worktree_base(repo, &prepared.project.base_branch)?
            };
            prepared.worktree = Some(worktree::create(repo, &path, &branch, from.as_deref())
                .map_err(|e| BusError::conflict("worktree.create_failed", e.to_string()))?);
        }
        Ok(prepared)
    }, |ctx: &mut Ctx, p, mut prepared| {
        // Recheck mutable records after the external work completes.
        let project = validate_create(ctx.tx(), &p)?;
        if project.path != prepared.project.path || project.base_branch != prepared.project.base_branch {
            return Err(BusError::conflict("session.project_changed",
                "Project changed during creation; any created worktree is preserved"));
        }
        let pair = match p.pair_with.as_deref() {
            Some(name) => {
                let pair = sessions::by_name(ctx.tx(), name)?;
                if pair.session.project_id != project.id { return Err(BusError::invalid("session.pair_project", "PAIR sessions must belong to the same project")); }
                if pair.session.pair_with.is_some() {
                    let role = p.role.unwrap_or(Role::Builder);
                    let (builders, reviewers): (i64, i64) = ctx.tx().query_row(
                        "SELECT COUNT(*) FILTER (WHERE role='builder'), COUNT(*) FILTER (WHERE role='reviewer') FROM sessions WHERE worktree=?1 AND state!='closed'",
                        [&pair.session.worktree], |row| Ok((row.get(0)?, row.get(1)?)),
                    ).bus()?;
                    if role != Role::Builder || pair.session.role != Role::Reviewer || reviewers != 1 {
                        return Err(BusError::conflict("session.pair_exists", format!("session {name} is already paired")));
                    }
                    if builders >= 2 {
                        return Err(BusError::conflict("session.review_group_full", "one reviewer can supervise at most two builders"));
                    }
                }
                if p.branch.as_deref().is_some_and(|branch| branch != pair.session.branch) {
                    return Err(BusError::invalid("session.pair_branch", "PAIR sessions share one branch"));
                }
                if p.worktree.as_deref().is_some_and(|worktree| worktree != pair.session.worktree) {
                    return Err(BusError::invalid("session.pair_worktree", "omit worktree for a PAIR session; Relay reuses its partner's checkout"));
                }
                Some(pair)
            }
            None => None,
        };
        let name = prepared.name.clone();
        let token = sessions::new_token();
        let repo = Path::new(&project.path);
        let pair_worktree = pair.as_ref().map(|row| row.session.worktree.as_str());
        let requested_worktree = p.worktree.as_deref().or(pair_worktree).unwrap_or(prepared.default_checkout);
        let (worktree_path, branch) = match requested_worktree {
            "primary" => {
                let all = worktree::list_with_dirty(repo, false).bus()?;
                let primary = all.first().ok_or_else(|| BusError::internal("no primary worktree"))?;
                (primary.path.clone(), primary.branch.clone())
            }
            "new" => {
                let wt = prepared.worktree.take().ok_or_else(|| BusError::internal("checkout was not prepared"))?;
                (wt.path, wt.branch)
            }
            other => {
                let path = Path::new(other);
                if !path.is_absolute() || !path.join(".git").exists() {
                    return Err(BusError::invalid("session.worktree", format!("{other:?} is not an existing worktree path")));
                }
                let all = worktree::list_with_dirty(repo, false).bus()?;
                let want = std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or(other.to_string());
                let wt = all.into_iter().find(|w| w.path == want)
                    .ok_or_else(|| BusError::invalid("session.worktree", format!("{other:?} is not a worktree of this project")))?;
                (wt.path, wt.branch)
            }
        };
        if let Some(pair) = &pair {
            if worktree_path != pair.session.worktree {
                return Err(BusError::invalid("session.pair_worktree", "PAIR sessions must share one worktree"));
            }
        }
        let relay_bin = crate::hooks::relay_bin();
        crate::hooks::install_git(
            repo,
            Path::new(&worktree_path),
            &name,
            ctx.instance(),
            &relay_bin,
        ).bus()?;
        ctx.tx().execute(
            "INSERT INTO sessions(name, project_id, provider, role, model, effort, branch, worktree, task_id, module_id, pair_with,
                                  bus_writes, allow_ui, state, token, epoch, created_at, updated_at, launch_prompt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'created', ?14, 0, ?15, ?15, ?16)",
            params![name, project.id, sessions::provider_str(p.provider), sessions::role_str(p.role.unwrap_or(relay_bus::types::Role::Builder)),
                    p.model, p.effort, branch, worktree_path, p.task_id, p.module_id, p.pair_with,
                    p.bus_writes.unwrap_or(false) as i64, p.allow_ui.unwrap_or(false) as i64, token, ctx.now,
                    p.prompt.filter(|prompt| !prompt.trim().is_empty())],
        ).bus()?;
        let id = ctx.tx().last_insert_rowid();
        if let Some(pair) = &pair {
            if pair.session.pair_with.is_none() {
            ctx.tx().execute(
                "UPDATE sessions SET pair_with=?1,updated_at=?2 WHERE id=?3",
                params![name, ctx.now, pair.session.id],
            ).bus()?;
            }
        }
        if let Some(task_id) = p.task_id {
            let ord: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(ord), -1) + 1 FROM task_sessions WHERE task_id = ?1", [task_id], |r| r.get(0)).bus()?;
            let queue_ord: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(queue_ord), -1) + 1 FROM task_sessions WHERE session_id = ?1", [id], |r| r.get(0)).bus()?;
            ctx.tx().execute("INSERT OR IGNORE INTO task_sessions(task_id, session_id, ord, queue_ord) VALUES (?1, ?2, ?3, ?4)", params![task_id, id, ord, queue_ord]).bus()?;
        }
        let row = sessions::by_id(ctx.tx(), id)?.ok_or_else(|| BusError::internal("session vanished"))?;
        emit_session(ctx, &row.session);
        ctx.emit("worktree.changed", json!({ "project_id": project.id }));
        Ok(row.session)
    });

    e.register::<Spawn>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        match row.session.state {
            SessionState::Created => {
                if let Some(prompt) = p.prompt {
                    let launch_prompt = (!prompt.trim().is_empty()).then_some(prompt);
                    ctx.tx()
                        .execute(
                            "UPDATE sessions SET launch_prompt=?1,updated_at=?2 WHERE id=?3",
                            params![launch_prompt, ctx.now, row.session.id],
                        )
                        .bus()?;
                }
                let updated = sessions::by_id(ctx.tx(), row.session.id)?
                    .ok_or_else(|| BusError::internal("session vanished"))?;
                launch(ctx, &updated, LaunchKind::Fresh)
            }
            s if sessions::is_live(s) => Err(BusError::conflict(
                "session.already_spawned",
                format!("session {} is {}", row.session.name, sessions::state_str(s)),
            )),
            s => Err(BusError::conflict(
                "session.state",
                format!(
                    "session {} is {}; use session.resume or session.wake",
                    row.session.name,
                    sessions::state_str(s)
                ),
            )),
        }
    });

    e.register::<Resume>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        if row.session.state != SessionState::Restorable {
            return Err(BusError::conflict(
                "session.state",
                format!(
                    "session {} is {}; only restorable sessions resume",
                    row.session.name,
                    sessions::state_str(row.session.state)
                ),
            ));
        }
        launch(ctx, &row, LaunchKind::Resume)
    });

    e.register::<ClearRestorable>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        if row.session.state != SessionState::Restorable {
            return Err(BusError::conflict("session.state", format!("session {} is not restorable", row.session.name)));
        }
        // Unlike Resume, Clear must not pass the provider's saved conversation. Unlike Discard,
        // it keeps the durable Relay session and worktree, then launches a new PTY epoch in place.
        ctx.tx().execute(
            "UPDATE sessions SET provider_ref=NULL,restore_reason=NULL,exit_code=NULL,updated_at=?1 WHERE id=?2",
            params![ctx.now, row.session.id],
        ).bus()?;
        ctx.tx().execute("DELETE FROM session_scrollback WHERE session_id=?1", [row.session.id]).bus()?;
        let cleared = sessions::by_id(ctx.tx(), row.session.id)?
            .ok_or_else(|| BusError::internal("session vanished"))?;
        launch(ctx, &cleared, LaunchKind::Fresh)
    });

    e.register::<Park>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        if !sessions::is_live(row.session.state) {
            return Err(BusError::conflict("session.state", format!("session {} is {}; only live sessions park", row.session.name, sessions::state_str(row.session.state))));
        }
        let pty = ctx.engine().take_pty(row.session.id)
            .ok_or_else(|| BusError::conflict("session.not_spawned", format!("session {} has no PTY", row.session.name)))?;
        let (text, epoch, seq) = pty.scrollback(None);
        sessions::save_scrollback(ctx.tx(), row.session.id, &text, epoch, seq, &ctx.now)?;
        pty.kill(Duration::from_secs(3));
        ctx.tx().execute(
            "UPDATE sessions SET state='parked',pid=NULL,restore_reason=NULL,updated_at=?1 WHERE id=?2",
            params![ctx.now, row.session.id],
        ).bus()?;
        let updated = sessions::by_id(ctx.tx(), row.session.id)?.ok_or_else(|| BusError::internal("session vanished"))?;
        emit_session(ctx, &updated.session);
        Ok(updated.session)
    });

    e.register::<Wake>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        if row.session.state != SessionState::Parked {
            return Err(BusError::conflict(
                "session.state",
                format!(
                    "session {} is {}; only parked sessions wake",
                    row.session.name,
                    sessions::state_str(row.session.state)
                ),
            ));
        }
        launch(ctx, &row, LaunchKind::Resume)
    });

    e.register::<Get>(|ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_same_project(ctx, &row)?;
        let mut session = row.session;
        overlay_live_output(ctx, std::slice::from_mut(&mut session));
        Ok(session)
    });

    e.register::<List>(|ctx, p| {
        // One boundary, stated once: an agent sees the metadata of every session in its own
        // project and nothing outside it (D106). `session.get` draws the same line.
        let scope = actor_project(ctx)?;
        if let (Some(own), Some(asked)) = (scope, p.project_id) {
            if own != asked {
                return Err(BusError::not_own("project"));
            }
        }
        let project_id = scope.or(p.project_id);
        let mut sql = String::from("SELECT * FROM sessions WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(pid) = project_id {
            sql.push_str(" AND project_id = ?");
            args.push(Box::new(pid));
        }
        if !p.include_closed.unwrap_or(false) {
            sql.push_str(" AND state != 'closed'");
        }
        if let Some(states) = &p.state {
            if !states.is_empty() {
                let marks = states.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                sql.push_str(&format!(" AND state IN ({marks})"));
                for s in states {
                    args.push(Box::new(sessions::state_str(*s).to_string()));
                }
            }
        }
        sql.push_str(" ORDER BY id");
        // The shell re-lists sessions on every `session.changed`. The filter combinations are
        // a bounded handful, so the built SQL is cached like any fixed statement.
        let mut st = ctx.tx().prepare_cached(&sql).bus()?;
        let rows = st
            .query_map(
                rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())),
                sessions::row,
            )
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>();
        let mut sessions: Vec<Session> = rows.bus()?.into_iter().map(|r| r.session).collect();
        overlay_live_output(ctx, &mut sessions);
        Ok(ListOut { sessions })
    });

    e.register::<Peers>(|ctx, p| {
        let (project_id, except): (Id, Option<String>) = match (p.session.as_deref(), p.project_id)
        {
            (Some(name), None) => {
                let row = sessions::by_name(ctx.tx(), name)?;
                assert_own(ctx, &row, true)?;
                (row.session.project_id, Some(name.to_string()))
            }
            (None, Some(project_id)) => {
                crate::handlers::workspace::get_project(ctx.tx(), project_id)?;
                if let Some(sid) = ctx.actor_session_id() {
                    let own = sessions::by_id(ctx.tx(), sid)?
                        .ok_or_else(|| BusError::actor("bound session vanished"))?;
                    if own.session.project_id != project_id {
                        return Err(BusError::not_own("project"));
                    }
                    (project_id, Some(own.session.name))
                } else if ctx.actor.is_agent() {
                    return Err(BusError::actor(
                        "agent actor is not bound to a live session",
                    ));
                } else {
                    (project_id, None)
                }
            }
            // An agent asking "who else is here?" means its own project. Requiring it to say
            // which project, when it only ever has one, was pure ceremony.
            (None, None) => match ctx.actor_session_id() {
                Some(sid) => {
                    let own = sessions::by_id(ctx.tx(), sid)?
                        .ok_or_else(|| BusError::actor("bound session vanished"))?;
                    (own.session.project_id, Some(own.session.name))
                }
                None => {
                    return Err(BusError::invalid(
                        "session.peers_target",
                        "provide exactly one of session or project_id",
                    ))
                }
            },
            _ => {
                return Err(BusError::invalid(
                    "session.peers_target",
                    "provide exactly one of session or project_id",
                ))
            }
        };
        Ok(PeersOut {
            peers: crate::awareness::peers(
                ctx.tx(),
                project_id,
                except.as_deref(),
                Some(ctx.engine()),
            )?,
        })
    });

    e.register::<Brief>(|ctx, p| {
        // Metadata is project-visible; a peer's brief and scrollback are not.
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, true)?;
        crate::awareness::brief(ctx.tx(), &p.session, Some(ctx.engine()))
    });

    e.register::<Bootstrap>(|ctx, _| {
        let session_id = ctx
            .actor_session_id()
            .ok_or_else(|| BusError::actor("session.bootstrap requires a bound agent session"))?;
        let row = sessions::by_id(ctx.tx(), session_id)?
            .ok_or_else(|| BusError::actor("bound session vanished"))?;
        crate::awareness::bootstrap(ctx.tx(), &row.session.name, ctx.engine())
    });

    e.register::<Done>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, false)?;
        let s = &row.session;
        // An op that can only report success gets reported success (D118). `blocked` and
        // `partial` leave the task where it is and hand the reason to the person.
        let status = p.status.as_deref().unwrap_or("completed");
        if !matches!(status, "completed" | "blocked" | "partial") {
            return Err(BusError::invalid(
                "session.done_status",
                "status must be completed, blocked, or partial",
            ));
        }
        let blockers: Vec<String> = p.blockers.unwrap_or_default()
            .into_iter().map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect();
        if status != "completed" && blockers.is_empty() {
            return Err(BusError::invalid(
                "session.done_blockers",
                format!("status {status:?} needs at least one blocker: say what stopped it"),
            ));
        }
        let current_task_id = s.task_id;
        if status == "completed" {
            ctx.tx().execute("UPDATE sessions SET done_pending_stop=?1 WHERE id=?2", params![current_task_id.unwrap_or(0),s.id]).bus()?;
        }
        if matches!(s.role, Role::Builder | Role::Reviewer) && status == "completed" {
            if let Some(task_id) = current_task_id { complete_group_assignment(ctx,s,task_id,p.sha.as_deref())?; }
        }
        if status != "completed" {
            if let Some(task_id) = current_task_id {
                ctx.tx().execute("UPDATE task_sessions SET completed_at=NULL WHERE task_id=?1 AND session_id=?2",params![task_id,s.id]).bus()?;
            }
        }
        // A builder that stopped short leaves its task visible as stopped, not silently active.
        if s.role == Role::Builder && status != "completed" {
            if let Some(task_id) = current_task_id {
                let changed = ctx.tx().execute(
                    "UPDATE tasks SET state='blocked', updated_at=?1
                     WHERE id=?2 AND project_id=?3 AND deleted_at IS NULL AND col='active'",
                    params![ctx.now, task_id, s.project_id],
                ).bus()?;
                if changed > 0 {
                    ctx.emit("task.changed", json!({"task_id": task_id, "state": "blocked"}));
                }
            }
        }
        let session_state = if status == "completed" { "idle" } else { "blocked" };
        release_claims(ctx, s)?;
        ctx.tx().execute(
            "UPDATE sessions SET state=?1, last_output_at=?2, updated_at=?2 WHERE id=?3",
            params![session_state, ctx.now, s.id],
        ).bus()?;
        note_pty_state(ctx, s.id, session_state);
        let reported = p.summary.as_deref().map(str::trim).filter(|v| !v.is_empty());
        let summary = match (status, reported) {
            ("completed", Some(text)) => text.to_string(),
            ("completed", None) => "Agent reported completion".to_string(),
            (_, text) => format!(
                "{} — {}: {}",
                text.unwrap_or("Agent stopped short"),
                if status == "blocked" { "blocked by" } else { "still open" },
                blockers.join("; "),
            ),
        };
        let summary = summary.as_str();
        let category = if status == "completed" { "agent_done" } else { "agent_blocked" };
        let link = done_link(s);
        let headline = match status {
            "completed" => format!("{} finished", s.name),
            "blocked" => format!("{} is blocked", s.name),
            _ => format!("{} finished part of its work", s.name),
        };
        let fresh_notification = record_agent_notification(ctx, s, category, &headline, summary, &link)?;
        if fresh_notification {
          if let Some(message) = crate::handlers::notes::send_system(
            ctx.tx(), s.project_id, "*", &format!("{headline}: {summary}"), s.task_id, &ctx.now,
          )? {
            ctx.emit("mailbox.new", serde_json::to_value(message).bus()?);
          }
        }
        let updated = sessions::by_id(ctx.tx(), s.id)?.ok_or_else(|| BusError::internal("session vanished"))?;
        emit_session(ctx, &updated.session);
        if fresh_notification {
            ctx.emit("notify.new", json!({"category":category, "project_id":s.project_id, "session":s.name, "status":status}));
        }
        Ok(updated.session)
    });

    // ---------------------------------------------------------------- intent and claims
    e.register::<Intent>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, false)?;
        let text = p.text.trim();
        if text.chars().count() > 200 {
            return Err(BusError::invalid(
                "session.intent",
                "intent is one line: 200 characters at most",
            ));
        }
        let intent = (!text.is_empty()).then(|| text.to_string());
        ctx.tx()
            .execute(
                "UPDATE sessions SET intent=?1, updated_at=?2 WHERE id=?3",
                params![intent, ctx.now, row.session.id],
            )
            .bus()?;
        let updated = sessions::by_id(ctx.tx(), row.session.id)?
            .ok_or_else(|| BusError::internal("session vanished"))?;
        emit_session(ctx, &updated.session);
        Ok(updated.session)
    });

    e.register::<Claim>(|ctx: &mut Ctx, p| {
        let session_id = ctx.actor_session_id()
            .ok_or_else(|| BusError::actor("session.claim requires a bound agent session"))?;
        let own = sessions::by_id(ctx.tx(), session_id)?
            .ok_or_else(|| BusError::actor("bound session vanished"))?.session;
        if p.paths.is_empty() {
            return Err(BusError::invalid("session.claim", "claim at least one path"));
        }
        let symbol = p.symbol.unwrap_or_default().trim().to_string();
        let mut paths = Vec::with_capacity(p.paths.len());
        for raw in &p.paths {
            paths.push(claim_path(raw)?);
        }

        // Who already holds these. Reported either way: a collision an agent is told about is
        // one it can negotiate, which is the whole point of a claim (D116).
        let mut collisions = Vec::new();
        for path in &paths {
            let mut stmt = ctx.tx().prepare_cached(
                "SELECT session, symbol, created_at FROM claims
                 WHERE project_id=?1 AND path=?2 AND session_id!=?3
                   AND (symbol='' OR ?4='' OR symbol=?4) ORDER BY session",
            ).bus()?;
            let held = stmt.query_map(params![own.project_id, path, session_id, symbol], |row| {
                Ok(Collision {
                    path: path.clone(),
                    symbol: row.get("symbol")?,
                    session: row.get("session")?,
                    since: row.get("created_at")?,
                })
            }).bus()?.collect::<rusqlite::Result<Vec<_>>>().bus()?;
            collisions.extend(held);
        }
        if p.exclusive.unwrap_or(false) && !collisions.is_empty() {
            return Err(BusError::conflict(
                "session.claim_held",
                format!(
                    "{} already claimed by {}",
                    collisions[0].path,
                    collisions.iter().map(|c| c.session.as_str()).collect::<Vec<_>>().join(", "),
                ),
            ).with_details(serde_json::to_value(&collisions).bus()?)
             .with_hint("drop exclusive to work alongside them, or coordinate with mailbox.send"));
        }
        for path in &paths {
            ctx.tx().execute(
                "INSERT INTO claims(project_id, session_id, session, path, symbol, note, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(session_id, path, symbol) DO UPDATE SET note=excluded.note, updated_at=excluded.updated_at",
                params![own.project_id, session_id, own.name, path, symbol, p.note, ctx.now],
            ).bus()?;
        }
        ctx.set_project(own.project_id);
        ctx.emit("overlap.changed", json!({"project_id": own.project_id, "session": own.name, "claimed": paths}));
        Ok(ClaimOut {
            claimed: paths,
            collisions,
        })
    });

    e.register::<Release>(|ctx: &mut Ctx, p| {
        let session_id = ctx
            .actor_session_id()
            .ok_or_else(|| BusError::actor("session.release requires a bound agent session"))?;
        let own = sessions::by_id(ctx.tx(), session_id)?
            .ok_or_else(|| BusError::actor("bound session vanished"))?
            .session;
        let released = match &p.paths {
            None => ctx
                .tx()
                .execute("DELETE FROM claims WHERE session_id=?1", [session_id])
                .bus()?,
            Some(paths) => {
                let mut removed = 0;
                for raw in paths {
                    let path = claim_path(raw)?;
                    removed += ctx
                        .tx()
                        .execute(
                            "DELETE FROM claims WHERE session_id=?1 AND path=?2",
                            params![session_id, path],
                        )
                        .bus()?;
                }
                removed
            }
        };
        ctx.set_project(own.project_id);
        ctx.emit(
            "overlap.changed",
            json!({"project_id": own.project_id, "session": own.name, "released": released}),
        );
        Ok(ReleaseOut {
            released: released as u32,
        })
    });

    e.register::<Report>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, false)?;
        let s = &row.session;
        let data = p.data.unwrap_or_else(|| json!({}));
        let pending_stop: Option<Id> = ctx.tx().query_row(
            "SELECT done_pending_stop FROM sessions WHERE id=?1", [s.id], |row| row.get(0),
        ).bus()?;
        let completed_without_done = p.kind == "stop" && pending_stop.is_none() && s.state == SessionState::Running;
        let provider_ref = data.get("session_id").or_else(|| data.get("provider_ref")).and_then(Value::as_str);
        let state = match p.kind.as_str() {
            "session_start" | "tool_use" => "running",
            "stop" | "idle" => "idle",
            "blocked" => "blocked",
            "notification" => {
                let type_ = data.get("notification_type").or_else(|| data.get("type")).and_then(Value::as_str).unwrap_or("");
                if matches!(type_, "permission_prompt" | "idle_prompt" | "agent_needs_input" | "blocked") { "blocked" } else { sessions::state_str(s.state) }
            }
            _ => return Err(BusError::invalid("session.report_kind", "kind must be session_start, tool_use, stop, notification, idle, or blocked")),
        };
        let state_changed = state != sessions::state_str(s.state);
        let provider_ref_changed = provider_ref.is_some_and(|value| s.provider_ref.as_deref() != Some(value));
        ctx.tx().execute(
            "UPDATE sessions SET state=?1, provider_ref=COALESCE(?2, provider_ref), last_output_at=?3, updated_at=?3 WHERE id=?4",
            params![state, provider_ref, ctx.now, s.id],
        ).bus()?;
        if matches!(p.kind.as_str(), "stop" | "session_start") {
            ctx.tx().execute("UPDATE sessions SET done_pending_stop=NULL WHERE id=?1", [s.id]).bus()?;
        }
        note_pty_state(ctx, s.id, state);
        if matches!(p.kind.as_str(), "tool_use" | "stop") {
            // The command behind a device lease has finished; keep the lease only briefly.
            let command = data.pointer("/tool_input/command").and_then(Value::as_str);
            crate::handlers::device_lease::command_finished(ctx.engine(), s.id, command, p.kind == "stop");
        }
        if p.kind == "stop" {
            if let (Some(completed), Some(current)) = (pending_stop, s.task_id) {
                if completed != current { deliver_assignment(ctx, s, current, false)?; }
            }
        }
        ctx.set_project(s.project_id);
        if s.role == Role::Builder {
            let task_state = match state { "running" => Some("running"), "blocked" => Some("blocked"), _ => None };
            if let (Some(task_state), Some(task_id)) = (task_state, s.task_id) {
                let changed = ctx.tx().execute(
                    "UPDATE tasks SET state=?1,updated_at=?2 WHERE id=?3 AND col='active' AND deleted_at IS NULL AND state!=?1",
                    params![task_state, ctx.now, task_id],
                ).bus()?;
                if changed > 0 {
                    ctx.emit("task.changed", json!({"task_id":task_id,"state":task_state}));
                }
            }
        }
        let blocked = state == "blocked";
        if blocked && state_changed {
            let body = data.get("message").and_then(Value::as_str).unwrap_or("Agent needs attention");
            let link = json!({"op":"session.get", "payload":{"session":s.name}}).to_string();
            if record_agent_notification(ctx, s, "agent_blocked", &format!("{} needs attention", s.name), body, &link)? {
            ctx.emit("notify.new", json!({"category":"agent_blocked", "project_id":s.project_id, "session":s.name}));
            if let Some(message) = crate::handlers::notes::send_system(
                ctx.tx(), s.project_id, "*", &format!("{} is blocked: {}", s.name, body), s.task_id, &ctx.now,
            )? {
                ctx.emit("mailbox.new", serde_json::to_value(message).bus()?);
            }
            }
        }
        if completed_without_done {
            let summary = data.get("last-assistant-message")
                .or_else(|| data.get("last_assistant_message"))
                .or_else(|| data.get("message"))
                .and_then(Value::as_str)
                .filter(|message| !message.trim().is_empty())
                .unwrap_or("Agent turn completed");
            notify_completion(ctx, s, summary)?;
        }
        if state_changed || provider_ref_changed {
            let updated = sessions::by_id(ctx.tx(), s.id)?.ok_or_else(|| BusError::internal("session vanished"))?;
            emit_session(ctx, &updated.session);
        }
        Ok(Empty {})
    });

    e.register::<Update>(|ctx: &mut Ctx, p| {
        let before = sessions::by_name(ctx.tx(), &p.session)?;
        let s = &before.session;
        let spawned = before.epoch > 0 || s.spawned_at.is_some();
        if spawned && (p.branch.is_some() || p.model.is_some() || p.effort.is_some()) {
            return Err(BusError::conflict("session.already_spawned", "branch, model, and effort cannot change after first spawn"));
        }
        let branch = p.branch.clone().unwrap_or_else(|| s.branch.clone());
        let model = p.model.clone().map(Some).unwrap_or_else(|| s.model.clone());
        let effort = p.effort.clone().map(Some).unwrap_or_else(|| s.effort.clone());
        crate::providers::validate_options(s.provider, model.as_deref(), effort.as_deref())?;
        let task_id = p.task_id.unwrap_or(s.task_id);
        let module_id = p.module_id.unwrap_or(s.module_id);
        if let Some(task_id) = task_id {
            let valid: bool = ctx.tx().query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND project_id=?2 AND deleted_at IS NULL)",
                params![task_id, s.project_id], |row| row.get(0),
            ).bus()?;
            if !valid { return Err(BusError::not_found("task.not_found", format!("no task {task_id} in project {}", s.project_id))); }
        }
        if let Some(module_id) = module_id {
            let valid: bool = ctx.tx().query_row(
                "SELECT EXISTS(SELECT 1 FROM modules WHERE id=?1 AND project_id=?2 AND deleted_at IS NULL)",
                params![module_id, s.project_id], |row| row.get(0),
            ).bus()?;
            if !valid { return Err(BusError::not_found("module.not_found", format!("no module {module_id} in project {}", s.project_id))); }
        }
        if branch != s.branch {
            if s.pair_with.is_some() { return Err(BusError::conflict("session.pair_branch", "a PAIR branch cannot be renamed independently")); }
            worktree::rename_branch(Path::new(&s.worktree), &branch)
                .map_err(|error| BusError::conflict("session.branch", error.to_string()))?;
        }
        ctx.tx().execute(
            "UPDATE sessions SET branch=?1,model=?2,effort=?3,task_id=?4,module_id=?5,bus_writes=?6,allow_ui=?7,updated_at=?8 WHERE id=?9",
            params![branch, model, effort, task_id, module_id, p.bus_writes.unwrap_or(s.bus_writes) as i64,
                p.allow_ui.unwrap_or(s.allow_ui) as i64, ctx.now, s.id],
        ).bus()?;
        if let Some(task_id) = task_id {
            let ord: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(ord),-1)+1 FROM task_sessions WHERE task_id=?1", [task_id], |row| row.get(0)).bus()?;
            let queue_ord: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(queue_ord),-1)+1 FROM task_sessions WHERE session_id=?1", [s.id], |row| row.get(0)).bus()?;
            ctx.tx().execute("INSERT OR IGNORE INTO task_sessions(task_id,session_id,ord,queue_ord) VALUES (?1,?2,?3,?4)", params![task_id,s.id,ord,queue_ord]).bus()?;
        }
        let updated = sessions::by_id(ctx.tx(), s.id)?.ok_or_else(|| BusError::internal("session vanished"))?;
        ctx.set_undo("session.update", json!({
            "session": s.name, "branch": s.branch, "model": s.model, "effort": s.effort,
            "task_id": s.task_id, "module_id": s.module_id, "bus_writes": s.bus_writes, "allow_ui": s.allow_ui,
        }), Some(json!({"updated_at":updated.session.updated_at})));
        emit_session(ctx, &updated.session);
        Ok(updated.session)
    });

    e.register_unlocked::<RestorableList>(|ctx, _| {
        let rows = ctx.read(|conn| {
            let mut stmt = conn.prepare_cached("SELECT * FROM sessions WHERE state='restorable' ORDER BY id").bus()?;
            let rows = stmt.query_map([], |row| Ok((sessions::row(row)?, row.get::<_, Option<String>>("restore_reason")?)))
                .bus()?.collect::<rusqlite::Result<Vec<_>>>().bus()?;
            Ok(rows)
        })?;
        let mut out = Vec::new();
        for (row, reason) in rows {
            let dirty = gix::open(&row.session.worktree)
                .ok()
                .is_some_and(|repo| worktree::is_dirty(&repo));
            out.push(Restorable {
                session: row.session,
                reason: reason.unwrap_or_else(|| "app_restart".into()),
                worktree_dirty: dirty,
            });
        }
        Ok(RestorableOut { sessions: out })
    });

    e.register::<DiscardRestorable>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        let s = &row.session;
        if s.state != SessionState::Restorable {
            return Err(BusError::conflict("session.state", format!("session {} is not restorable", s.name)));
        }
        let project = crate::handlers::workspace::get_project(ctx.tx(), s.project_id)?;
        let other_sessions: i64 = ctx.tx().query_row(
            "SELECT COUNT(*) FROM sessions WHERE id!=?1 AND worktree=?2 AND state!='closed'",
            params![s.id,s.worktree], |r| r.get(0),
        ).bus()?;
        let repo = Path::new(&project.path);
        let wt = Path::new(&s.worktree);
        if other_sessions == 0 {
            uninstall_git_chain(ctx.tx(), repo, wt)?;
            if ctx.tx().query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE worktree=?1 AND provider='claude')", [&s.worktree], |r| r.get::<_,bool>(0)).bus()? {
                crate::hooks::uninstall_claude(wt).bus()?;
            }
            if ctx.tx().query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE worktree=?1 AND provider='codex')", [&s.worktree], |r| r.get::<_,bool>(0)).bus()? {
                crate::hooks::uninstall_codex(wt).bus()?;
            }
            if wt.starts_with(worktree::pool_dir(repo)) {
                worktree::remove(repo, wt, true).map_err(|error| BusError::conflict("worktree.remove_failed", error.to_string()))?;
            }
        }
        ctx.tx().execute("UPDATE sessions SET state='closed',pid=NULL,closed_at=?1,updated_at=?1 WHERE id=?2", params![ctx.now,s.id]).bus()?;
        release_claims(ctx, s)?;
        ctx.tx().execute("UPDATE sessions SET pair_with=NULL,updated_at=?1 WHERE pair_with=?2 AND state!='closed'", params![ctx.now,s.name]).bus()?;
        ctx.tx().execute("DELETE FROM session_scrollback WHERE session_id=?1", [s.id]).bus()?;
        let updated = sessions::by_id(ctx.tx(), s.id)?.ok_or_else(|| BusError::internal("session vanished"))?;
        emit_session(ctx, &updated.session);
        ctx.emit("worktree.changed", json!({"project_id":s.project_id}));
        Ok(Empty {})
    });

    e.register::<Close>(|ctx: &mut Ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        let s = &row.session;
        let project = crate::handlers::workspace::get_project(ctx.tx(), s.project_id)?;
        let pair_live: bool = ctx.tx().query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id!=?1 AND worktree=?2 AND state!='closed')",
            params![s.id, s.worktree], |row| row.get(0),
        ).bus()?;
        if pair_live && p.remove_worktree.unwrap_or(true) {
            return Err(BusError::conflict("session.pair_live", "close the PAIR partner first or pass remove_worktree:false"));
        }
        if let Some(pty) = ctx.engine().take_pty(s.id) {
            let (text, epoch, seq) = pty.scrollback(None);
            sessions::save_scrollback(ctx.tx(), s.id, &text, epoch, seq, &ctx.now)?;
            if p.remove_worktree.unwrap_or(true) {
                // Stop writers before deleting their checkout, with a bounded grace period.
                pty.kill(Duration::from_millis(150));
            } else {
                // Provider shutdown hooks may need the bus. Let them run after the
                // transaction unlocks instead of waiting on them while holding it.
                ctx.after_commit(move |_| pty.kill(Duration::from_millis(150)));
            }
        }
        let mut freed = 0u64;
        let repo = Path::new(&project.path);
        let wt = Path::new(&s.worktree);
        let other_sessions: i64 = ctx.tx().query_row(
            "SELECT COUNT(*) FROM sessions WHERE id!=?1 AND worktree=?2 AND state!='closed'",
            params![s.id, s.worktree], |row| row.get(0),
        ).bus()?;
        if other_sessions == 0 {
            uninstall_git_chain(ctx.tx(), repo, wt)?;
            let any_claude: bool = ctx.tx().query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE worktree=?1 AND provider='claude')",
                [&s.worktree], |row| row.get(0),
            ).bus()?;
            if any_claude {
                crate::hooks::uninstall_claude(wt).bus()?;
            }
            let any_codex: bool = ctx.tx().query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE worktree=?1 AND provider='codex')",
                [&s.worktree], |row| row.get(0),
            ).bus()?;
            if any_codex {
                crate::hooks::uninstall_codex(wt).bus()?;
            }
        }
        let pooled = wt.starts_with(worktree::pool_dir(repo));
        if pooled && p.remove_worktree.unwrap_or(true) {
            freed = worktree::remove(repo, wt, p.purge_build.unwrap_or(true))
                .map_err(|e| BusError::conflict("worktree.remove_failed", e.to_string()))?;
        }
        // The generated hook directory outlives nothing: leaving one per dead session behind
        // makes `.relay/hooks` read like a fleet that never shut down.
        crate::hooks::remove_hook_dir(repo, &s.name);
        ctx.tx().execute("UPDATE sessions SET state = 'closed', pid = NULL, closed_at = ?1, updated_at = ?1 WHERE id = ?2", params![ctx.now, s.id]).bus()?;
        ctx.tx().execute("UPDATE sessions SET pair_with=NULL,updated_at=?1 WHERE pair_with=?2 AND state!='closed'", params![ctx.now, s.name]).bus()?;
        ctx.tx().execute("DELETE FROM session_scrollback WHERE session_id=?1", [s.id]).bus()?;
        release_claims(ctx, s)?;
        let expired = ctx.tx().execute(
            "UPDATE holds SET state = 'expired', resolved_at = ?1, resolved_by = 'system'
             WHERE session_id = ?2 AND state = 'open'",
            params![ctx.now, s.id],
        ).bus()?;
        let row = sessions::by_id(ctx.tx(), s.id)?.ok_or_else(|| BusError::internal("session vanished"))?;
        emit_session(ctx, &row.session);
        if expired > 0 {
            ctx.emit("guardrail.resolved", json!({
                "session": s.name,
                "state": "expired",
                "count": expired,
            }));
        }
        ctx.emit("worktree.changed", json!({ "project_id": project.id }));
        // Off the request: the session's device leases go, and its branch is deleted if its
        // work is already merged (or kept, with the reason audited) — `branch_cleanup`.
        let (session_id, project_id, branch) = (s.id, project.id, s.branch.clone());
        ctx.after_commit(move |engine| {
            crate::handlers::device_lease::release_session(&engine, session_id);
            // Tests drive the cleanup synchronously instead (`branch_cleanup::run`).
            if engine.instance != crate::Instance::Test {
                crate::branch_cleanup::after_close(engine, project_id, branch);
            }
        });
        Ok(CloseOut { freed_mb: freed as f64 / (1024.0 * 1024.0) })
    });

    e.register::<Attach>(|ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, true)?;
        if ctx.engine().pty(row.session.id).is_none() {
            return Err(BusError::conflict(
                "session.not_spawned",
                format!(
                    "session {} has no PTY (state {})",
                    row.session.name,
                    sessions::state_str(row.session.state)
                ),
            ));
        }
        Ok(Empty {})
    });
    e.register::<Detach>(|ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, true)?;
        Ok(Empty {})
    });
    e.register::<Input>(|ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        let pty = ctx.engine().pty(row.session.id).ok_or_else(|| {
            BusError::conflict(
                "session.not_spawned",
                format!("session {} has no PTY", row.session.name),
            )
        })?;
        if pty.exited() {
            return Err(BusError::conflict(
                "session.exited",
                format!("session {} has exited", row.session.name),
            ));
        }
        pty.write(p.data.as_bytes())
            .map_err(|e| BusError::conflict("session.io", e.to_string()))?;
        if row.session.state == SessionState::Idle && !p.data.is_empty() {
            ctx.tx()
                .execute(
                    "UPDATE sessions SET state='running',updated_at=?1 WHERE id=?2",
                    params![ctx.now, row.session.id],
                )
                .bus()?;
            let updated = sessions::by_id(ctx.tx(), row.session.id)?
                .ok_or_else(|| BusError::internal("session vanished"))?;
            emit_session(ctx, &updated.session);
        }
        Ok(Empty {})
    });
    e.register::<Resize>(|ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        let pty = ctx.engine().pty(row.session.id).ok_or_else(|| {
            BusError::conflict(
                "session.not_spawned",
                format!("session {} has no PTY", row.session.name),
            )
        })?;
        pty.resize(p.cols, p.rows)
            .map_err(|e| BusError::conflict("session.io", e.to_string()))?;
        Ok(Empty {})
    });
    e.register::<Scrollback>(|ctx, p| {
        let row = sessions::by_name(ctx.tx(), &p.session)?;
        assert_own(ctx, &row, true)?;
        let (text, epoch, seq) = match ctx.engine().pty(row.session.id) {
            Some(pty) => pty.scrollback(p.lines.map(|n| n as usize)),
            None => {
                let (text, epoch, seq) = sessions::load_scrollback(ctx.tx(), row.session.id)?
                    .ok_or_else(|| {
                        BusError::conflict(
                            "session.not_spawned",
                            format!(
                                "session {} has no PTY or saved scrollback",
                                row.session.name
                            ),
                        )
                    })?;
                let text = match p.lines {
                    Some(lines) => {
                        let all = text.lines().collect::<Vec<_>>();
                        all[all.len().saturating_sub(lines as usize)..].join("\n")
                    }
                    None => text,
                };
                (text, epoch, seq)
            }
        };
        Ok(ScrollbackOut { text, epoch, seq })
    });
}

/// For the socket door: resolve a session name to its PTY (after `session.attach` succeeded).
pub fn pty_by_name(engine: &Engine, name: &str) -> Result<(Id, Arc<Pty>), BusError> {
    let conn = engine.store.lock();
    let row = sessions::by_name(&conn, name)?;
    drop(conn);
    let pty = engine.pty(row.session.id).ok_or_else(|| {
        BusError::conflict("session.not_spawned", format!("session {name} has no PTY"))
    })?;
    Ok((row.session.id, pty))
}
