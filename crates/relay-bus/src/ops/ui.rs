//! `ui.*` / `os.*` — BUS.md §10.17; agents need `allow_ui`.
//!
//! Core answers every `ui.*` op itself, from an in-memory shell model (page, panes, windows)
//! that lives as long as the engine and is not persisted (BUS.md §6.5). No client executes
//! them and none is required: a headless engine answers them the same way. Each change
//! emits `ui.changed` with the whole model, and the native client applies only part of it:
//! the page (with `project_id`) and a focused pane's `target.session`, which focuses that
//! terminal. It shows `ui.toast` text in its notice bar, and `ui.layout.apply` through
//! `layout.changed`. Panes without a `target.session`, `ui.pane.move` and `ui.window.*` change
//! the model only: nothing on screen opens, moves or pops out, and `ui.state` describes the
//! model, not the window. The summaries below say which is which, because an MCP tool description is
//! all an agent reads.
use crate::registry::{Audit, OpMeta, Scope, Undo};
use crate::types::{Id, Page, PaneInfo, PaneKind, PaneRef, PaneTarget, WindowInfo};
use crate::{op, Empty};
use serde_json::Value;

result!(#[schemars(rename = "UiStateOut")] StateOut { pub project_id: Option<Id>, pub page: Page, pub panes: Vec<PaneInfo>, pub focused: Option<PaneRef>, pub windows: Vec<WindowInfo> });
op!(State, "ui.state", Empty => StateOut, OpMeta::query(Scope::Global, 9, "The engine's shell model: the last page switched to, plus panes and windows recorded by ui.* ops. Not the native window's own panes"));
payload!(#[schemars(rename = "UiPageSwitchIn")] PageSwitchIn { pub page: Page, pub project_id: Option<Id> });
op!(PageSwitch, "ui.page.switch", PageSwitchIn => Empty,
    OpMeta::mutation(Scope::Global, 9, "Switch the page (and project); the native client follows").audit(Audit::AgentOnly).undo(Undo::Inverse));
payload!(#[schemars(rename = "PaneOpenIn")] PaneOpenIn { pub kind: PaneKind, pub target: Option<PaneTarget>, pub at: Option<String> });
result!(#[schemars(rename = "PaneOpenOut")] PaneOpenOut { pub pane: PaneRef });
op!(PaneOpen, "ui.pane.open", PaneOpenIn => PaneOpenOut,
    OpMeta::mutation(Scope::Global, 9, "Record a pane in the shell model. The native client acts only on target.session (it focuses that terminal); other targets open nothing").audit(Audit::AgentOnly));
payload!(#[schemars(rename = "PaneIn")] PaneIn { pub pane: PaneRef });
op!(PaneClose, "ui.pane.close", PaneIn => Empty, OpMeta::mutation(Scope::Global, 9, "Remove a pane from the shell model; the native client closes nothing").audit(Audit::AgentOnly));
op!(PaneFocus, "ui.pane.focus", PaneIn => Empty, OpMeta::mutation(Scope::Global, 9, "Focus a pane in the shell model; the native client follows only when the pane has target.session").audit(Audit::AgentOnly));
payload!(#[schemars(rename = "PaneMoveIn")] PaneMoveIn { pub pane: PaneRef, pub to: PaneRef, pub edge: String });
op!(PaneMove, "ui.pane.move", PaneMoveIn => Empty, OpMeta::mutation(Scope::Global, 9, "Reorder panes in the shell model; the native client does not move anything").audit(Audit::AgentOnly));
payload!(#[schemars(rename = "LayoutListIn")] LayoutListIn { pub project_id: Id });
result!(#[schemars(rename = "LayoutListOut")] LayoutListOut { pub layouts: Vec<String> });
op!(LayoutList, "ui.layout.list", LayoutListIn => LayoutListOut, OpMeta::query(Scope::Project, 9, "Saved layout presets"));
payload!(#[schemars(rename = "UiLayoutSaveIn")] LayoutSaveIn { pub project_id: Id, pub name: String, pub state: Option<Value> });
payload!(#[schemars(rename = "LayoutNameIn")] LayoutNameIn { pub project_id: Id, pub name: String });
op!(LayoutSave, "ui.layout.save", LayoutSaveIn => Empty,
    OpMeta::mutation(Scope::Project, 9, "Save the current arrangement under a name (overwrites)").undo(Undo::Inverse).emits(&["layout.changed"]));
op!(LayoutApply, "ui.layout.apply", LayoutNameIn => Empty, OpMeta::mutation(Scope::Project, 9, "Apply a preset").audit(Audit::AgentOnly).emits(&["layout.changed"]));
op!(LayoutDelete, "ui.layout.delete", LayoutNameIn => Empty,
    OpMeta::mutation(Scope::Project, 9, "Delete a preset").undo(Undo::Inverse).emits(&["layout.changed"]));
result!(#[schemars(rename = "UiPopoutOut")] PopoutOut { pub window_id: String });
op!(WindowPopout, "ui.window.popout", PaneIn => PopoutOut, OpMeta::mutation(Scope::Global, 9, "Record a pop-out window in the shell model; no OS window opens").audit(Audit::AgentOnly));
payload!(#[schemars(rename = "WindowIn")] WindowIn { pub window_id: String });
op!(WindowClose, "ui.window.close", WindowIn => Empty, OpMeta::mutation(Scope::Global, 9, "Remove a pop-out window from the shell model").audit(Audit::AgentOnly));
result!(#[schemars(rename = "WindowListOut")] WindowListOut { pub windows: Vec<WindowInfo> });
op!(WindowList, "ui.window.list", Empty => WindowListOut, OpMeta::query(Scope::Global, 9, "Windows in the shell model (main plus recorded pop-outs), not real OS windows"));
payload!(#[schemars(rename = "UiToastIn")] ToastIn { pub text: String, pub level: Option<String>, pub ttl_ms: Option<u32> });
op!(Toast, "ui.toast", ToastIn => Empty, OpMeta::mutation(Scope::Global, 9, "Show text in the native client's notice bar (emits ui.toast; level and ttl_ms are carried but ignored)").audit(Audit::AgentOnly));
payload!(#[schemars(rename = "UiRevealIn")] RevealIn { pub path: String });
op!(OsReveal, "os.reveal", RevealIn => Empty, OpMeta::mutation(Scope::Global, 9, "Reveal in the file manager").audit(Audit::Never));
payload!(#[schemars(rename = "UiOpenUrlIn")] OpenUrlIn { pub url: String });
op!(OsOpenUrl, "os.open_url", OpenUrlIn => Empty, OpMeta::mutation(Scope::Global, 9, "Open a URL in the system browser").audit(Audit::Never));

entries!(State, PageSwitch, PaneOpen, PaneClose, PaneFocus, PaneMove, LayoutList, LayoutSave, LayoutApply, LayoutDelete, WindowPopout, WindowClose, WindowList, Toast, OsReveal, OsOpenUrl);
