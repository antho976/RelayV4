use relay_bus::{Registry, Request, Actor, Response, BusError};
use relay_bus::registry::{valid_name, OpKind, Audit};

#[test]
fn registry_is_well_formed() {
    let reg = Registry::global();
    assert!(reg.len() >= 150, "expected the full catalogue, got {}", reg.len());
    for e in reg.entries() {
        assert!(valid_name(e.name), "{}", e.name);
        assert!(!e.meta.summary.is_empty(), "{} has no summary", e.name);
        assert!((1..=12).contains(&e.meta.phase), "{} phase {}", e.name, e.meta.phase);
        if e.meta.kind == OpKind::Query {
            assert_eq!(e.meta.audit, Audit::Never, "{} is a query but audited", e.name);
        }
    }
    // Every op named in BUS.md §15 as phase-1 executable is registered.
    for n in ["bus.ping", "bus.schema", "bus.ops", "bus.subscribe", "bus.unsubscribe", "app.version", "app.status",
              "app.reconcile", "audit.list", "audit.get", "settings.get", "settings.set", "settings.reset",
              "workspace.create", "workspace.list", "workspace.update", "workspace.remove",
              "project.add", "project.list", "project.get", "project.update", "project.remove"] {
        assert!(reg.get(n).is_some(), "missing {n}");
    }
}

#[test]
fn schema_renders_and_is_stable() {
    let a = relay_bus::schema::render_pretty();
    let b = relay_bus::schema::render_pretty();
    assert_eq!(a, b, "schema rendering must be deterministic");
    let v: serde_json::Value = serde_json::from_str(&a).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["op_count"].as_u64().unwrap() as usize, Registry::global().len());
    assert!(v["ops"]["task.create"]["payload"].is_object());
    assert!(relay_bus::schema::render_op("task.create").is_some());
    assert!(relay_bus::schema::render_op("nope.nope").is_none());
}

#[test]
fn envelope_round_trips() {
    let r = Request::new(Actor::agent("brisk-otter"), "task.get", serde_json::json!({"task_id": 1}));
    let s = serde_json::to_string(&r).unwrap();
    let back: Request = serde_json::from_str(&s).unwrap();
    assert_eq!(back.op, "task.get");
    assert_eq!(back.actor, Actor::Agent("brisk-otter".into()));
    // system is parseable (internal) — doors reject it, the type does not.
    assert_eq!(Actor::parse("system").unwrap(), Actor::System);
    assert!(Actor::parse("agent:").is_err());
    assert!(Actor::parse("root").is_err());
    let e = Response::err(r.id, BusError::not_implemented("task.get", 7));
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"ok\":false"));
    let u = Response::unparsed(BusError::parse("eof"));
    assert!(serde_json::to_string(&u).unwrap().contains("\"id\":null"));
}

/// BUS.md §0.5: the committed schema file must equal the generated one.
/// Regenerate with `cargo run -p relay-bus --example dump_schema > schema/bus.v1.json`.
#[test]
fn committed_schema_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/bus.v1.json");
    let committed = std::fs::read_to_string(path).expect("schema/bus.v1.json exists");
    let generated = relay_bus::schema::render_pretty();
    assert!(committed == generated,
        "schema/bus.v1.json is stale — regenerate: cargo run -p relay-bus --example dump_schema > schema/bus.v1.json");
}
