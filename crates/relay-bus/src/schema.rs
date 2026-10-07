//! Renders `schema/bus.v1.json` (BUS.md §0.5): envelope shapes, every op's payload and
//! result, and the shared `$defs`. CI compares the committed file with [`render_pretty`].

use crate::envelope::{Event, Frame, Request, Response};
use crate::error::BusError;
use crate::registry::{OpInfo, Registry};
use schemars::generate::SchemaSettings;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// The full schema document as JSON. It cannot change within a build, so it is generated once
/// per process; `bus.schema {}` then costs a clone rather than a walk of every op.
pub fn render() -> Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(generate).clone()
}

fn generate() -> Value {
    let settings = SchemaSettings::draft2020_12().with(|s| {
        s.definitions_path = "#/$defs/".into();
        s.inline_subschemas = false;
    });
    let mut g = settings.into_generator();

    let envelope = json!({
        "request": g.subschema_for::<Request>(),
        "response": g.subschema_for::<Response>(),
        "event": g.subschema_for::<Event>(),
        "frame": g.subschema_for::<Frame>(),
        "error": g.subschema_for::<BusError>(),
    });

    let mut ops = Map::new();
    for e in Registry::global().entries() {
        let payload = (e.payload_schema)(&mut g);
        let result = (e.result_schema)(&mut g);
        let mut info = serde_json::to_value(OpInfo::from_entry(e, false)).expect("OpInfo serializes");
        let obj = info.as_object_mut().expect("object");
        obj.remove("name");
        obj.remove("implemented");
        obj.insert("payload".into(), payload.to_value());
        obj.insert("result".into(), result.to_value());
        ops.insert(e.name.to_string(), info);
    }

    let defs: Map<String, Value> = g.take_definitions(true).into_iter().collect();

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "relay://bus/v1",
        "title": "RELAY v4 command bus",
        "description": "Generated from crates/relay-bus by `relay schema`. Do not edit; see docs/engine/BUS.md.",
        "version": crate::envelope::ENVELOPE_V,
        "op_count": ops.len(),
        "envelope": envelope,
        "ops": ops,
        "$defs": defs,
    })
}

/// Pretty JSON with a trailing newline — byte-stable for the CI diff.
pub fn render_pretty() -> String {
    let mut s = serde_json::to_string_pretty(&render()).expect("schema serializes");
    s.push('\n');
    s
}

/// One op's `{payload, result}` schema, self-contained (defs inlined), for `bus.schema {op}`.
pub fn render_op(name: &str) -> Option<Value> {
    let e = Registry::global().get(name)?;
    let payload = inlined(e.payload_schema);
    let result = inlined(e.result_schema);
    // `implemented` is a property of a *running* engine, not of the schema. Reporting it here
    // contradicted `bus.ops` for every op; the static document simply does not answer it.
    let mut meta = serde_json::to_value(OpInfo::from_entry(e, false)).expect("OpInfo serializes");
    if let Some(object) = meta.as_object_mut() {
        object.remove("implemented");
    }
    Some(json!({ "op": name, "meta": meta, "payload": payload, "result": result }))
}

/// One schema with every subschema inlined. A recursive type (`Entry.children`) cannot be, so
/// schemars leaves a `$ref` to it; its definition goes in this schema's own `$defs`, so the
/// schema still stands alone when a caller lifts it out (MCP's `inputSchema`).
fn inlined(schema: fn(&mut schemars::SchemaGenerator) -> schemars::Schema) -> Value {
    let settings = SchemaSettings::draft2020_12().with(|s| {
        s.inline_subschemas = true;
    });
    let mut g = settings.into_generator();
    let mut value = schema(&mut g).to_value();
    let defs = g.take_definitions(true);
    if let (false, Some(object)) = (defs.is_empty(), value.as_object_mut()) {
        object.insert("$defs".into(), Value::Object(defs.into_iter().collect()));
    }
    value
}
