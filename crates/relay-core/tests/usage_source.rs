//! Which source `usage.get` prefers (RA-663). The test instance never reads provider state, so
//! this drives a dev-instance engine, the path production runs, with the provider homes pointed
//! at a temp dir. It sets them in this process's environment, which is why it is a file of its
//! own with one test: nothing else in this binary runs while the variables change.

mod common;

use common::call;
use relay_core::engine::Engine;
use relay_core::{Instance, Store};
use serde_json::json;

#[test]
fn usage_get_prefers_the_status_line_file_and_keeps_reports_where_there_is_none() {
    let home = tempfile::tempdir().unwrap();
    let claude = home.path().join("claude-config");
    std::fs::create_dir_all(claude.join(".claude")).unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("CLAUDE_CONFIG_DIR", &claude);
    // An empty Codex home: no rollouts, so Codex has only what was reported.
    std::env::set_var("CODEX_HOME", home.path().join("codex"));

    let engine = Engine::new(Instance::Dev, Store::open_memory().unwrap());
    // Reports as `usage.report` stores them, dated after the file below.
    engine.store.with_tx(|tx| {
        tx.execute("INSERT INTO workspaces(path,name,ord,created_at,updated_at) VALUES ('/w','w',0,'t','t')", [])?;
        tx.execute("INSERT INTO projects(workspace_id,path,name,base_branch,ord,created_at,updated_at) VALUES (1,'/w/p','p','main',0,'t','t')", [])?;
        for (name, provider, used) in [("calm-otter", "claude", 41), ("sly-egret", "codex", 12)] {
            tx.execute(
                "INSERT INTO sessions(name,project_id,provider,state,usage,created_at,updated_at) VALUES (?1,1,?2,'running',?3,'t','2099-01-01T00:00:00Z')",
                rusqlite::params![name, provider, json!({"five_hour":{"used_pct":used}}).to_string()],
            )?;
        }
        Ok(())
    }).unwrap();

    let get = |payload| call(&engine, "usage.get", payload).into_result().unwrap()["usage"].clone();
    // No status-line file yet: the report stands.
    let usage = get(json!({"provider":"claude"}));
    assert_eq!(usage[0]["windows"]["five_hour"]["used_pct"], 41, "{usage}");

    // What the Claude status line writes (`hooks.rs`): `$CLAUDE_CONFIG_DIR/.claude/relay-usage.json`.
    std::fs::write(
        claude.join(".claude/relay-usage.json"),
        r#"{"timestamp":"2026-01-02T03:04:05Z","rate_limits":{"five_hour":{"used_percentage":73,"resets_at":4102444800}}}"#,
    ).unwrap();
    let usage = get(json!({}));
    let by = |provider: &str| usage.as_array().unwrap().iter().find(|item| item["provider"] == provider).cloned()
        .unwrap_or_else(|| panic!("no {provider} usage in {usage}"));
    // The file wins over the report, even an older file over a newer report: the status line
    // rewrites it on every refresh, and nothing in Relay sends `usage.report` on its own.
    assert_eq!(by("claude")["windows"]["five_hour"]["used_pct"], 73.0);
    assert_eq!(by("claude")["windows"]["five_hour"]["resets_at"], 4102444800_u64);
    assert_eq!(by("claude")["taken_at"], "2026-01-02T03:04:05Z");
    // Codex left nothing to read, so its report is what usage.get answers with.
    assert_eq!(by("codex")["windows"]["five_hour"]["used_pct"], 12);
    assert_eq!(by("codex")["taken_at"], "2099-01-01T00:00:00Z");
    // The provider filter applies to both sources.
    let usage = get(json!({"provider":"codex"}));
    assert_eq!(usage.as_array().unwrap().len(), 1, "{usage}");
    assert_eq!(usage[0]["provider"], "codex");
}
