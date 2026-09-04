//! Phase 9 notification center and one-shot dashboard aggregation.

use crate::engine::{Ctx, Engine, IntoBus};
use relay_bus::error::Confirm;
use relay_bus::ops::notify::*;
use relay_bus::types::{Notification, NotifyCategory};
use relay_bus::Empty;
use rusqlite::Row;
use serde_json::json;

fn category(value: &str) -> NotifyCategory {
    match value {
        "agent_done" => NotifyCategory::AgentDone,
        "agent_blocked" => NotifyCategory::AgentBlocked,
        "guardrail" => NotifyCategory::Guardrail,
        "integration" => NotifyCategory::Integration,
        "provider" => NotifyCategory::Provider,
        "disk" => NotifyCategory::Disk,
        _ => NotifyCategory::System,
    }
}

fn category_str(value: NotifyCategory) -> &'static str {
    match value {
        NotifyCategory::AgentDone => "agent_done",
        NotifyCategory::AgentBlocked => "agent_blocked",
        NotifyCategory::Guardrail => "guardrail",
        NotifyCategory::Integration => "integration",
        NotifyCategory::Provider => "provider",
        NotifyCategory::Disk => "disk",
        NotifyCategory::System => "system",
    }
}

fn notification_row(row: &Row) -> rusqlite::Result<Notification> {
    let link = row
        .get::<_, Option<String>>("link")?
        .and_then(|value| serde_json::from_str::<Confirm>(&value).ok());
    Ok(Notification {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        category: category(&row.get::<_, String>("category")?),
        title: row.get("title")?,
        body: row.get("body")?,
        link,
        read: row.get::<_, i64>("read")? != 0,
        created_at: row.get("created_at")?,
    })
}

fn list(
    ctx: &Ctx,
    project_id: Option<i64>,
    unread: Option<bool>,
    category_: Option<NotifyCategory>,
    limit: Option<u32>,
) -> Result<Vec<Notification>, relay_bus::BusError> {
    let mut sql = String::from("SELECT * FROM notifications WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(project_id) = project_id {
        sql.push_str(" AND project_id=?");
        args.push(Box::new(project_id));
    }
    if unread.unwrap_or(false) {
        sql.push_str(" AND read=0");
    }
    if let Some(category_) = category_ {
        sql.push_str(" AND category=?");
        args.push(Box::new(category_str(category_).to_string()));
    }
    sql.push_str(" ORDER BY created_at DESC,id DESC LIMIT ?");
    args.push(Box::new(i64::from(limit.unwrap_or(100).min(500))));
    let mut stmt = ctx.tx().prepare(&sql).bus()?;
    let notifications = stmt
        .query_map(
            rusqlite::params_from_iter(args.iter().map(|value| value.as_ref())),
            notification_row,
        )
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    Ok(notifications)
}

pub fn register(engine: &mut Engine) {
    engine.register::<List>(|ctx, payload| {
        Ok(ListOut {
            notifications: list(
                ctx,
                payload.project_id,
                payload.unread_only,
                payload.category,
                payload.limit,
            )?,
        })
    });
    engine.register::<Ack>(|ctx: &mut Ctx, payload| {
        let changed = ctx
            .tx()
            .execute(
                "UPDATE notifications SET read=1 WHERE id=?1",
                [payload.notification_id],
            )
            .bus()?;
        if changed == 0 {
            return Err(relay_bus::BusError::not_found(
                "notify.not_found",
                format!("no notification {}", payload.notification_id),
            ));
        }
        ctx.emit(
            "notify.changed",
            json!({"notification_id":payload.notification_id,"read":true}),
        );
        Ok(Empty {})
    });
    engine.register::<AckAll>(|ctx: &mut Ctx, payload| {
        if let Some(category) = payload.category {
            ctx.tx()
                .execute(
                    "UPDATE notifications SET read=1 WHERE read=0 AND category=?1",
                    [category_str(category)],
                )
                .bus()?;
        } else {
            ctx.tx()
                .execute("UPDATE notifications SET read=1 WHERE read=0", [])
                .bus()?;
        }
        ctx.emit(
            "notify.changed",
            json!({"all":true,"category":payload.category}),
        );
        Ok(Empty {})
    });
    engine.register::<SettingsGetNotify>(|ctx, _| {
        crate::handlers::settings::get(ctx.tx(), Some("notifications"))
    });
    engine.register::<SettingsSetNotify>(|ctx: &mut Ctx, payload| {
        if !payload.patch.is_object() {
            return Err(relay_bus::BusError::invalid(
                "notify.settings",
                "patch must be an object",
            ));
        }
        let before = crate::handlers::settings::get(ctx.tx(), Some("notifications"))?;
        let mut next = before.clone();
        crate::handlers::settings::merge_value(&mut next, &payload.patch);
        crate::handlers::settings::set(ctx.tx(), "notifications", &next, &ctx.now)?;
        ctx.set_undo("notify.settings.set", json!({"patch":before}), None);
        ctx.emit(
            "settings.changed",
            json!({"path":"notifications","value":next}),
        );
        Ok(next)
    });
    engine.register::<DashboardGet>(|ctx, _| {
        let project_ids = ctx.tx().prepare_cached("SELECT id FROM projects ORDER BY id").bus()?
            .query_map([], |row| row.get::<_, i64>(0)).bus()?
            .collect::<rusqlite::Result<Vec<_>>>().bus()?;
        let mut sessions_live = Vec::new();
        for project_id in project_ids { sessions_live.extend(crate::awareness::peers(ctx.tx(), project_id, None, Some(ctx.engine()))?); }
        sessions_live.retain(|peer| matches!(peer.state, relay_bus::types::SessionState::Spawning | relay_bus::types::SessionState::Running | relay_bus::types::SessionState::Idle | relay_bus::types::SessionState::Blocked));
        let mut stmt = ctx.tx().prepare_cached("SELECT * FROM tasks WHERE col='in_review' AND deleted_at IS NULL ORDER BY updated_at DESC,id DESC").bus()?;
        let mut rows = stmt.query([]).bus()?;
        let mut in_review = Vec::new();
        while let Some(row) = rows.next().bus()? { in_review.push(crate::handlers::task::row_task(ctx.tx(), row).bus()?); }
        let holds_open = ctx.tx().prepare_cached("SELECT * FROM holds WHERE state='open' ORDER BY created_at DESC,id DESC").bus()?
            .query_map([], crate::guardrail::hold_row).bus()?
            .collect::<rusqlite::Result<Vec<_>>>().bus()?;
        let projects = ctx.tx().prepare_cached(
            "SELECT p.id,p.name,p.base_branch,
             COALESCE(SUM(CASE WHEN t.col!='done' THEN 1 ELSE 0 END),0),
             COALESCE(SUM(CASE WHEN t.col='ready' THEN 1 ELSE 0 END),0),
             COALESCE(SUM(CASE WHEN t.col='active' THEN 1 ELSE 0 END),0),
             COALESCE(SUM(CASE WHEN t.col='in_review' THEN 1 ELSE 0 END),0),
             COALESCE(SUM(CASE WHEN t.col='done' AND datetime(t.updated_at)>=datetime('now','-7 days') THEN 1 ELSE 0 END),0),
             (SELECT COUNT(*) FROM sessions s WHERE s.project_id=p.id AND s.state IN ('spawning','running','idle','blocked')),
             (SELECT COUNT(*) FROM sessions s WHERE s.project_id=p.id AND s.state='blocked')
             FROM projects p LEFT JOIN tasks t ON t.project_id=p.id AND t.deleted_at IS NULL
             GROUP BY p.id,p.name,p.base_branch ORDER BY p.name,p.id"
        ).bus()?.query_map([], |row| Ok(DashboardProject {
            project_id: row.get(0)?, name: row.get(1)?, base_branch: row.get(2)?, tasks_open: row.get(3)?,
            ready: row.get(4)?, active: row.get(5)?, in_review: row.get(6)?, done_recent: row.get(7)?,
            live_sessions: row.get(8)?, blocked_sessions: row.get(9)?,
        })).bus()?.collect::<rusqlite::Result<Vec<_>>>().bus()?;
        let notifications = list(ctx, None, Some(false), None, Some(8))?;
        let resources = crate::handlers::app::resources(ctx.tx(), ctx.engine())?;
        Ok(DashboardOut { projects, sessions_live, in_review, holds_open, notifications, resources })
    });
}
