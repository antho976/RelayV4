//! "Where it left off": what a stopped session was doing, on its slate, before anyone resumes it.
//!
//! The session row carries its intent, branch and task; the last `session.done` report lives
//! only in the notification it raised, and a restorable session's dirty worktree and stop reason
//! come from `session.restorable`. Panes reconciled together share one round of reads.
use super::{label, rows, text, Ui};
use crate::terminal::Pane;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};

struct Request {
    pane: Weak<Pane>,
    serial: u64,
    session: Value,
}

thread_local! {
    static PENDING: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
    static SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

/// `ts` (RFC 3339) as a person says it: "just now", "5 min ago", "3 h ago", "yesterday", "Oct 4".
pub(super) fn relative_time(ts: &str) -> String {
    let Ok(then) = glib::DateTime::from_iso8601(ts, None) else {
        return ts.to_string();
    };
    let Ok(now) = glib::DateTime::now_utc() else {
        return ts.to_string();
    };
    let minutes = now.difference(&then).as_minutes();
    if minutes < 1 {
        return String::from("just now");
    }
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    if minutes < 24 * 60 {
        return format!("{} h ago", minutes / 60);
    }
    let local = |d: &glib::DateTime| d.to_local().ok();
    let (Some(now), Some(then)) = (local(&now), local(&then)) else {
        return ts.to_string();
    };
    if minutes < 48 * 60 && now.add_days(-1).is_ok_and(|y| y.ymd() == then.ymd()) {
        return String::from("yesterday");
    }
    let pattern = if now.year() == then.year() { "%b %-d" } else { "%b %-d, %Y" };
    then.format(pattern).map(|s| s.to_string()).unwrap_or_else(|_| ts.to_string())
}

/// `ts` as a local clock reading, "Oct 4, 14:02".
fn local_stamp(ts: &str) -> Option<String> {
    let local = glib::DateTime::from_iso8601(ts, None).ok()?.to_local().ok()?;
    local.format("%b %-d, %H:%M").ok().map(|s| s.to_string())
}

/// What `session.done` / a blocked report said last, newest first in `notifications`.
fn last_report<'a>(session: &Value, notifications: &'a [Value]) -> Option<&'a Value> {
    let prefix = format!("{} ", text(session, "name"));
    let since = text(session, "created_at");
    notifications.iter().find(|n| {
        matches!(text(n, "category"), "agent_done" | "agent_blocked")
            && text(n, "title").starts_with(&prefix)
            && text(n, "created_at") >= since
            && !matches!(text(n, "body").trim(), "" | "Agent turn completed")
    })
}

impl Ui {
    /// Fill `pane`'s slate with what `session` was working on when it stopped.
    pub(super) fn describe_stopped_session(self: &Rc<Self>, pane: &Rc<Pane>, session: &Value) {
        let serial = pane.begin_context();
        PENDING.with(|p| {
            p.borrow_mut().push(Request {
                pane: Rc::downgrade(pane),
                serial,
                session: session.clone(),
            })
        });
        if SCHEDULED.with(|s| s.replace(true)) {
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            SCHEDULED.with(|s| s.set(false));
            let requests = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
            ui.describe_batch(requests).await;
        });
    }

    async fn describe_batch(&self, requests: Vec<Request>) {
        let sessions: Vec<&Value> = requests.iter().map(|r| &r.session).collect();
        let mut restorable = BTreeMap::new();
        if sessions.iter().any(|s| text(s, "state") == "restorable") {
            if let Ok(data) = self.call("session.restorable", json!({})).await {
                for item in rows(&data, "sessions") {
                    if let Some(id) = item["session"]["id"].as_i64() {
                        restorable.insert(id, item);
                    }
                }
            }
        }
        let projects: BTreeSet<i64> = sessions.iter().filter_map(|s| s["project_id"].as_i64()).collect();
        let mut reports = BTreeMap::new();
        for project in projects {
            if let Ok(data) = self.call("notify.list", json!({"project_id":project,"limit":200})).await {
                reports.insert(project, rows(&data, "notifications"));
            }
        }
        let task_ids: BTreeSet<i64> = sessions.iter().filter_map(|s| s["task_id"].as_i64()).collect();
        let mut tasks = BTreeMap::new();
        for id in task_ids {
            if let Ok(task) = self.call("task.get", json!({"task_id":id})).await {
                tasks.insert(id, task);
            }
        }
        for request in requests {
            let Some(pane) = request.pane.upgrade() else { continue };
            let Some(card) = pane.context_card(request.serial) else { continue };
            let session = &request.session;
            let notifications = session["project_id"]
                .as_i64()
                .and_then(|p| reports.get(&p))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let restore = session["id"].as_i64().and_then(|id| restorable.get(&id));
            let task = session["task_id"].as_i64().and_then(|id| tasks.get(&id));
            render(card, session, task, last_report(session, notifications), restore);
        }
    }
}

fn render(card: &gtk::Box, session: &Value, task: Option<&Value>, report: Option<&Value>, restore: Option<&Value>) {
    card.append(&label("WHERE IT LEFT OFF", "slate-context-title"));
    let grid = gtk::Grid::new();
    grid.set_column_spacing(14);
    grid.set_row_spacing(7);
    let mut row = 0;
    let mut add = |key: &str, value: &str, class: &str, lines: i32| {
        // Caption and first line of the value share a baseline, whatever their sizes.
        let caption = label(key, "slate-context-key");
        caption.set_valign(gtk::Align::BaselineFill);
        grid.attach(&caption, 0, row, 1, 1);
        let shown = label(value, "slate-context-value");
        if !class.is_empty() {
            shown.add_css_class(class);
        }
        shown.set_valign(gtk::Align::BaselineFill);
        shown.set_hexpand(true);
        shown.set_wrap(true);
        shown.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        shown.set_lines(lines);
        shown.set_ellipsize(gtk::pango::EllipsizeMode::End);
        shown.set_max_width_chars(40);
        shown.set_tooltip_text(Some(value));
        grid.attach(&shown, 1, row, 1, 1);
        row += 1;
    };
    let mut recorded = false;
    if let Some(id) = session["task_id"].as_i64() {
        let title = task.map(|t| text(t, "title").trim()).unwrap_or_default();
        let value = if title.is_empty() { format!("#{id}") } else { format!("#{id} {title}") };
        add("TASK", &value, "", 2);
        recorded = true;
    }
    let intent = text(session, "intent").trim();
    if !intent.is_empty() {
        add("DOING", intent, "", 2);
        recorded = true;
    }
    if let Some(report) = report {
        let mut value = text(report, "body").trim().to_string();
        let when = relative_time(text(report, "created_at"));
        if text(report, "category") == "agent_blocked" {
            value = format!("Blocked: {value}");
        }
        add("LAST REPORT", &format!("{value} · {when}"), "slate-context-report", 3);
        recorded = true;
    }
    let branch = text(session, "branch");
    if !branch.is_empty() {
        let dirty = restore.is_some_and(|r| r["worktree_dirty"] == true);
        let value = if dirty { format!("{branch} · uncommitted changes") } else { branch.to_string() };
        add("BRANCH", &value, if dirty { "slate-context-dirty" } else { "" }, 2);
    }
    let stopped = match text(session, "state") {
        "restorable" => Some(match restore.map(|r| text(r, "reason")) {
            Some("crash") => String::from("Relay stopped unexpectedly"),
            Some("app_restart") | None => String::from("Relay was closed"),
            Some(other) => other.replace('_', " "),
        }),
        "exited" => Some(match session["exit_code"].as_i64() {
            Some(0) | None => String::from("The process exited"),
            Some(code) => format!("The process exited with code {code}"),
        }),
        _ => None,
    };
    if let Some(stopped) = stopped {
        add("STOPPED", &stopped, "", 2);
    }
    let active = [text(session, "last_output_at"), text(session, "updated_at")]
        .into_iter()
        .find(|ts| !ts.is_empty());
    if let Some(ts) = active {
        let mut value = relative_time(ts);
        // "Sep 27 · Sep 27, 15:48" says the date twice; the stamp alone is enough.
        if let Some(stamp) = local_stamp(ts).filter(|s| *s != value) {
            value = if stamp.starts_with(&value) { stamp } else { format!("{value} · {stamp}") };
        }
        add("LAST ACTIVE", &value, "", 1);
    }
    card.append(&grid);
    if !recorded {
        let none = label("No task, intent or report was recorded.", "slate-context-empty");
        none.set_wrap(true);
        card.append(&none);
    }
    card.set_visible(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ago(minutes: i64) -> String {
        let then = glib::DateTime::now_utc().unwrap().add_minutes(-(minutes as i32)).unwrap();
        then.format_iso8601().unwrap().to_string()
    }

    #[test]
    fn relative_time_reads_like_speech() {
        assert_eq!(relative_time(&ago(0)), "just now");
        assert_eq!(relative_time(&ago(5)), "5 min ago");
        assert_eq!(relative_time(&ago(3 * 60 + 10)), "3 h ago");
        assert_eq!(relative_time("not a time"), "not a time");
        assert!(!relative_time(&ago(9 * 24 * 60)).contains("ago"));
    }

    #[test]
    fn last_report_skips_placeholders_and_older_sessions() {
        let session = json!({"name":"builder-1","created_at":"2026-10-01T00:00:00Z"});
        let notes = vec![
            json!({"category":"agent_done","title":"builder-1 finished","body":"Agent turn completed","created_at":"2026-10-04T00:00:00Z"}),
            json!({"category":"agent_done","title":"builder-10 finished","body":"Other session","created_at":"2026-10-03T00:00:00Z"}),
            json!({"category":"agent_blocked","title":"builder-1 needs attention","body":"Waiting on review","created_at":"2026-10-02T00:00:00Z"}),
            json!({"category":"agent_done","title":"builder-1 finished","body":"Earlier life","created_at":"2026-09-01T00:00:00Z"}),
        ];
        assert_eq!(text(last_report(&session, &notes).unwrap(), "body"), "Waiting on review");
        assert!(last_report(&session, &notes[..2]).is_none());
    }
}
