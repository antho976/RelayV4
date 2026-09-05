//! `session.*` — BUS.md §10.8. Lifecycle ops are user-only; agents get done/report/attach/brief/peers/get/list.
use crate::registry::{Actors, Audit, OpMeta, Scope, Undo};
use crate::types::{GuardrailCaps, Id, Peer, Provider, Role, Session, SessionState, Ts};
use crate::{op, Empty};
use serde_json::Value;

payload!(#[schemars(rename = "SessionCreateIn")] CreateIn {
    pub project_id: Id, pub provider: Provider, pub role: Option<Role>, pub model: Option<String>, pub effort: Option<String>,
    pub branch: Option<String>,
    /// `"new"` (default: pooled worktree), `"primary"`, or an absolute path of an existing worktree.
    pub worktree: Option<String>,
    pub task_id: Option<Id>, pub module_id: Option<Id>, pub pair_with: Option<String>,
    pub bus_writes: Option<bool>, pub allow_ui: Option<bool>,
    /// Persist the initial assignment before spawning, so a partial launch can recover it.
    pub prompt: Option<String>,
});
op!(Create, "session.create", CreateIn => Session,
    OpMeta::mutation(Scope::Project, 3, "Allocate a session (name, worktree, token); spawns nothing").actors(Actors::UserOnly).emits(&["session.changed"]));

payload!(#[schemars(rename = "SessionSpawnIn")] SpawnIn {
    pub session: String,
    /// Omitted preserves the allocated assignment; an empty string explicitly clears it.
    pub prompt: Option<String>,
});
op!(Spawn, "session.spawn", SpawnIn => Session,
    OpMeta::mutation(Scope::Session, 3, "Start the provider CLI in the worktree with the brief").actors(Actors::UserOnly).emits(&["session.changed"]));
payload!(#[schemars(rename = "SessionNameIn")] NameIn { pub session: String });
op!(Resume, "session.resume", NameIn => Session,
    OpMeta::mutation(Scope::Session, 6, "Provider-resume a restorable session").actors(Actors::UserOnly).emits(&["session.changed"]));
op!(ClearRestorable, "session.clear_restorable", NameIn => Session,
    OpMeta::mutation(Scope::Session, 6, "Fresh-spawn a restorable session without its saved provider context").actors(Actors::UserOnly).emits(&["session.changed"]));
op!(Park, "session.park", NameIn => Session,
    OpMeta::mutation(Scope::Session, 6, "Kill the CLI, keep pane/scrollback/worktree/token").actors(Actors::UserOnly).emits(&["session.changed"]));
op!(Wake, "session.wake", NameIn => Session,
    OpMeta::mutation(Scope::Session, 6, "Respawn a parked session with provider resume").actors(Actors::UserOnly).emits(&["session.changed"]));
payload!(#[schemars(rename = "SessionCloseIn")] CloseIn { pub session: String, pub remove_worktree: Option<bool>, pub purge_build: Option<bool> });
result!(#[schemars(rename = "SessionCloseOut")] CloseOut { pub freed_mb: f64 });
op!(Close, "session.close", CloseIn => CloseOut,
    OpMeta::mutation(Scope::Session, 3, "Kill, purge build output, remove worktree; branch kept").actors(Actors::UserOnly).emits(&["session.changed", "worktree.changed"]));
payload!(#[schemars(rename = "SessionDoneIn")] DoneIn {
    pub session: String, pub summary: Option<String>, pub sha: Option<String>,
    /// `completed` (default), `blocked`, or `partial`. An op that can only report success
    /// gets reported success (D118).
    pub status: Option<String>,
    /// What stopped it, for `blocked` and `partial`.
    pub blockers: Option<Vec<String>>,
});
op!(Done, "session.done", DoneIn => Session,
    OpMeta::mutation(Scope::Session, 5, "Record completion; all builders finish before review, and all reviewers finish before the group advances. Blocked/partial retain the assignment.").actors(Actors::AgentOnly).emits(&["session.changed", "task.changed", "notify.new", "notify.changed", "mailbox.new"]));
payload!(#[schemars(rename = "SessionIntentIn")] IntentIn {
    pub session: String,
    /// One line, present tense. Empty clears it.
    pub text: String,
});
op!(Intent, "session.intent", IntentIn => Session,
    OpMeta::mutation(Scope::Session, 5, "Declare in one line what this session is doing; peers read it in session.peers").actors(Actors::AgentOnly).emits(&["session.changed"]));
payload!(#[schemars(rename = "SessionClaimIn")] ClaimIn {
    /// Worktree-relative paths this session is taking on.
    pub paths: Vec<String>,
    /// Narrow the claim to one symbol within each path.
    pub symbol: Option<String>,
    pub note: Option<String>,
    /// Refuse the whole claim if any path is already claimed by a peer, instead of recording
    /// it alongside theirs and reporting the collision.
    pub exclusive: Option<bool>,
});
result!(#[schemars(rename = "SessionClaimCollision")] Collision {
    pub path: String, pub symbol: String, pub session: String, pub since: Ts,
});
result!(#[schemars(rename = "SessionClaimOut")] ClaimOut {
    pub claimed: Vec<String>,
    /// Peers already holding one of these paths. Empty is the quiet, expected case.
    pub collisions: Vec<Collision>,
});
op!(Claim, "session.claim", ClaimIn => ClaimOut,
    OpMeta::mutation(Scope::Session, 5, "Declare the files this session is working on and learn who else holds them").actors(Actors::AgentOnly).emits(&["overlap.changed"]));
payload!(#[schemars(rename = "SessionReleaseIn")] ReleaseIn {
    /// Omit to release every claim this session holds.
    pub paths: Option<Vec<String>>,
});
result!(#[schemars(rename = "SessionReleaseOut")] ReleaseOut { pub released: u32 });
op!(Release, "session.release", ReleaseIn => ReleaseOut,
    OpMeta::mutation(Scope::Session, 5, "Drop claims this session no longer holds").actors(Actors::AgentOnly).emits(&["overlap.changed"]));
op!(Get, "session.get", NameIn => Session, OpMeta::query(Scope::Session, 3, "One session"));
payload!(#[schemars(rename = "SessionListIn")] ListIn { pub project_id: Option<Id>, pub state: Option<Vec<SessionState>>, pub include_closed: Option<bool> });
result!(#[schemars(rename = "SessionListOut")] ListOut { pub sessions: Vec<Session> });
op!(List, "session.list", ListIn => ListOut, OpMeta::query(Scope::Global, 3, "Sessions, filtered; agents see their own project only"));
payload!(#[schemars(rename = "SessionPeersIn")] PeersIn { pub session: Option<String>, pub project_id: Option<Id> });
result!(#[schemars(rename = "SessionPeersOut")] PeersOut { pub peers: Vec<Peer> });
op!(Peers, "session.peers", PeersIn => PeersOut, OpMeta::query(Scope::Project, 5, "The live peer table"));
result!(#[schemars(rename = "SessionBriefParts")] BriefParts { pub state: String, pub peers: String, pub notes: String, pub adjacent: String, pub skills: String });
result!(#[schemars(rename = "SessionBriefOut")] BriefOut {
    /// Everything, skills included. What the UI shows.
    pub text: String,
    /// State, peers, notes and adjacent work only — the part small enough to inject on every
    /// spawn. Skill bodies are referenced by path instead of inlined (D101).
    pub compact: String,
    pub parts: BriefParts,
});
op!(Brief, "session.brief", NameIn => BriefOut, OpMeta::query(Scope::Session, 5, "The knowledge injection, inspectable"));
result!(#[schemars(rename = "SessionBootstrapTask")] BootstrapTask {
    pub id: Id, pub title: String, pub body: String, pub changelog: String,
    pub column: String, pub state: String,
});
result!(#[schemars(rename = "SessionBootstrapPair")] BootstrapPair {
    pub session: String, pub provider: Provider, pub role: Role,
    pub branch: String, pub state: SessionState,
});
result!(#[schemars(rename = "SessionBootstrapGuardrails")] BootstrapGuardrails {
    pub caps: GuardrailCaps,
    pub denied_commands: Vec<String>,
    /// Absolute roots this session may write to, worktree first.
    pub write_roots: Vec<String>,
    /// The op that answers "would this be allowed?" without doing it.
    pub dry_run: String,
});
result!(#[schemars(rename = "SessionBootstrapOut")] BootstrapOut {
    pub session: String, pub role: Role, pub project_id: Id, pub project: String, pub base_branch: String,
    pub worktree: String, pub branch: String, pub module: Option<String>,
    /// Current task retained for older clients and one-task agents.
    pub task: Option<BootstrapTask>,
    /// Current task followed by every queued or in-review task attached to this session.
    pub tasks: Vec<BootstrapTask>,
    pub pair: Option<BootstrapPair>,
    pub assignment: Option<String>,
    /// Worktree-relative path of the full brief.
    pub brief_path: String,
    /// Everyone else live in this project, now.
    pub peers: Vec<Peer>,
    /// Every op this session may really call, all three gating layers applied.
    pub can_call: Vec<String>,
    /// How to inspect operation summaries and exact payload/result schemas without guessing.
    pub discovery: String,
    /// How to reach the peers above, and which channel actually reaches all of them.
    pub comms: String,
    pub guardrails: BootstrapGuardrails,
});
op!(Bootstrap, "session.bootstrap", Empty => BootstrapOut,
    OpMeta::query(Scope::Session, 12, "Who am I, who else is here, and what may I call: the one call every agent makes first").actors(Actors::AgentOnly));
payload!(#[schemars(rename = "SessionUpdateIn")] UpdateIn {
    pub session: String, pub branch: Option<String>, pub model: Option<String>, pub effort: Option<String>,
    #[serde(default, deserialize_with = "crate::nullable")] pub task_id: Option<Option<Id>>, #[serde(default, deserialize_with = "crate::nullable")] pub module_id: Option<Option<Id>>, pub bus_writes: Option<bool>, pub allow_ui: Option<bool>,
});
op!(Update, "session.update", UpdateIn => Session,
    OpMeta::mutation(Scope::Session, 6, "Patch a session; branch/model/effort conflict after first spawn").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["session.changed"]));
payload!(#[schemars(rename = "SessionReportIn")] ReportIn { pub session: String, pub kind: String, pub data: Option<Value> });
op!(Report, "session.report", ReportIn => Empty,
    OpMeta::mutation(Scope::Session, 5, "Provider hooks calling home: session_start | tool_use | stop | notification | idle | blocked").audit(Audit::AgentOnly).actors(Actors::AgentOnly).emits(&["session.changed", "notify.new"]));
payload!(#[schemars(rename = "SessionAttachIn")] AttachIn { pub session: String, pub from_seq: Option<u64>, pub epoch: Option<u64> });
op!(Attach, "session.attach", AttachIn => Empty, OpMeta::query(Scope::Session, 3, "Attach the pty stream, replaying from (epoch, seq)").stream("pty"));
op!(Detach, "session.detach", NameIn => Empty, OpMeta::query(Scope::Session, 3, "Detach the pty stream"));
payload!(#[schemars(rename = "SessionInputIn")] InputIn { pub session: String, pub data: String });
op!(Input, "session.input", InputIn => Empty,
    OpMeta::mutation(Scope::Session, 3, "Text to the PTY").audit(Audit::AgentOnly).actors(Actors::UserOnly).emits(&["session.changed"]));
payload!(#[schemars(rename = "SessionResizeIn")] ResizeIn { pub session: String, pub cols: u16, pub rows: u16 });
op!(Resize, "session.resize", ResizeIn => Empty,
    OpMeta::mutation(Scope::Session, 3, "Resize the PTY").audit(Audit::AgentOnly).actors(Actors::UserOnly));
payload!(#[schemars(rename = "SessionScrollbackIn")] ScrollbackIn { pub session: String, pub lines: Option<u32> });
result!(#[schemars(rename = "SessionScrollbackOut")] ScrollbackOut { pub text: String, pub epoch: u64, pub seq: u64 });
op!(Scrollback, "session.scrollback", ScrollbackIn => ScrollbackOut, OpMeta::query(Scope::Session, 3, "Scrollback text and the (epoch, seq) it ends at"));
result!(#[schemars(rename = "SessionRestorable")] Restorable { pub session: Session, pub reason: String, pub worktree_dirty: bool });
result!(#[schemars(rename = "SessionRestorableOut")] RestorableOut { pub sessions: Vec<Restorable> });
op!(RestorableList, "session.restorable", Empty => RestorableOut, OpMeta::query(Scope::Global, 6, "Sessions offering resume at launch"));
op!(DiscardRestorable, "session.discard_restorable", NameIn => Empty,
    OpMeta::mutation(Scope::Session, 6, "Decline resume: clean the session").actors(Actors::UserOnly).emits(&["session.changed"]));

entries!(
    Create,
    Spawn,
    Resume,
    ClearRestorable,
    Park,
    Wake,
    Close,
    Done,
    Intent,
    Claim,
    Release,
    Get,
    List,
    Peers,
    Brief,
    Bootstrap,
    Update,
    Report,
    Attach,
    Detach,
    Input,
    Resize,
    Scrollback,
    RestorableList,
    DiscardRestorable
);
