//! Crash recovery on launch (SPEC §14): reap orphaned provider processes, fsck the sessions
//! table against live PIDs, flag dirty worktrees, list tasks stuck in `active` with no live
//! session. Logs what it did to `meta.recovery.last` (`app.recovery.last`).

use crate::engine::Engine;
use crate::pty;

use anyhow::Result;
use relay_bus::ops::app::RecoveryReport;
use relay_bus::types::Id;
use rusqlite::params;
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long an audit row survives when settings say nothing. Well past any undo grace, and past
/// any window in which `audit.list` is still how someone reconstructs what happened.
const DEFAULT_AUDIT_RETENTION_DAYS: i64 = 180;

fn kill_wait(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
        libc::kill(pid as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while pty::pid_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if pty::pid_alive(pid) {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// Run once at engine start, before the doors open. Returns the report it also stored.
/// When the dirty-worktree flagging (step 3) happens.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DirtyScan {
    /// In the call, before the report is stored: what tests want.
    Inline,
    /// On a worker after the report is stored, filling `dirty_worktrees` in when it is done:
    /// what `relay serve` wants, because the flag is advisory and the scan is a `git status`
    /// per checkout, which was most of a 160 ms startup (PERF §1.5).
    Deferred,
}

pub fn run(engine: &Engine) -> Result<RecoveryReport> {
    run_with(engine, DirtyScan::Inline)
}

pub fn run_with(engine: &Engine, dirty_scan: DirtyScan) -> Result<RecoveryReport> {
    let now = crate::time::now();
    let instance = engine.instance.as_str();
    let mut report = RecoveryReport {
        at: now.clone(),
        reaped_pids: vec![],
        fsck_fixes: vec![],
        dirty_worktrees: vec![],
        tasks_reset_offered: vec![],
    };

    // 1. orphans: any live process carrying our instance's RELAY_SESSION was spawned by a
    //    previous engine. Nothing can reattach to it — reap it; its session becomes restorable.
    let store_path = engine.store.path().display().to_string();
    let orphans = pty::relay_children(instance, &store_path);
    for (pid, name) in &orphans {
        kill_wait(*pid);
        report.reaped_pids.push(*pid as i64);
        report
            .fsck_fixes
            .push(format!("reaped orphan pid {pid} (session {name})"));
    }

    let mut conn = engine.store.lock();
    let tx = conn.transaction()?;
    // 2. sessions that claim to be live: their pid is dead (or was just reaped) → restorable
    {
        let mut st = tx.prepare_cached("SELECT id, name, state, pid FROM sessions WHERE state IN ('spawning','running','idle','blocked')")?;
        let rows: Vec<(Id, String, String, Option<i64>)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        for (id, name, state, pid) in rows {
            let why = match pid {
                // Alive, and its environment says it is this session's provider: a child of a
                // previous engine that step 1 has not finished off yet. Reap it.
                Some(p) if pty::owned_by_session(p as u32, instance, &store_path, &name) => {
                    kill_wait(p as u32);
                    if !report.reaped_pids.contains(&p) {
                        report.reaped_pids.push(p);
                    }
                    format!("pid {p} still alive from a previous engine; reaped")
                }
                // Alive, but not ours: the number was recycled onto some other process since
                // the row was written. It is left strictly alone; the row is still stale.
                Some(p) if pty::pid_alive(p as u32) => {
                    format!("pid {p} now belongs to another process; left alone")
                }
                Some(p) => format!("pid {p} is dead"),
                None => "no pid recorded".to_string(),
            };
            tx.execute("UPDATE sessions SET state='restorable',pid=NULL,restore_reason='crash',updated_at=?1 WHERE id=?2", params![now, id])?;
            report
                .fsck_fixes
                .push(format!("session {name}: {state} → restorable ({why})"));
        }
    }
    // 3. dirty worktrees per project: inline, or on a worker once the report is stored
    let mut st = tx.prepare_cached("SELECT path FROM projects")?;
    let repos: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    drop(st);
    if dirty_scan == DirtyScan::Inline {
        report.dirty_worktrees = dirty_worktrees(&repos);
    }
    // 4. requested device runs are process-backed and cannot survive an engine restart.
    //    Preserve the durable record, but close its lifecycle instead of showing it as live.
    {
        let changed = tx.execute(
            "UPDATE device_runs SET state='stopped',finished_at=?1 WHERE state IN ('building','running')",
            [&now],
        )?;
        if changed > 0 {
            report
                .fsck_fixes
                .push(format!("closed {changed} interrupted device run(s)"));
        }
    }
    // 5. generated hook directories with no session behind them. Stale ones accumulate one
    //    per closed session and make `.relay/hooks` misreport the live fleet.
    {
        let mut st = tx.prepare_cached(
            "SELECT p.path, COALESCE(GROUP_CONCAT(s.name, char(10)), '') FROM projects p
             LEFT JOIN sessions s ON s.project_id = p.id AND s.state != 'closed'
             GROUP BY p.id",
        )?;
        let repos = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(st);
        let mut swept = 0;
        for (repo, live) in repos {
            let live: Vec<String> = live.lines().map(str::to_string).collect();
            swept += crate::hooks::sweep_hook_dirs(Path::new(&repo), &live);
        }
        if swept > 0 {
            report.fsck_fixes.push(format!("removed {swept} stale hook director(ies)"));
        }
    }
    // 6. tasks stuck in active with no live session (phase 7 offers the reset in the UI)
    {
        let mut st = tx.prepare_cached(
            "SELECT t.id FROM tasks t WHERE t.col = 'active' AND t.deleted_at IS NULL AND NOT EXISTS (
                SELECT 1 FROM task_sessions ts JOIN sessions s ON s.id = ts.session_id
                WHERE ts.task_id = t.id AND s.state IN ('spawning','running','idle','blocked','parked'))")?;
        report.tasks_reset_offered = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    }
    // Repair claims left by older builds before pruning their completion evidence.
    // A later claim renewal belongs to new work and must survive an older Done.
    {
        let mut statement = tx.prepare_cached(
            "DELETE FROM claims WHERE session_id IN (SELECT id FROM sessions WHERE state='closed')
             OR EXISTS (SELECT 1 FROM audit a WHERE a.session_id=claims.session_id
                 AND a.op='session.done' AND a.kind='ok' AND a.ts>=claims.updated_at)
             OR EXISTS (SELECT 1 FROM task_sessions ts WHERE ts.session_id=claims.session_id
                 AND ts.completed_at>=claims.updated_at)
             RETURNING project_id,session,path",
        )?;
        let released = statement.query_map([], |row| Ok((
            row.get::<_, Id>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
        )))?.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (project, session, path) in &released {
            tx.execute(
                "UPDATE overlaps SET active=0,last_seen=?1 WHERE project_id=?2 AND path=?3
                 AND kind='claim' AND active=1
                 AND EXISTS (SELECT 1 FROM json_each(overlaps.sessions) WHERE value=?4)",
                params![now, project, path, session],
            )?;
        }
        if !released.is_empty() {
            report.fsck_fixes.push(format!("released {} stale file claim(s)", released.len()));
        }
    }
    // 7. audit retention (SPEC §14). The log is append-only within its window, not forever: a
    //    long-lived store is mostly old rows nothing can act on any more, and every one of them
    //    is a page the connection pages past. Rows that are half of an undo pair are kept
    //    whatever their age — that link is the record of what was reversed.
    let pruned = {
        let days: i64 = tx
            .query_row(
                "SELECT value FROM settings WHERE path='audit.retention_days'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|value| serde_json::from_str::<i64>(&value).ok())
            .unwrap_or(DEFAULT_AUDIT_RETENTION_DAYS);
        if days > 0 {
            let cutoff = crate::time::days_ago(days);
            tx.execute(
                "DELETE FROM audit
                  WHERE ts < ?1
                    AND undo_of IS NULL
                    AND undone_by IS NULL
                    AND id NOT IN (SELECT undo_of FROM audit WHERE undo_of IS NOT NULL)",
                [&cutoff],
            )?
        } else {
            0
        }
    };
    if pruned > 0 {
        report
            .fsck_fixes
            .push(format!("pruned {pruned} audit row(s) past the retention window"));
    }
    let json = serde_json::to_string(&report)?;
    tx.execute("INSERT INTO meta(key, value) VALUES ('recovery.last', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [json])?;
    tx.commit()?;
    // Deleted pages are free space, not reclaimed space. Compact only when a prune actually
    // happened, so the usual launch pays nothing for it.
    if pruned > 0 {
        if let Err(error) = conn.execute_batch("VACUUM") {
            tracing::warn!(error = %error, "could not compact the store after pruning the audit log");
        }
    }
    drop(conn);
    if dirty_scan == DirtyScan::Deferred {
        match engine.arc() {
            Some(engine) => {
                let mut deferred = report.clone();
                std::thread::Builder::new()
                    .name("recovery-dirty-scan".into())
                    .spawn(move || {
                        crate::background_priority();
                        deferred.dirty_worktrees = dirty_worktrees(&repos);
                        if deferred.dirty_worktrees.is_empty() {
                            return;
                        }
                        if let Ok(json) = serde_json::to_string(&deferred) {
                            let conn = engine.store.lock();
                            let _ = conn.execute(
                                "INSERT INTO meta(key, value) VALUES ('recovery.last', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                                [json],
                            );
                        }
                    })
                    .ok();
            }
            None => report.dirty_worktrees = dirty_worktrees(&repos),
        }
    }
    if !report.reaped_pids.is_empty() || !report.fsck_fixes.is_empty() {
        tracing::info!(
            reaped = report.reaped_pids.len(),
            fixes = report.fsck_fixes.len(),
            "crash recovery acted"
        );
    }
    Ok(report)
}

/// Pooled checkouts with uncommitted changes. The primary checkout being dirty is normal;
/// pooled ones with changes are worth a flag. A `git status` per checkout.
fn dirty_worktrees(repos: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for repo in repos {
        if let Ok(wts) = crate::worktree::list(Path::new(repo)) {
            let pool = crate::worktree::pool_dir(Path::new(repo)).display().to_string();
            out.extend(wts.into_iter().filter(|w| w.dirty && w.path.starts_with(&pool)).map(|w| w.path));
        }
    }
    out
}

pub fn last(conn: &rusqlite::Connection) -> Result<Option<RecoveryReport>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'recovery.last'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(raw
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| serde_json::from_value(v).ok()))
}
