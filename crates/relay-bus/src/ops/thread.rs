//! `thread.*` — Threads: conversations with an agent that reads and changes the person's own
//! data (docs/THREADS.md).
//!
//! A thread is kept by the engine in its own SQLite file (`threads.db`), so a thread reopens
//! from Relay's copy without asking the agent anything. The agent itself runs in the background
//! while the thread is in use and is resumed by its own conversation id when a cold thread gets
//! a new message. Only the person starts, steers or removes threads (`Actors::UserOnly`); the
//! agent reaches the person's data through the ops its thread is given, not through these.
//!
//! Events: `thread.changed` (a thread was created, renamed, deleted, or started or finished a
//! turn), `thread.message` (a message was stored) and `thread.delta` (a piece of the reply being
//! written, for live display; not stored).
use crate::registry::{Actors, OpMeta, Scope};
use crate::types::Id;
use crate::{op, Empty};
use serde_json::Value;

result!(#[schemars(rename = "ThreadView")] ThreadView {
    pub id: Id,
    pub title: String,
    /// `claude`; the agent that answers in this thread.
    pub provider: String,
    pub model: Option<String>,
    /// `low`, `medium`, `high`, `xhigh` or `max`; the provider's default when absent.
    pub effort: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// The agent is writing a reply.
    pub working: bool,
    /// The agent is running for this thread now, so a new message costs no resume.
    pub live: bool,
    /// The start of the last message, for the thread list.
    pub preview: Option<String>,
});
result!(#[schemars(rename = "ThreadMessage")] MessageView {
    pub id: Id,
    pub thread_id: Id,
    /// `user`, `assistant` (text and tool calls), `tool` (a tool's answer) or `error`.
    pub role: String,
    /// `{"text"}` for `user` and `error`; `{"blocks": [...]}` for `assistant`, each block
    /// `{"type":"text","text"}` or `{"type":"tool_use","id","name","input"}`;
    /// `{"tool_use_id","is_error","text"}` for `tool`.
    pub body: Value,
    pub created_at: String,
});

result!(#[schemars(rename = "ThreadListOut")] ListOut { pub threads: Vec<ThreadView> });
op!(List, "thread.list", Empty => ListOut, OpMeta::query(Scope::Global, 5, "Every thread, most recently used first"));

payload!(#[schemars(rename = "ThreadGetIn")] GetIn {
    pub id: Id,
    /// Only the messages stored after this message id.
    pub after: Option<Id>,
});
result!(#[schemars(rename = "ThreadGetOut")] GetOut { pub thread: ThreadView, pub messages: Vec<MessageView> });
op!(Get, "thread.get", GetIn => GetOut, OpMeta::query(Scope::Global, 5, "One thread and its messages, oldest first"));

payload!(#[schemars(rename = "ThreadCreateIn")] CreateIn {
    /// The first message; the thread is named after it and the agent starts on it.
    pub text: Option<String>,
    /// A model for this thread's agent; the provider's default when absent.
    pub model: Option<String>,
    /// How hard it thinks: `low`, `medium`, `high`, `xhigh` or `max`; the default when absent.
    pub effort: Option<String>,
});
op!(Create, "thread.create", CreateIn => ThreadView,
    OpMeta::mutation(Scope::Global, 5, "Start a thread, optionally with its first message").actors(Actors::UserOnly).emits(&["thread.changed", "thread.message"]));

payload!(#[schemars(rename = "ThreadSendIn")] SendIn { pub id: Id, pub text: String });
op!(Send, "thread.send", SendIn => MessageView,
    OpMeta::mutation(Scope::Global, 5, "Send a message in a thread; the agent answers in thread.delta and thread.message events")
        .actors(Actors::UserOnly).emits(&["thread.changed", "thread.message", "thread.delta"]));

payload!(#[schemars(rename = "ThreadIdIn")] IdIn { pub id: Id });
op!(Stop, "thread.stop", IdIn => Empty,
    OpMeta::mutation(Scope::Global, 5, "Stop the agent's reply in a thread; the next message resumes it").actors(Actors::UserOnly).emits(&["thread.changed"]));

payload!(#[schemars(rename = "ThreadSetIn")] SetIn {
    pub id: Id,
    /// The agent's model; empty or absent for the provider's default.
    pub model: Option<String>,
    /// The agent's effort; empty or absent for the default.
    pub effort: Option<String>,
});
op!(Set, "thread.set", SetIn => ThreadView,
    OpMeta::mutation(Scope::Global, 5, "Choose a thread's model and effort; a running agent restarts with them on the next message")
        .actors(Actors::UserOnly).emits(&["thread.changed"]));

payload!(#[schemars(rename = "ThreadRenameIn")] RenameIn { pub id: Id, pub title: String });
op!(Rename, "thread.rename", RenameIn => ThreadView,
    OpMeta::mutation(Scope::Global, 5, "Rename a thread").actors(Actors::UserOnly).emits(&["thread.changed"]));

op!(Delete, "thread.delete", IdIn => Empty,
    OpMeta::mutation(Scope::Global, 5, "Delete a thread and its messages, stopping its agent").actors(Actors::UserOnly).emits(&["thread.changed"]));

entries!(List, Get, Create, Send, Stop, Set, Rename, Delete);
