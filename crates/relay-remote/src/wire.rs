//! The handshake a phone performs before it may speak bus lines, and the one check every
//! line it sends afterwards goes through.
//!
//! The engine's socket door trusts anything with the same uid (BUS.md §0.6). The network is
//! not that, so this door adds one thing in front of it: a device that paired once, proving on
//! every connection that it still holds its token, without sending the token again.

use relay_bus::envelope::Response;
use relay_bus::error::BusError;
use relay_core::Instance;
use serde::{Deserialize, Serialize};

pub const WIRE_V: u32 = 1;

/// The first line the door sends, before the phone says anything.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Greeting {
    pub v: u32,
    pub relay: String,
    pub host: String,
    pub host_id: String,
    pub instance: String,
    pub version: String,
    /// Random per connection; the phone answers with `proof(challenge, token)`.
    pub challenge: String,
}

/// The phone's answer: either a pairing code (first time) or a proof (every time after).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Hello {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
}

/// The door's verdict. `token` is present exactly once: in the reply to a pairing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Welcome {
    pub v: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Welcome {
    pub fn paired(device: &str, token: &str) -> Welcome {
        Welcome { v: WIRE_V, ok: true, device: Some(device.into()), token: Some(token.into()), error: None }
    }
    pub fn admitted(device: &str) -> Welcome {
        Welcome { v: WIRE_V, ok: true, device: Some(device.into()), token: None, error: None }
    }
    pub fn denied(code: &str) -> Welcome {
        Welcome { v: WIRE_V, ok: false, device: None, token: None, error: Some(code.into()) }
    }
}

/// What a phone sends instead of its token: a digest that is only good for this connection.
/// Passive listening on the LAN learns the proof, not the credential. A pairing reply is the one
/// message that carries the token itself, which is why pairing windows are short and pairing
/// through the rendezvous should go over `wss://`. The proof binds nothing that follows: an
/// on-path attacker can inject into the admitted connection (docs/MOBILE.md §6).
pub fn proof(challenge: &str, token: &str) -> String {
    crate::registry::sha256_hex(&format!("{challenge}:{token}"))
}

/// Constant-time equality for two hex digests of the same length.
pub fn digest_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether a bus line from a paired phone may be forwarded to the engine.
pub enum Gate {
    /// Hand the line to the socket door as-is.
    Forward,
    /// Answer this ourselves; the engine never sees it.
    Reject(Box<Response>),
}

/// A phone is the user at the keyboard, and only that: `agent:*` and `system` are refused here
/// before they can reach the socket door with a guessed token, and `test` only on an instance
/// that accepts it (BUS.md §4.1). Anything that is not a request object is forwarded untouched
/// so the engine produces its own typed `bus.parse` refusal.
pub fn gate(line: &str, instance: Instance) -> Gate {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Gate::Forward;
    };
    let actor = value.get("actor").and_then(|a| a.as_str()).unwrap_or("");
    let allowed = actor == "user" || (actor == "test" && instance.accepts_test_actor());
    if allowed {
        return Gate::Forward;
    }
    let id = value
        .get("id")
        .and_then(|i| i.as_str())
        .and_then(|i| i.parse().ok());
    let error = BusError::invalid(
        "bus.actor",
        format!("a remote device acts as `user`; actor {actor:?} is not accepted on this door"),
    );
    Gate::Reject(Box::new(match id {
        Some(id) => Response::err(id, error),
        None => Response::unparsed(error),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_depends_on_both_halves_and_compares_in_constant_time() {
        let a = proof("c1", "t1");
        assert_eq!(a, proof("c1", "t1"));
        assert_ne!(a, proof("c2", "t1"));
        assert_ne!(a, proof("c1", "t2"));
        assert!(digest_eq(&a, &a));
        assert!(!digest_eq(&a, &proof("c2", "t1")));
        assert!(!digest_eq(&a, &a[..10]));
    }

    #[test]
    fn only_the_user_passes_the_gate() {
        let user = r#"{"v":1,"id":"7f2b5d2e-2f1a-4b7c-9a6c-1c9d1e6a0001","actor":"user","op":"bus.ping","payload":{}}"#;
        assert!(matches!(gate(user, Instance::Stable), Gate::Forward));
        let agent = r#"{"v":1,"id":"7f2b5d2e-2f1a-4b7c-9a6c-1c9d1e6a0002","actor":"agent:brisk-otter","op":"bus.ping","payload":{},"token":"x"}"#;
        match gate(agent, Instance::Stable) {
            Gate::Reject(resp) => {
                assert!(!resp.ok);
                assert_eq!(resp.id.map(|i| i.to_string()).as_deref(), Some("7f2b5d2e-2f1a-4b7c-9a6c-1c9d1e6a0002"));
                assert_eq!(resp.error.unwrap().code, "bus.actor");
            }
            Gate::Forward => panic!("agent forwarded"),
        }
        let test = r#"{"v":1,"id":"7f2b5d2e-2f1a-4b7c-9a6c-1c9d1e6a0003","actor":"test","op":"bus.ping","payload":{}}"#;
        assert!(matches!(gate(test, Instance::Dev), Gate::Forward));
        assert!(matches!(gate(test, Instance::Stable), Gate::Reject(_)));
        // The engine's own parser owns the verdict on garbage.
        assert!(matches!(gate("not json", Instance::Stable), Gate::Forward));
        assert!(matches!(gate(r#"{"v":1}"#, Instance::Stable), Gate::Reject(r) if r.id.is_none()));
    }

    #[test]
    fn hello_accepts_either_shape() {
        let pair: Hello = serde_json::from_str(r#"{"v":1,"pair":"ABCD-EFGH","device_name":"Pixel"}"#).unwrap();
        assert_eq!(pair.pair.as_deref(), Some("ABCD-EFGH"));
        let proof: Hello = serde_json::from_str(r#"{"v":1,"device":"abc","proof":"00"}"#).unwrap();
        assert_eq!(proof.device.as_deref(), Some("abc"));
        assert!(proof.pair.is_none());
    }
}
