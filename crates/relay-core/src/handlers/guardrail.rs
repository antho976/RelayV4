//! `guardrail.*` (SPEC §5, BUS.md §9): pure checks, enforcing gates, durable holds,
//! confirmation/rejection and effective configuration.

use crate::engine::{Ctx, Engine, IntoBus};
use crate::guardrail::{self, Decision, GateRequest};
use crate::handlers::workspace::get_project;
use crate::sessions;
use relay_bus::envelope::{Request, Response};
use relay_bus::error::BusError;
use relay_bus::ops::guardrail::*;
use relay_bus::types::{GateKind, HoldState, Id, Verdict};
use rusqlite::params;
use serde_json::json;
use std::path::Path;

pub fn register(engine: &mut Engine) {
    engine.register::<ConfigGet>(|ctx, payload| {
        guardrail::config(ctx.tx(), payload.project_id)
    });

    engine.register::<ConfigSet>(|ctx: &mut Ctx, payload| {
        if !payload.patch.is_object() {
            return Err(BusError::invalid("guardrail.config", "patch must be an object"));
        }
        if let Some(project_id) = payload.project_id {
            get_project(ctx.tx(), project_id)?;
            ctx.set_project(project_id);
        }
        let path = payload.project_id.map(|id| format!("guardrails.projects.{id}"));
        let before = if let Some(path) = &path {
            crate::handlers::settings::get(ctx.tx(), Some(path))?
        } else {
            serde_json::to_value(guardrail::config(ctx.tx(), None)?)
                .map_err(crate::engine::internal)?
        };
        let mut merged = if before.is_object() { before.clone() } else { json!({}) };
        crate::handlers::settings::merge_value(&mut merged, &payload.patch);
        if let Some(path) = &path {
            crate::handlers::settings::set(ctx.tx(), path, &merged, &ctx.now.clone())?;
        } else {
            // Write global keys independently: replacing `guardrails` as a whole would
            // delete the separately stored `guardrails.projects.*` overrides.
            let object = merged.as_object().ok_or_else(|| {
                BusError::invalid("guardrail.config", "merged config must be an object")
            })?;
            for (key, value) in object {
                crate::handlers::settings::set(
                    ctx.tx(),
                    &format!("guardrails.{key}"),
                    value,
                    &ctx.now.clone(),
                )?;
            }
        }
        // Typed read validates the merged result. An error rolls the handler transaction back.
        let effective = guardrail::config(ctx.tx(), payload.project_id)?;
        ctx.set_undo("guardrail.config.set", json!({"project_id": payload.project_id, "patch": before}), None);
        ctx.emit("guardrail.config_changed", json!({"project_id": payload.project_id}));
        Ok(effective)
    });

    engine.register::<Check>(|ctx, payload| {
        let project = get_project(ctx.tx(), payload.project_id)?;
        // A dry run has to judge the tree the write would land in. Judging the project root
        // would answer a question the caller did not ask (D111).
        let worktree = crate::handlers::file::default_worktree(ctx, &project, None)?;
        let decision = guardrail::evaluate(
            ctx.tx(),
            &GateRequest {
                actor: &ctx.actor,
                project_id: project.id,
                worktree: &worktree,
                kind: payload.kind,
                path: payload.path.as_deref(),
                new_text: payload.new_text.as_deref(),
                diff: payload.diff.as_deref(),
                command: payload.command.as_deref(),
                skip_policy: None,
            },
        )?;
        Ok(check_out(decision))
    });

    engine.register::<Explain>(|ctx, payload| {
        // `guardrail.check` answers one action. A plan is decided before the first action, and
        // learning at minute zero that it will trip a cap is worth more than learning it at
        // commit time (D117). Pure: it evaluates, it never holds and never writes.
        let project = get_project(ctx.tx(), payload.project_id)?;
        let worktree = crate::handlers::file::default_worktree(ctx, &project, None)?;
        let cfg = guardrail::config(ctx.tx(), Some(project.id))?;
        let mut worst = Verdict::Allow;
        let mut note = |verdict: Verdict| {
            // refuse beats hold beats allow
            worst = match (worst, verdict) {
                (Verdict::Refuse, _) | (_, Verdict::Refuse) => Verdict::Refuse,
                (Verdict::Hold, _) | (_, Verdict::Hold) => Verdict::Hold,
                _ => Verdict::Allow,
            };
        };

        let requested = payload.paths.unwrap_or_default();
        let mut paths = Vec::with_capacity(requested.len());
        for path in &requested {
            // No text yet, so this reports the policies that can be judged from a path alone:
            // protected paths, write roots, and whether a shape gate will demand full text.
            let decision = guardrail::evaluate(
                ctx.tx(),
                &GateRequest {
                    actor: &ctx.actor,
                    project_id: project.id,
                    worktree: &worktree,
                    kind: GateKind::Write,
                    path: Some(path),
                    new_text: None,
                    diff: Some(""),
                    command: None,
                    skip_policy: None,
                },
            );
            let item = match decision {
                Ok(guardrail::Decision::Allow) => ExplainItem {
                    subject: path.clone(), verdict: Verdict::Allow, policy: None, message: None,
                },
                Ok(guardrail::Decision::Refuse(error)) => ExplainItem {
                    subject: path.clone(), verdict: Verdict::Refuse,
                    policy: Some(error.code.clone()), message: Some(error.message.clone()),
                },
                Ok(guardrail::Decision::Hold { policy, error, .. }) => ExplainItem {
                    subject: path.clone(), verdict: Verdict::Hold,
                    policy: Some(policy), message: Some(error.message.clone()),
                },
                Err(error) => ExplainItem {
                    subject: path.clone(), verdict: Verdict::Refuse,
                    policy: Some(error.code.clone()), message: Some(error.message.clone()),
                },
            };
            note(item.verdict);
            paths.push(item);
        }

        let mut commands = Vec::new();
        for command in payload.commands.unwrap_or_default() {
            let decision = guardrail::evaluate(
                ctx.tx(),
                &GateRequest {
                    actor: &ctx.actor,
                    project_id: project.id,
                    worktree: &worktree,
                    kind: GateKind::Exec,
                    path: None,
                    new_text: None,
                    diff: None,
                    command: Some(&command),
                    skip_policy: None,
                },
            )?;
            let item = match decision {
                guardrail::Decision::Allow => ExplainItem {
                    subject: command, verdict: Verdict::Allow, policy: None, message: None,
                },
                guardrail::Decision::Refuse(error) => ExplainItem {
                    subject: command, verdict: Verdict::Refuse,
                    policy: Some(error.code.clone()), message: Some(error.message.clone()),
                },
                guardrail::Decision::Hold { policy, error, .. } => ExplainItem {
                    subject: command, verdict: Verdict::Hold,
                    policy: Some(policy), message: Some(error.message.clone()),
                },
            };
            note(item.verdict);
            commands.push(item);
        }

        let files = requested.len() as u32;
        let lines = payload.lines.unwrap_or(0);
        let over_caps = files > cfg.caps.files || lines > cfg.caps.lines;
        if over_caps {
            note(Verdict::Refuse);
        }
        Ok(ExplainOut {
            verdict: worst,
            paths,
            commands,
            files,
            lines,
            caps: cfg.caps.clone(),
            over_caps,
            write_roots: guardrail::write_roots(&cfg, &worktree)
                .iter().map(|root| root.display().to_string()).collect(),
        })
    });

    engine.register::<Gate>(|ctx: &mut Ctx, payload| gate(ctx, payload, None));

    engine.register::<HoldsList>(|ctx, payload| {
        let mut sql = String::from("SELECT * FROM holds WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(project_id) = payload.project_id {
            sql.push_str(" AND project_id = ?");
            args.push(Box::new(project_id));
        }
        if let Some(session) = payload.session {
            sql.push_str(" AND session = ?");
            args.push(Box::new(session));
        }
        if payload.open_only.unwrap_or(true) {
            sql.push_str(" AND state = 'open'");
        }
        sql.push_str(" ORDER BY id DESC LIMIT 1000");
        let mut stmt = ctx.tx().prepare(&sql).bus()?;
        let holds = stmt
            .query_map(
                rusqlite::params_from_iter(args.iter().map(|arg| arg.as_ref())),
                guardrail::hold_row,
            )
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        Ok(HoldsListOut { holds })
    });

    engine.register::<HoldGet>(|ctx, payload| {
        let hold = guardrail::hold_by_id(ctx.tx(), payload.hold_id)?;
        let mut request = guardrail::frozen_request(ctx.tx(), payload.hold_id)?;
        request.token = None;
        Ok(HoldGetOut { hold, request })
    });
    engine.register::<Confirm>(confirm);
    engine.register::<Reject>(reject);
}

fn gate(ctx: &mut Ctx, payload: GateIn, skip_policy: Option<&str>) -> Result<GateOut, BusError> {
    let session = sessions::by_name(ctx.tx(), &payload.session)?;
    if let Some(actor_session_id) = ctx.actor_session_id() {
        if actor_session_id != session.session.id {
            return Err(BusError::not_own("session"));
        }
    }
    let project_id = session.session.project_id;
    ctx.set_project(project_id);
    ctx.set_session(session.session.id);
    let decision = guardrail::evaluate(
        ctx.tx(),
        &GateRequest {
            actor: &ctx.actor,
            project_id,
            worktree: Path::new(&session.session.worktree),
            kind: payload.kind,
            path: payload.path.as_deref(),
            new_text: payload.new_text.as_deref(),
            diff: payload.diff.as_deref(),
            command: payload.command.as_deref(),
            skip_policy,
        },
    )?;
    match decision {
        Decision::Allow => {
            // A command that writes to an Android device takes this session's device lease, or
            // is refused naming the session already using the device (device_lease).
            if let (GateKind::Exec, Some(command)) = (payload.kind, payload.command.as_deref()) {
                super::device_lease::gate_command(ctx, session.session.id, &session.session.name, command)?;
            }
            Ok(GateOut { verdict: Verdict::Allow, error: None, hold_id: None })
        }
        Decision::Refuse(error) => {
            guardrail::insert_refusal_notification(ctx.tx(), project_id, &ctx.actor, &error, &ctx.now)?;
            ctx.commit_error(None);
            ctx.emit("guardrail.refused", json!({"session": payload.session, "code": error.code}));
            ctx.emit("notify.new", json!({"category": "guardrail", "project_id": project_id}));
            Err(error)
        }
        Decision::Hold { policy, mut error, details } => {
            let frozen = Request::new(
                ctx.actor.clone(),
                "guardrail.gate",
                serde_json::to_value(&payload).map_err(crate::engine::internal)?,
            )
            .with_id(ctx.req_id);
            let hold_id = guardrail::insert_hold(
                ctx.tx(),
                &frozen,
                project_id,
                Some(session.session.id),
                Some(&session.session.name),
                &policy,
                &details,
                &ctx.now,
            )?;
            error = error.with_confirm("guardrail.confirm", json!({"hold_id": hold_id}));
            ctx.commit_error(Some(hold_id));
            ctx.emit("guardrail.held", json!({"hold_id": hold_id, "session": payload.session, "policy": policy}));
            ctx.emit("notify.new", json!({"category": "guardrail", "project_id": project_id, "hold_id": hold_id}));
            Err(error)
        }
    }
}

fn confirm(ctx: &mut Ctx, payload: ConfirmIn) -> Result<ConfirmOut, BusError> {
    let hold = guardrail::hold_by_id(ctx.tx(), payload.hold_id)?;
    if hold.state != HoldState::Open {
        return Err(BusError::conflict(
            "guardrail.hold_resolved",
            format!("hold {} is {:?}", hold.id, hold.state),
        ));
    }
    let frozen = guardrail::frozen_request(ctx.tx(), hold.id)?;
    if frozen.op != "guardrail.gate" {
        let outcome = match ctx.replay_registered(&frozen.op, frozen.payload.clone(), hold.actor.clone(), hold.policy.clone()) {
            Ok(value) => Response::ok(ctx.req_id, value),
            Err(error) => Response::err(ctx.req_id, error),
        };
        ctx.tx().execute(
            "UPDATE holds SET state = 'confirmed', resolved_at = ?1, resolved_by = ?2 WHERE id = ?3 AND state = 'open'",
            params![ctx.now, ctx.actor.to_string(), hold.id],
        ).bus()?;
        let resolved = guardrail::hold_by_id(ctx.tx(), hold.id)?;
        if let Some(project_id) = hold.project_id { ctx.set_project(project_id); }
        if let Some(session_id) = hold.session_id { ctx.set_session(session_id); }
        ctx.audit_as(frozen.op, hold.actor.clone());
        ctx.emit("guardrail.resolved", json!({"hold_id": hold.id, "state": "confirmed", "by": ctx.actor.to_string()}));
        return Ok(ConfirmOut { hold: resolved, outcome });
    }
    let gate_payload: GateIn = serde_json::from_value(frozen.payload.clone())
        .map_err(|e| BusError::schema("guardrail.gate", e))?;
    let session = sessions::by_name(ctx.tx(), &gate_payload.session)?;
    let decision = guardrail::evaluate(
        ctx.tx(),
        &GateRequest {
            // Other policies still judge the original actor. Confirmation skips only the
            // policy named by this hold; protected paths/caps can still refuse.
            actor: &hold.actor,
            project_id: session.session.project_id,
            worktree: Path::new(&session.session.worktree),
            kind: gate_payload.kind,
            path: gate_payload.path.as_deref(),
            new_text: gate_payload.new_text.as_deref(),
            diff: gate_payload.diff.as_deref(),
            command: gate_payload.command.as_deref(),
            skip_policy: Some(&hold.policy),
        },
    )?;

    let outcome = match decision {
        Decision::Allow => Response::ok(
            ctx.req_id,
            serde_json::to_value(GateOut { verdict: Verdict::Allow, error: None, hold_id: None })
                .map_err(crate::engine::internal)?,
        ),
        Decision::Refuse(error) => Response::err(ctx.req_id, error),
        Decision::Hold { policy, mut error, details } => {
            let nested = Request::new(
                hold.actor.clone(),
                "guardrail.gate",
                frozen.payload.clone(),
            );
            let next_id = guardrail::insert_hold(
                ctx.tx(),
                &nested,
                session.session.project_id,
                Some(session.session.id),
                Some(&session.session.name),
                &policy,
                &details,
                &ctx.now,
            )?;
            error = error.with_confirm("guardrail.confirm", json!({"hold_id": next_id}));
            Response::err(ctx.req_id, error)
        }
    };
    ctx.tx()
        .execute(
            "UPDATE holds SET state = 'confirmed', resolved_at = ?1, resolved_by = ?2 WHERE id = ?3 AND state = 'open'",
            params![ctx.now, ctx.actor.to_string(), hold.id],
        )
        .bus()?;
    let resolved = guardrail::hold_by_id(ctx.tx(), hold.id)?;
    ctx.set_project(session.session.project_id);
    ctx.set_session(session.session.id);
    ctx.audit_as(frozen.op, hold.actor.clone());
    ctx.emit("guardrail.resolved", json!({"hold_id": hold.id, "state": "confirmed", "by": ctx.actor.to_string()}));
    if let (Some(project_id), Some(session)) = (hold.project_id, hold.session.as_deref()) {
        if let Some(message) = super::notes::send_system_priority(
            ctx.tx(), project_id, session,
            &format!("Guardrail hold {} was confirmed by {}.", hold.id, ctx.actor),
            None, &ctx.now,
        )? {
            ctx.emit("mailbox.new", serde_json::to_value(message).bus()?);
        }
    }
    Ok(ConfirmOut { hold: resolved, outcome })
}

/// Apply the phase-4 policy engine to a mutation that is itself a bus op. A hold freezes
/// that exact op so confirmation can replay it in the same transaction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn enforce(
    ctx: &mut Ctx, project_id: Id, worktree: &Path, kind: GateKind, path: Option<&str>,
    new_text: Option<&str>, diff: Option<&str>, command: Option<&str>,
) -> Result<(), BusError> {
    let decision = guardrail::evaluate(ctx.tx(), &GateRequest {
        actor: &ctx.actor, project_id, worktree, kind, path, new_text, diff, command,
        skip_policy: ctx.skip_policy(),
    })?;
    match decision {
        Decision::Allow => Ok(()),
        Decision::Refuse(error) => {
            guardrail::insert_refusal_notification(ctx.tx(), project_id, &ctx.actor, &error, &ctx.now)?;
            ctx.commit_error(None);
            ctx.emit("guardrail.refused", json!({"op": ctx.op, "code": error.code}));
            ctx.emit("notify.new", json!({"category": "guardrail", "project_id": project_id}));
            Err(error)
        }
        Decision::Hold { policy, mut error, details } => {
            let frozen = Request::new(ctx.actor.clone(), ctx.op, ctx.payload().clone()).with_id(ctx.req_id);
            let session_id = ctx.actor_session_id();
            let session = session_id.and_then(|id| ctx.tx().query_row(
                "SELECT name FROM sessions WHERE id = ?1", [id], |r| r.get::<_, String>(0),
            ).ok());
            let hold_id = guardrail::insert_hold(
                ctx.tx(), &frozen, project_id, session_id, session.as_deref(), &policy, &details, &ctx.now,
            )?;
            error = error.with_confirm("guardrail.confirm", json!({"hold_id": hold_id}));
            ctx.commit_error(Some(hold_id));
            ctx.emit("guardrail.held", json!({"hold_id": hold_id, "op": ctx.op, "policy": policy}));
            ctx.emit("notify.new", json!({"category": "guardrail", "project_id": project_id, "hold_id": hold_id}));
            Err(error)
        }
    }
}

fn reject(ctx: &mut Ctx, payload: RejectIn) -> Result<RejectOut, BusError> {
    let hold = guardrail::hold_by_id(ctx.tx(), payload.hold_id)?;
    if hold.state != HoldState::Open {
        return Err(BusError::conflict(
            "guardrail.hold_resolved",
            format!("hold {} is {:?}", hold.id, hold.state),
        ));
    }
    ctx.tx()
        .execute(
            "UPDATE holds SET state = 'rejected', resolved_at = ?1, resolved_by = ?2,
                 details = json_set(details, '$.rejection_reason', ?3)
             WHERE id = ?4 AND state = 'open'",
            params![ctx.now, ctx.actor.to_string(), payload.reason, hold.id],
        )
        .bus()?;
    let resolved = guardrail::hold_by_id(ctx.tx(), hold.id)?;
    if let Some(project_id) = hold.project_id {
        ctx.set_project(project_id);
    }
    if let Some(session_id) = hold.session_id {
        ctx.set_session(session_id);
    }
    if let (Some(project_id), Some(session)) = (hold.project_id, hold.session.as_deref()) {
        if let Some(message) = super::notes::send_system_priority(
            ctx.tx(), project_id, session,
            &format!("Guardrail hold {} was rejected: {}", hold.id, payload.reason.as_deref().unwrap_or("no reason provided")),
            None, &ctx.now,
        )? {
            ctx.emit("mailbox.new", serde_json::to_value(message).bus()?);
        }
    }
    ctx.emit("guardrail.resolved", json!({"hold_id": hold.id, "state": "rejected", "by": ctx.actor.to_string(), "reason": payload.reason}));
    Ok(RejectOut { hold: resolved })
}

fn check_out(decision: Decision) -> CheckOut {
    match decision {
        Decision::Allow => CheckOut { verdict: Verdict::Allow, error: None },
        Decision::Refuse(error) => CheckOut { verdict: Verdict::Refuse, error: Some(error) },
        Decision::Hold { error, .. } => CheckOut { verdict: Verdict::Hold, error: Some(error) },
    }
}
