//! `device.leases`, `device.claim`, `device.release`, and the lease plumbing the rest of the
//! engine calls: `device.run`, the guardrail hook's shell commands, session exit and close.
//! The lease map itself is `crate::device_lease`.

use crate::device_lease::{self, Holder, Kind, Lease};
use crate::engine::{Ctx, Engine, IntoBus};
use crate::sessions;
use relay_bus::error::BusError;
use relay_bus::ops::device::{Claim, Leases, LeasesOut, Release, ReleaseOut};
use relay_bus::types::{Id, SessionState};
use rusqlite::{Connection, OptionalExtension};
use std::time::Duration;

pub const ACQUIRED: &str = "device.lease.acquired";
pub const RELEASED: &str = "device.lease.released";

pub fn register(e: &mut Engine) {
    e.register_unlocked::<Leases>(|ctx, _| {
        let released = ctx.read(|conn| Ok(prune(ctx.engine(), conn)))?;
        for lease in released { ctx.emit(RELEASED, lease.event()); }
        Ok(LeasesOut { leases: ctx.engine().device_leases.list().iter().map(Lease::view).collect() })
    });

    e.register::<Claim>(|ctx: &mut Ctx, p| {
        let device = p.device.as_deref().map(str::trim).unwrap_or(device_lease::ANY_DEVICE);
        if device.is_empty() || device.chars().any(char::is_whitespace) {
            return Err(BusError::invalid("device.serial", "device must be a serial from device.list, or omitted for every device"));
        }
        if p.action.trim().is_empty() {
            return Err(BusError::invalid("device.claim_action", "say what you will be doing with the device"));
        }
        let minutes = p.minutes.unwrap_or(device_lease::CLAIM_DEFAULT_MINUTES).clamp(1, device_lease::CLAIM_MAX_MINUTES);
        let holder = actor_holder(ctx)?;
        let lease = Lease::new(device, holder, Kind::Claim, &p.action, Some(Duration::from_secs(u64::from(minutes) * 60)));
        acquire(ctx, lease.clone())?;
        Ok(ctx.engine().device_leases.holder_of(&lease.device).map(|held| held.view()).unwrap_or_else(|| lease.view()))
    });

    e.register::<Release>(|ctx: &mut Ctx, p| {
        let holder = actor_holder(ctx)?;
        let user = ctx.actor_session_id().is_none() && !ctx.actor.is_agent();
        let device = p.device.as_deref().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
        let leases = &ctx.engine().device_leases;
        if let Some(device) = &device {
            if let Some(held) = leases.list().into_iter().find(|lease| &lease.device == device) {
                if let Kind::Run(run_id) = held.kind {
                    return Err(BusError::conflict("device.lease_run", format!("device {device} is held by device run {run_id}"))
                        .with_hint("stop the run with device.run.stop; its lease goes with it"));
                }
                if held.holder != holder && !user {
                    return Err(device_lease::busy(&held).with_hint("only the holder or the Relay user can release a lease"));
                }
            }
        }
        let released = leases.release_where(|lease| {
            !matches!(lease.kind, Kind::Run(_))
                && device.as_ref().is_none_or(|device| &lease.device == device)
                && (lease.holder == holder || (user && device.is_some()))
        });
        let views = released.iter().map(Lease::view).collect();
        for lease in released { ctx.emit(RELEASED, lease.event()); }
        Ok(ReleaseOut { released: views })
    });
}

/// The calling session, or the user.
fn actor_holder(ctx: &Ctx) -> Result<Holder, BusError> {
    match ctx.actor_session_id() {
        Some(id) => {
            let row = sessions::by_id(ctx.tx(), id)?.ok_or_else(|| BusError::internal("bound session vanished"))?;
            Ok(Holder::Session { id, name: row.session.name })
        }
        None if ctx.actor.is_agent() => Err(BusError::actor("device leases need a bound agent session")),
        None => Ok(Holder::User),
    }
}

/// Who holds a run started from `worktree`: the live session that owns that checkout, or the user.
pub(crate) fn worktree_holder(conn: &Connection, project_id: Id, worktree: &str) -> Result<Holder, BusError> {
    let owner: Option<(Id, String)> = conn
        .prepare_cached("SELECT id,name FROM sessions WHERE project_id=?1 AND worktree=?2 AND state!='closed' ORDER BY id LIMIT 1")
        .bus()?
        .query_row(rusqlite::params![project_id, worktree], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()
        .bus()?;
    Ok(owner.map_or(Holder::User, |(id, name)| Holder::Session { id, name }))
}

/// Drop leases whose holder is gone: a run no longer live, a session that is not running.
pub(crate) fn prune(engine: &Engine, conn: &Connection) -> Vec<Lease> {
    if engine.device_leases.is_empty() {
        return Vec::new();
    }
    let live_runs: Vec<Id> = engine.device_runs.lock().unwrap().keys().copied().collect();
    engine.device_leases.prune(|lease| match (lease.kind, &lease.holder) {
        (Kind::Run(id), _) => !live_runs.contains(&id),
        (_, Holder::Session { id, .. }) => !session_running(conn, *id),
        (_, Holder::User) => false,
    })
}

fn session_running(conn: &Connection, id: Id) -> bool {
    let state: Option<String> = conn
        .prepare_cached("SELECT state FROM sessions WHERE id=?1")
        .and_then(|mut statement| statement.query_row([id], |row| row.get(0)).optional())
        .ok()
        .flatten();
    state.is_some_and(|state| sessions::is_live(sessions::parse_state(&state)) || sessions::parse_state(&state) == SessionState::Created)
}

/// Take a lease inside a request: prune, acquire, queue the events. `device.busy` on conflict.
pub(crate) fn acquire(ctx: &mut Ctx, lease: Lease) -> Result<(), BusError> {
    let mut released = prune(ctx.engine(), ctx.tx());
    let event = lease.event();
    let taken = ctx.engine().device_leases.acquire(lease, &mut released);
    for gone in released { ctx.emit(RELEASED, gone.event()); }
    if taken? { ctx.emit(ACQUIRED, event); }
    Ok(())
}

/// Refuse early, before any slow work, when `holder` could not take `device`.
pub(crate) fn check(ctx: &mut Ctx, device: &str, holder: &Holder) -> Result<(), BusError> {
    for gone in prune(ctx.engine(), ctx.tx()) { ctx.emit(RELEASED, gone.event()); }
    ctx.engine().device_leases.check(device, holder)
}

/// The guardrail hook saw an agent about to run `command`. A device-writing command takes the
/// session's shell lease, or is refused with `device.busy` naming whoever holds the device.
pub(crate) fn gate_command(ctx: &mut Ctx, session_id: Id, session: &str, command: &str) -> Result<(), BusError> {
    let Some(found) = device_lease::device_command(command) else { return Ok(()) };
    let holder = Holder::Session { id: session_id, name: session.to_string() };
    acquire(ctx, Lease::new(&found.device, holder, Kind::Shell, &found.action, Some(device_lease::SHELL_RUNNING)))
}

/// A tool call finished (PostToolUse) or the turn stopped: a shell lease it took now only
/// lasts for the grace period.
pub(crate) fn command_finished(engine: &Engine, session_id: Id, command: Option<&str>, turn_ended: bool) {
    if turn_ended || command.is_some_and(|command| device_lease::device_command(command).is_some()) {
        engine.device_leases.shell_finished(session_id);
    }
}

/// Release a finished run's lease, from its worker (no request open).
pub fn release_run(engine: &Engine, run_id: Id) {
    for lease in engine.device_leases.release_run(run_id) {
        engine.emit_system(RELEASED, lease.event());
    }
}

/// A session exited or closed: its shell leases and claims go with it, with events.
pub fn release_session(engine: &Engine, session_id: Id) {
    for lease in engine.device_leases.release_session(session_id) {
        engine.emit_system(RELEASED, lease.event());
    }
}
