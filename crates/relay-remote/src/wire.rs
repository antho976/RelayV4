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

/// What a paired phone may call: the ops the app sends (apps/relay-mobile), and nothing else.
/// The socket door gives `user` the whole bus, which is right for a person at this machine's
/// keyboard; a credential that lives on a phone, and can be lost with it, should not be able to
/// reconfigure what the engine executes (`settings.set`, which holds `providers.*.path` and
/// `device.adb_path`), quit the engine, or call ops no screen of the app uses. An op the app
/// starts sending is added here. Sorted: `gate` binary-searches it.
pub const PHONE_OPS: &[&str] = &[
    "app.backup.list", "app.backup.now", "app.resources.watch",
    "audit.list", "audit.undo",
    "avd.boot", "avd.list",
    "bus.ping", "bus.subscribe", "bus.unsubscribe",
    "dashboard.get",
    "device.build", "device.list", "device.run", "device.run.list", "device.run.stop",
    "file.create", "file.delete", "file.read", "file.rename", "file.restore", "file.restore_head",
    "file.search", "file.tree", "file.write",
    "git.branch.clean_merged", "git.branch.create", "git.branch.delete", "git.branch.switch",
    "git.branches", "git.commit", "git.diff", "git.diff.file", "git.fetch", "git.log", "git.pr.list",
    "git.pr.open", "git.push", "git.show", "git.stage", "git.status", "git.suggest_message",
    "git.unstage",
    "github.connect", "github.repo.list", "github.status",
    "guardrail.config.get", "guardrail.config.set", "guardrail.confirm", "guardrail.hold.get",
    "guardrail.holds.list", "guardrail.reject",
    "integration.discard", "integration.get", "integration.list", "integration.request",
    "mailbox.list", "mailbox.outbox", "mailbox.send",
    "module.changelog.draft", "module.complete", "module.create", "module.delete", "module.get",
    "module.list", "module.reopen", "module.restore", "module.update",
    // Tally's sync (apps/tally): the one money op a phone calls.
    "money.sync",
    "notes.append", "notes.create", "notes.delete", "notes.get", "notes.list", "notes.pin",
    "notes.restore", "notes.standing", "notes.update",
    "notify.ack", "notify.ack_all", "notify.list", "notify.settings.get", "notify.settings.set",
    "overlap.list",
    "plugin.enable", "plugin.get", "plugin.list",
    "project.add", "project.clone", "project.get", "project.list", "project.remove", "project.update",
    "provider.list", "provider.refresh", "provider.update",
    "session.attach", "session.brief", "session.clear_restorable", "session.close", "session.create",
    "session.detach", "session.discard_restorable", "session.get", "session.input", "session.list",
    "session.park", "session.peers", "session.resize", "session.restorable", "session.resume",
    "session.scrollback", "session.spawn", "session.update", "session.wake",
    "settings.get", "settings.reset",
    "skill.create", "skill.delete", "skill.enable", "skill.install", "skill.list", "skill.update",
    "task.activity", "task.approve", "task.attach", "task.changelog.write", "task.children",
    "task.create", "task.delete", "task.detach", "task.dispatch", "task.get", "task.label.add",
    "task.label.list", "task.label.remove", "task.link_commit", "task.list", "task.move",
    "task.parent.set", "task.relate", "task.restore", "task.unrelate", "task.update",
    "ui.page.switch",
    "usage.get",
    "workspace.create", "workspace.discover", "workspace.list", "workspace.remove", "workspace.update",
    "worktree.list",
];

/// A phone is the user at the keyboard, and only that: `agent:*` and `system` are refused here
/// before they can reach the socket door with a guessed token, `test` is accepted only on an
/// instance that accepts it (BUS.md §4.1), and only `PHONE_OPS` pass. `line` is one line: the
/// engine reads its door line by line, so a frame holding several is gated piece by piece
/// (`bridge.rs`). A line that is not JSON is forwarded untouched so the engine produces its own
/// typed `bus.parse` refusal; it cannot act on what it cannot parse.
pub fn gate(line: &str, instance: Instance) -> Gate {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Gate::Forward;
    };
    let actor = value.get("actor").and_then(|a| a.as_str()).unwrap_or("");
    let op = value.get("op").and_then(|o| o.as_str()).unwrap_or("");
    let error = if !(actor == "user" || (actor == "test" && instance.accepts_test_actor())) {
        BusError::invalid(
            "bus.actor",
            format!("a remote device acts as `user`; actor {actor:?} is not accepted on this door"),
        )
    } else if PHONE_OPS.binary_search(&op).is_err() {
        BusError::refused("remote.op", format!("{op} is not open to a paired phone; run it on the PC"))
    } else {
        return Gate::Forward;
    };
    let id = value
        .get("id")
        .and_then(|i| i.as_str())
        .and_then(|i| i.parse().ok());
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
    fn a_phone_calls_only_the_ops_the_app_uses() {
        let line = |op: &str| format!(r#"{{"v":1,"id":"7f2b5d2e-2f1a-4b7c-9a6c-1c9d1e6a0004","actor":"user","op":"{op}","payload":{{}}}}"#);
        assert!(matches!(gate(&line("session.input"), Instance::Stable), Gate::Forward));
        for op in ["settings.set", "app.quit", "guardrail.grant.revoke", "worktree.remove", "os.open_url", "no.such"] {
            match gate(&line(op), Instance::Stable) {
                Gate::Reject(resp) => assert_eq!(resp.error.unwrap().code, "remote.op"),
                Gate::Forward => panic!("{op} forwarded"),
            }
        }
        // `binary_search` needs the order, and a renamed op must not linger here unnoticed.
        assert!(PHONE_OPS.windows(2).all(|w| w[0] < w[1]), "PHONE_OPS is not sorted");
        let registry = relay_bus::registry::Registry::global();
        for op in PHONE_OPS {
            assert!(registry.get(op).is_some(), "{op} is not a bus op");
        }
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
