//! The rebuilt board (task #31): sub-tasks with roll-up, first-class types, labels, and
//! stored blocked-by / duplicate-of edges. Every assertion crosses the bus door, because the
//! CLI and agents reach this model through exactly the same ops the UI does.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap()
        .success());
}
fn call(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(actor, op, payload), Door::InProcess)
}
fn ok(engine: &Engine, op: &str, payload: Value) -> Value {
    call(engine, Actor::User, op, payload)
        .into_result()
        .unwrap_or_else(|e| panic!("{op}: {} {}", e.code, e.message))
}
fn code(response: Response) -> String {
    response.error.expect("expected error").code
}
fn ids(value: &Value) -> Vec<i64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect()
}

struct Fixture {
    _root: tempfile::TempDir,
    engine: Arc<Engine>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        let repo = ws.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "board@relay.test"]);
        git(&repo, &["config", "user.name", "Board"]);
        std::fs::write(repo.join("README.md"), "board\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let engine = Engine::new(
            Instance::Test,
            Store::open(&root.path().join("store/store.db"), false).unwrap(),
        );
        ok(&engine, "workspace.create", json!({"path":ws}));
        ok(&engine, "project.add", json!({"workspace_id":1,"path":repo}));
        Self {
            _root: root,
            engine,
        }
    }
    fn task(&self, title: &str, extra: Value) -> Value {
        let mut payload = json!({"project_id":1,"title":title});
        let map = payload.as_object_mut().unwrap();
        for (k, v) in extra.as_object().unwrap() {
            map.insert(k.clone(), v.clone());
        }
        ok(&self.engine, "task.create", payload)
    }
}

#[test]
fn task_note_and_module_draft_expectations_reject_stale_writes_atomically() {
    let f = Fixture::new();
    for (kind, id_key, field, original) in [
        (
            "task",
            "task_id",
            "body",
            f.task("Draft", json!({"body":"original"})),
        ),
        (
            "notes",
            "note_id",
            "body",
            ok(
                &f.engine,
                "notes.create",
                json!({"project_id":1,"body":"original"}),
            ),
        ),
        (
            "module",
            "module_id",
            "name",
            ok(
                &f.engine,
                "module.create",
                json!({"project_id":1,"name":"original"}),
            ),
        ),
    ] {
        let op = format!("{kind}.update");
        let gate = Arc::new(std::sync::Barrier::new(3));
        let mut writers = Vec::new();
        for body in ["first writer", "second writer"] {
            let engine = f.engine.clone();
            let gate = gate.clone();
            let op = op.clone();
            let payload = json!({id_key:original["id"],field:body,"expected":{field:"original"}});
            writers.push(std::thread::spawn(move || {
                gate.wait();
                call(&engine, Actor::User, &op, payload)
            }));
        }
        gate.wait();
        let replies: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(replies.iter().filter(|r| r.ok).count(), 1);
        assert_eq!(
            replies
                .iter()
                .filter(|r| !r.ok)
                .next()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .code,
            format!("{kind}.edit_conflict")
        );
        let winner = &replies
            .iter()
            .find(|r| r.ok)
            .unwrap()
            .result
            .as_ref()
            .unwrap()[field];
        assert_eq!(
            ok(
                &f.engine,
                &format!("{kind}.get"),
                json!({id_key:original["id"]})
            )[field],
            *winner
        );

        // A legacy caller can still patch, while a stale editor cannot overwrite its change.
        ok(
            &f.engine,
            &op,
            json!({id_key:original["id"],field:"legacy edit"}),
        );
        assert_eq!(
            code(call(
                &f.engine,
                Actor::User,
                &op,
                json!({id_key:original["id"],field:"stale draft","expected":{field:winner}})
            )),
            format!("{kind}.edit_conflict")
        );
        assert_eq!(
            code(call(
                &f.engine,
                Actor::User,
                &op,
                json!({id_key:original["id"],"expected":{"typo":null}})
            )),
            format!("{kind}.expected_field")
        );
    }
}

#[test]
fn draft_expectations_preserve_nulls_and_ignore_unmentioned_fields() {
    let f = Fixture::new();
    let task = f.task("Draft", json!({}));
    ok(
        &f.engine,
        "task.update",
        json!({"task_id":task["id"],"priority":"high"}),
    );
    let updated = ok(
        &f.engine,
        "task.update",
        json!({"task_id":task["id"],"body":"edited","expected":{"body":"","size":null,"module_id":null,"type":task["type"]}}),
    );
    assert_eq!(updated["priority"], "high");
    let note = ok(
        &f.engine,
        "notes.create",
        json!({"project_id":1,"body":"original"}),
    );
    assert!(note["title"].is_null());
    let updated = ok(
        &f.engine,
        "notes.update",
        json!({"note_id":note["id"],"title":"Named","expected":{"title":null,"pinned":false}}),
    );
    assert_eq!(updated["title"], "Named");
    assert_eq!(
        code(call(
            &f.engine,
            Actor::User,
            "notes.update",
            json!({"note_id":note["id"],"title":null,"expected":{"title":null}})
        )),
        "notes.edit_conflict"
    );
}

#[test]
fn sub_tasks_roll_up_and_stay_real_cards() {
    let f = Fixture::new();
    let e = &f.engine;
    let parent = f.task("Rebuild the board", json!({"type":"feature"}));
    let child_a = f.task("Model", json!({"parent_id":parent["id"],"type":"chore"}));
    let child_b = f.task("Board", json!({"parent_id":parent["id"]}));
    let grandchild = f.task("Card", json!({"parent_id":child_b["id"]}));

    assert_eq!(child_a["depth"], 1);
    assert_eq!(grandchild["depth"], 2);
    assert_eq!(grandchild["parent_id"], child_b["id"]);

    // A sub-task is a task: it has its own column and its own card.
    assert_eq!(child_a["column"], "backlog");
    let listed = ok(e, "task.list", json!({"project_id":1}))["tasks"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(listed, 4, "children are listed alongside their parent");

    // Roll-up counts the whole subtree, not just the direct children.
    let reread = ok(e, "task.get", json!({"task_id":parent["id"]}));
    assert_eq!(reread["rollup"]["total"], 3);
    assert_eq!(reread["rollup"]["done"], 0);
    assert_eq!(ids(&reread["children"]), vec![child_a["id"].as_i64().unwrap(), child_b["id"].as_i64().unwrap()]);

    ok(e, "task.move", json!({"task_id":grandchild["id"],"column":"in_review"}));
    ok(e, "task.approve", json!({"task_id":grandchild["id"],"sha":"deadbeef"}));
    let reread = ok(e, "task.get", json!({"task_id":parent["id"]}));
    assert_eq!(reread["rollup"], json!({"total":3,"done":1}));

    // Filtering by parent is how a group is pulled out of the columns.
    let direct = ok(e, "task.children", json!({"task_id":parent["id"]}))["tasks"].clone();
    assert_eq!(direct.as_array().unwrap().len(), 2);
    let whole = ok(
        e,
        "task.children",
        json!({"task_id":parent["id"],"recursive":true}),
    )["tasks"]
        .clone();
    assert_eq!(whole.as_array().unwrap().len(), 3);
    let roots = ok(e, "task.list", json!({"project_id":1,"parent_id":null}))["tasks"].clone();
    assert_eq!(roots.as_array().unwrap().len(), 1);
    assert_eq!(roots[0]["id"], parent["id"]);
}

#[test]
fn nesting_is_capped_and_cycles_refused() {
    let f = Fixture::new();
    let e = &f.engine;
    let a = f.task("A", json!({}));
    let b = f.task("B", json!({"parent_id":a["id"]}));
    let c = f.task("C", json!({"parent_id":b["id"]}));

    // TASK_DEPTH_MAX is 3: a root plus two levels. The fourth is refused, not silently flattened.
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.create",
            json!({"project_id":1,"title":"D","parent_id":c["id"]})
        )),
        "task.depth"
    );
    // Re-parenting checks the height of the subtree that would move, not just the task itself.
    // `b` is one level tall, so hanging it under `a`'s child would push `c` past the cap.
    let d = f.task("D", json!({"parent_id":a["id"]}));
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.parent.set",
            json!({"task_id":b["id"],"parent_id":d["id"]})
        )),
        "task.depth"
    );
    // And a task cannot be re-parented under its own descendant.
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.parent.set",
            json!({"task_id":a["id"],"parent_id":c["id"]})
        )),
        "task.parent_cycle"
    );
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.parent.set",
            json!({"task_id":a["id"],"parent_id":a["id"]})
        )),
        "task.parent_cycle"
    );
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.parent.set",
            json!({"task_id":a["id"],"parent_id":b["id"]})
        )),
        "task.parent_cycle"
    );
}

#[test]
fn promote_and_detach_survive_undo() {
    let f = Fixture::new();
    let e = &f.engine;
    let parent = f.task("Group", json!({}));
    let loose = f.task("Loose", json!({}));

    let promoted = ok(
        e,
        "task.parent.set",
        json!({"task_id":loose["id"],"parent_id":parent["id"]}),
    );
    assert_eq!(promoted["parent_id"], parent["id"]);
    assert_eq!(
        ok(e, "task.get", json!({"task_id":parent["id"]}))["rollup"]["total"],
        1
    );

    let row = ok(e, "audit.list", json!({"op_prefix":"task.parent.set","limit":1}))["rows"][0].clone();
    ok(e, "audit.undo", json!({"audit_id":row["id"]}));
    let back = ok(e, "task.get", json!({"task_id":loose["id"]}));
    assert!(back["parent_id"].is_null());
    assert_eq!(
        ok(e, "task.get", json!({"task_id":parent["id"]}))["rollup"]["total"],
        0
    );
}

#[test]
fn types_and_labels_are_separate_layers() {
    let f = Fixture::new();
    let e = &f.engine;
    let bug = f.task("Crash on launch", json!({"type":"bug","labels":["ui","regression"]}));
    let spike = f.task("Try gix", json!({"type":"spike"}));
    f.task("Plain", json!({}));

    assert_eq!(bug["type"], "bug");
    assert_eq!(spike["type"], "spike");
    assert_eq!(
        ok(e, "task.get", json!({"task_id":bug["id"]}))["type"],
        "bug"
    );
    // No type given = the neutral default, never an invented classification.
    assert_eq!(
        ok(e, "task.list", json!({"project_id":1,"type":"task"}))["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(bug["labels"], json!(["regression", "ui"]));

    let labels = ok(e, "task.label.list", json!({"project_id":1}))["labels"].clone();
    assert_eq!(labels.as_array().unwrap().len(), 2);
    assert_eq!(labels[0]["name"], "regression");

    let tagged = ok(
        e,
        "task.label.add",
        json!({"task_id":spike["id"],"label":"UI"}),
    );
    // The stored name wins, so one label reads one way everywhere on the board.
    assert_eq!(tagged["labels"], json!(["ui"]));
    // Label names are case-insensitive, so "UI" reuses the existing row rather than forking it.
    assert_eq!(
        ok(e, "task.label.list", json!({"project_id":1}))["labels"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        ok(e, "task.list", json!({"project_id":1,"label":"ui"}))["tasks"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let untagged = ok(
        e,
        "task.label.remove",
        json!({"task_id":spike["id"],"label":"ui"}),
    );
    assert_eq!(untagged["labels"], json!([]));
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.label.add",
            json!({"task_id":spike["id"],"label":"   "})
        )),
        "task.label"
    );
    // Changing a type is a normal patch, and it round-trips through undo like every other field.
    ok(e, "task.update", json!({"task_id":spike["id"],"type":"chore"}));
    let row = ok(e, "audit.list", json!({"op_prefix":"task.update","limit":1}))["rows"][0].clone();
    ok(e, "audit.undo", json!({"audit_id":row["id"]}));
    assert_eq!(
        ok(e, "task.get", json!({"task_id":spike["id"]}))["type"],
        "spike"
    );
}

#[test]
fn blocked_by_is_stored_both_ways_and_duplicate_of_is_single_valued() {
    let f = Fixture::new();
    let e = &f.engine;
    let a = f.task("Ship the card", json!({}));
    let b = f.task("Ship the model", json!({}));
    let c = f.task("Older report", json!({}));

    let blocked = ok(
        e,
        "task.relate",
        json!({"task_id":a["id"],"relation":"blocked_by","other_id":b["id"]}),
    );
    assert_eq!(ids(&blocked["blocked_by"]), vec![b["id"].as_i64().unwrap()]);
    // The inverse is a read of the same row — `blocks` is never stored separately.
    assert_eq!(
        ids(&ok(e, "task.get", json!({"task_id":b["id"]}))["blocks"]),
        vec![a["id"].as_i64().unwrap()]
    );

    ok(
        e,
        "task.relate",
        json!({"task_id":a["id"],"relation":"duplicate_of","other_id":c["id"]}),
    );
    let redirected = ok(
        e,
        "task.relate",
        json!({"task_id":a["id"],"relation":"duplicate_of","other_id":b["id"]}),
    );
    assert_eq!(redirected["duplicate_of"], b["id"]);

    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.relate",
            json!({"task_id":a["id"],"relation":"blocked_by","other_id":a["id"]})
        )),
        "task.relation_self"
    );

    let dropped = ok(
        e,
        "task.unrelate",
        json!({"task_id":a["id"],"relation":"blocked_by","other_id":b["id"]}),
    );
    assert_eq!(dropped["blocked_by"], json!([]));
    let row = ok(e, "audit.list", json!({"op_prefix":"task.unrelate","limit":1}))["rows"][0].clone();
    ok(e, "audit.undo", json!({"audit_id":row["id"]}));
    assert_eq!(
        ids(&ok(e, "task.get", json!({"task_id":a["id"]}))["blocked_by"]),
        vec![b["id"].as_i64().unwrap()]
    );
    // Deleting the other end hides the edge without losing it; restoring brings it back.
    ok(e, "task.delete", json!({"task_id":b["id"]}));
    assert_eq!(
        ok(e, "task.get", json!({"task_id":a["id"]}))["blocked_by"],
        json!([])
    );
    ok(e, "task.restore", json!({"task_id":b["id"]}));
    assert_eq!(
        ids(&ok(e, "task.get", json!({"task_id":a["id"]}))["blocked_by"]),
        vec![b["id"].as_i64().unwrap()]
    );
}

#[test]
fn dispatch_fans_out_to_sub_tasks() {
    let f = Fixture::new();
    let e = &f.engine;
    let parent = f.task("Body of work", json!({"type":"feature"}));
    let child = f.task("Half of it", json!({"parent_id":parent["id"]}));
    let grandchild = f.task("A quarter", json!({"parent_id":child["id"]}));
    let done = f.task("Already shipped", json!({"parent_id":parent["id"]}));
    ok(e, "task.move", json!({"task_id":done["id"],"column":"in_review"}));
    ok(e, "task.approve", json!({"task_id":done["id"],"sha":"cafe"}));

    let result = ok(
        e,
        "task.dispatch",
        json!({"task_id":parent["id"],"create":{"project_id":1,"provider":"claude","role":"builder"},"fanout":true}),
    );
    assert_eq!(result["task"]["column"], "active");
    let fanned = result["fanned"].as_array().unwrap();
    assert_eq!(fanned.len(), 2, "the done sub-task is left alone");
    let touched: Vec<i64> = fanned.iter().map(|d| d["task"]["id"].as_i64().unwrap()).collect();
    assert!(touched.contains(&child["id"].as_i64().unwrap()));
    assert!(touched.contains(&grandchild["id"].as_i64().unwrap()));
    // One session each, so every card on the board names the agent working it.
    let names: Vec<&str> = fanned.iter().map(|d| d["session"]["name"].as_str().unwrap()).collect();
    assert_ne!(names[0], names[1]);
    assert_ne!(names[0], result["session"]["name"].as_str().unwrap());
    assert_eq!(
        ok(e, "task.get", json!({"task_id":child["id"]}))["state"],
        "dispatched"
    );

    // A single sub-task dispatches on its own, and fanout without `create` is a typed refusal
    // rather than quietly piling several tasks onto one session.
    let lone = f.task("Standalone", json!({"parent_id":parent["id"]}));
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.dispatch",
            json!({"task_id":lone["id"],"session":result["session"]["name"],"fanout":true})
        )),
        "task.fanout_target"
    );
    let solo = ok(
        e,
        "task.dispatch",
        json!({"task_id":lone["id"],"session":result["session"]["name"]}),
    );
    assert_eq!(solo["task"]["state"], "dispatched");
    assert_eq!(solo["fanned"], json!([]));
    // task.list by session is the "which cards is this agent on" query the board filters with.
    let name = result["session"]["name"].as_str().unwrap();
    let mine = ok(e, "task.list", json!({"project_id":1,"session":name}))["tasks"].clone();
    assert_eq!(mine.as_array().unwrap().len(), 2);
}

#[test]
fn existing_tasks_keep_neutral_board_defaults() {
    let f = Fixture::new();
    let e = &f.engine;
    // Rows written before v16 are typed `task`, parentless, unlabelled and unrelated; the
    // migration must not invent a classification for them.
    let task = f.task("Written before the redesign", json!({}));
    let reread = ok(e, "task.get", json!({"task_id":task["id"]}));
    assert_eq!(reread["type"], "task");
    assert!(reread["parent_id"].is_null());
    assert_eq!(reread["depth"], 0);
    assert_eq!(reread["labels"], json!([]));
    assert_eq!(reread["blocked_by"], json!([]));
    assert_eq!(reread["blocks"], json!([]));
    assert!(reread["duplicate_of"].is_null());
    assert_eq!(reread["rollup"], json!({"total":0,"done":0}));
}
