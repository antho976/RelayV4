//! `file.*` — BUS.md §10.13. Paths are worktree-relative; `..` and absolute are `file.path`.
use crate::op;
use crate::registry::{Actors, OpMeta, Scope, Undo};
use crate::types::{Entry, Id};

payload!(#[schemars(rename = "FileTreeIn")] TreeIn { pub project_id: Id, pub worktree: Option<String>, pub path: Option<String>, pub depth: Option<u32>, pub git_badges: Option<bool> });
result!(#[schemars(rename = "FileTreeOut")] TreeOut { pub entries: Vec<Entry> });
op!(Tree, "file.tree", TreeIn => TreeOut, OpMeta::query(Scope::Project, 8, "Directory listing with git badges"));
payload!(#[schemars(rename = "FileReadIn")] ReadIn { pub project_id: Id, pub worktree: Option<String>, pub path: String, pub max_bytes: Option<u64> });
result!(#[schemars(rename = "FileReadOut")] ReadOut { pub text: Option<String>, pub bytes_b64: Option<String>, pub mime: String, pub size: u64, pub truncated: bool });
op!(Read, "file.read", ReadIn => ReadOut, OpMeta::query(Scope::Project, 8, "Read a file (text or base64)"));
payload!(#[schemars(rename = "FileWriteIn")] WriteIn { pub project_id: Id, pub worktree: Option<String>, pub path: String, pub text: String, pub expected_sha256: Option<String> });
result!(#[schemars(rename = "FileWriteOut")] WriteOut { pub bytes: u64, pub removed_lines: i64, pub added_lines: i64 });
op!(Write, "file.write", WriteIn => WriteOut,
    OpMeta::mutation(Scope::Project, 8, "Write a file through the guardrails").emits(&["file.changed"]));
payload!(#[schemars(rename = "FileCreateIn")] CreateIn { pub project_id: Id, pub worktree: Option<String>, pub path: String, pub kind: String, pub text: Option<String> });
op!(Create, "file.create", CreateIn => Entry,
    OpMeta::mutation(Scope::Project, 8, "Create a file or directory").emits(&["file.changed"]));
payload!(#[schemars(rename = "FileRenameIn")] RenameIn { pub project_id: Id, pub worktree: Option<String>, pub path: String, pub new_name: String });
op!(Rename, "file.rename", RenameIn => Entry,
    OpMeta::mutation(Scope::Project, 8, "Rename in place").emits(&["file.changed"]));
payload!(#[schemars(rename = "FileMoveIn")] MoveIn { pub project_id: Id, pub worktree: Option<String>, pub path: String, pub into: String });
op!(Move, "file.move", MoveIn => Entry,
    OpMeta::mutation(Scope::Project, 8, "Move into a directory").emits(&["file.changed"]));
payload!(#[schemars(rename = "FilePathIn")] PathIn { pub project_id: Id, pub worktree: Option<String>, pub path: String });
result!(#[schemars(rename = "FileDeleteOut")] DeleteOut { pub trash_id: Id });
op!(Delete, "file.delete", PathIn => DeleteOut,
    OpMeta::mutation(Scope::Project, 8, "Soft-delete to .relay/trash").undo(Undo::Inverse).emits(&["file.changed"]));
payload!(#[schemars(rename = "FileRestoreIn")] RestoreIn { pub project_id: Id, pub trash_id: Id });
op!(Restore, "file.restore", RestoreIn => Entry,
    OpMeta::mutation(Scope::Project, 8, "Restore from trash").emits(&["file.changed"]));
op!(RestoreHead, "file.restore_head", PathIn => Entry,
    OpMeta::mutation(Scope::Project, 8, "git checkout -- <path>: the one-click answer to a post-hoc violation").actors(Actors::UserOnly).emits(&["file.changed"]));
payload!(#[schemars(rename = "ImportIn")] ImportIn { pub project_id: Id, pub worktree: Option<String>, pub into: String, pub sources: Vec<String> });
result!(#[schemars(rename = "ImportOut")] ImportOut { pub entries: Vec<Entry> });
op!(Import, "file.import", ImportIn => ImportOut,
    OpMeta::mutation(Scope::Project, 8, "Copy files in from the OS (drag-in)").actors(Actors::UserOnly).emits(&["file.changed"]));
payload!(#[schemars(rename = "FileSearchIn")] SearchIn { pub project_id: Id, pub worktree: Option<String>, pub query: String, pub glob: Option<String>, pub regex: Option<bool>, pub limit: Option<u32> });
result!(#[schemars(rename = "Hit")] Hit { pub path: String, pub line: u32, pub col: u32, pub text: String });
result!(#[schemars(rename = "FileSearchOut")] SearchOut { pub hits: Vec<Hit> });
op!(Search, "file.search", SearchIn => SearchOut, OpMeta::query(Scope::Project, 8, "Content search"));

entries!(
    Tree,
    Read,
    Write,
    Create,
    Rename,
    Move,
    Delete,
    Restore,
    RestoreHead,
    Import,
    Search
);
