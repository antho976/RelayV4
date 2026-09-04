//! `ui.*` / `os.*` — BUS.md §10.17. Executor is the main window (§6.5); agents need `allow_ui`.
use crate::registry::{Audit, OpMeta, Scope, Undo};
use crate::types::{Id, Page, PaneInfo, PaneKind, PaneRef, PaneTarget, WindowInfo};
use crate::{op, Empty};
use serde_json::Value;

result!(#[schemars(rename = "UiStateOut")] StateOut { pub project_id: Option<Id>, pub page: Page, pub panes: Vec<PaneInfo>, pub focused: Option<PaneRef>, pub windows: Vec<WindowInfo> });
op!(State, "ui.state", Empty => StateOut, OpMeta::query(Scope::Global, 9, "What the UI has open").ui());
payload!(#[schemars(rename = "UiPageSwitchIn")] PageSwitchIn { pub page: Page, pub project_id: Option<Id> });
op!(PageSwitch, "ui.page.switch", PageSwitchIn => Empty,
    OpMeta::mutation(Scope::Global, 9, "Switch page").audit(Audit::AgentOnly).undo(Undo::Inverse).ui());
payload!(#[schemars(rename = "PaneOpenIn")] PaneOpenIn { pub kind: PaneKind, pub target: Option<PaneTarget>, pub at: Option<String> });
result!(#[schemars(rename = "PaneOpenOut")] PaneOpenOut { pub pane: PaneRef });
op!(PaneOpen, "ui.pane.open", PaneOpenIn => PaneOpenOut,
    OpMeta::mutation(Scope::Global, 9, "Open a pane (e.g. a diff for a sha)").audit(Audit::AgentOnly).ui());
payload!(#[schemars(rename = "PaneIn")] PaneIn { pub pane: PaneRef });
op!(PaneClose, "ui.pane.close", PaneIn => Empty, OpMeta::mutation(Scope::Global, 9, "Close a pane").audit(Audit::AgentOnly).ui());
op!(PaneFocus, "ui.pane.focus", PaneIn => Empty, OpMeta::mutation(Scope::Global, 9, "Focus a pane").audit(Audit::AgentOnly).ui());
payload!(#[schemars(rename = "PaneMoveIn")] PaneMoveIn { pub pane: PaneRef, pub to: PaneRef, pub edge: String });
op!(PaneMove, "ui.pane.move", PaneMoveIn => Empty, OpMeta::mutation(Scope::Global, 9, "Re-dock a pane").audit(Audit::AgentOnly).ui());
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
op!(WindowPopout, "ui.window.popout", PaneIn => PopoutOut, OpMeta::mutation(Scope::Global, 9, "Pop a pane out to its own OS window").audit(Audit::AgentOnly).ui());
payload!(#[schemars(rename = "WindowIn")] WindowIn { pub window_id: String });
op!(WindowClose, "ui.window.close", WindowIn => Empty, OpMeta::mutation(Scope::Global, 9, "Close a satellite window").audit(Audit::AgentOnly).ui());
result!(#[schemars(rename = "WindowListOut")] WindowListOut { pub windows: Vec<WindowInfo> });
op!(WindowList, "ui.window.list", Empty => WindowListOut, OpMeta::query(Scope::Global, 9, "Windows").ui());
payload!(#[schemars(rename = "UiToastIn")] ToastIn { pub text: String, pub level: Option<String>, pub ttl_ms: Option<u32> });
op!(Toast, "ui.toast", ToastIn => Empty, OpMeta::mutation(Scope::Global, 9, "Show a toast").audit(Audit::AgentOnly).ui());
payload!(#[schemars(rename = "UiRevealIn")] RevealIn { pub path: String });
op!(OsReveal, "os.reveal", RevealIn => Empty, OpMeta::mutation(Scope::Global, 9, "Reveal in the file manager").audit(Audit::Never).ui());
payload!(#[schemars(rename = "UiOpenUrlIn")] OpenUrlIn { pub url: String });
op!(OsOpenUrl, "os.open_url", OpenUrlIn => Empty, OpMeta::mutation(Scope::Global, 9, "Open a URL in the system browser").audit(Audit::Never).ui());

entries!(State, PageSwitch, PaneOpen, PaneClose, PaneFocus, PaneMove, LayoutList, LayoutSave, LayoutApply, LayoutDelete, WindowPopout, WindowClose, WindowList, Toast, OsReveal, OsOpenUrl);
