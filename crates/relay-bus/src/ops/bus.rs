//! `bus.*` — BUS.md §10.1.
use crate::registry::{Actors, Doors, OpInfo, OpMeta, Scope};
use crate::{op, Empty};
use crate::envelope::Actor;
use serde_json::Value;

result!(#[schemars(rename = "BusPong")] Pong { pub pong: bool, pub instance: String, pub version: String, pub uptime_s: u64 });
op!(Ping, "bus.ping", Empty => Pong, OpMeta::query(Scope::Global, 1, "Liveness: instance, version, uptime"));

payload!(#[schemars(rename = "BusSchemaIn")] SchemaIn { pub op: Option<String> });
result!(#[schemars(rename = "BusSchemaOut")] SchemaOut { pub schema: Value });
op!(Schema, "bus.schema", SchemaIn => SchemaOut, OpMeta::query(Scope::Global, 1, "The whole bus.v1.json, or one op's {payload, result} schema"));

payload!(#[schemars(rename = "BusOpsIn")] OpsIn { pub actor: Option<Actor> });
result!(#[schemars(rename = "BusOpsOut")] OpsOut { pub ops: Vec<OpInfo> });
op!(Ops, "bus.ops", OpsIn => OpsOut, OpMeta::query(Scope::Global, 1, "List ops with registry attributes, filtered to what the actor may call"));

payload!(#[schemars(rename = "BusWaitIn")] WaitIn {
    /// Exact names or `prefix.*`. Empty waits for anything.
    pub events: Option<Vec<String>>,
    /// How long to block before giving up. Default 60s, clamped to an hour.
    pub timeout_ms: Option<u64>,
    /// Only an event whose payload has every one of these top-level keys equal to the value
    /// given, e.g. `{"request_id": 7}`. Omit to take the first event that matches by name.
    pub matching: Option<Value>,
});
result!(#[schemars(rename = "BusWaitOut")] WaitOut {
    pub event: Option<crate::envelope::Event>,
    pub timed_out: bool,
});
op!(Wait, "bus.wait", WaitIn => WaitOut,
    OpMeta::query(Scope::Global, 5, "Block until a matching event arrives, or the timeout: the wake-up an agent has instead of a poll loop").doors(Doors::SocketOnly));

result!(#[schemars(rename = "BusWhoamiOut")] WhoamiOut {
    pub actor: String,
    pub is_agent: bool,
    pub session: Option<String>,
    pub role: Option<crate::types::Role>,
    pub project_id: Option<crate::types::Id>,
    pub project: Option<String>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    /// Every op this actor may really call, all three gating layers applied.
    pub can_call: Vec<String>,
    /// Absolute roots this actor may write to, worktree first.
    pub write_roots: Vec<String>,
});
op!(Whoami, "bus.whoami", Empty => WhoamiOut,
    OpMeta::query(Scope::Global, 1, "Who am I and what may I call — for any actor, agent or not"));

payload!(#[schemars(rename = "BusSubscribeIn")] SubscribeIn { pub events: Option<Vec<String>> });
result!(#[schemars(rename = "BusSubscribeOut")] SubscribeOut { pub subscribed: Vec<String> });
op!(Subscribe, "bus.subscribe", SubscribeIn => SubscribeOut,
    OpMeta::query(Scope::Global, 1, "Turn this socket connection into an event subscriber (socket door only)").doors(Doors::SocketOnly));
op!(Unsubscribe, "bus.unsubscribe", Empty => Empty,
    OpMeta::query(Scope::Global, 1, "Stop receiving events on this connection").doors(Doors::SocketOnly));

#[allow(dead_code)]
const _ACTORS_USED: Actors = Actors::All;

entries!(Ping, Schema, Ops, Whoami, Wait, Subscribe, Unsubscribe);
