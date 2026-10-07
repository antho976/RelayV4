//! `overlap.*` — explicit claims plus on-demand file/symbol collision detection.

use crate::engine::{Ctx, Engine, IntoBus};
use crate::sessions;
use relay_bus::error::BusError;
use relay_bus::ops::overlap::*;
use relay_bus::types::{Id, Overlap, OverlapKind};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use tree_sitter::{Language, Node, Parser};

#[derive(Clone)]
struct Finding {
    sessions: Vec<String>,
    path: String,
    symbol: Option<String>,
    kind: OverlapKind,
}

fn kind_str(kind: OverlapKind) -> &'static str {
    match kind {
        OverlapKind::File => "file",
        OverlapKind::Symbol => "symbol",
        OverlapKind::Claim => "claim",
    }
}

fn overlap_row(row: &Row) -> rusqlite::Result<Overlap> {
    let kind: String = row.get("kind")?;
    Ok(Overlap {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        sessions: serde_json::from_str(&row.get::<_, String>("sessions")?).unwrap_or_default(),
        path: row.get("path")?,
        symbol: row.get("symbol")?,
        kind: match kind.as_str() {
            "symbol" => OverlapKind::Symbol,
            "claim" => OverlapKind::Claim,
            _ => OverlapKind::File,
        },
        acked_by: serde_json::from_str(&row.get::<_, String>("acked_by")?).unwrap_or_default(),
        first_seen: row.get("first_seen")?,
        last_seen: row.get("last_seen")?,
    })
}

fn list(tx: &Transaction, project_id: Id) -> Result<Vec<Overlap>, BusError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT * FROM overlaps WHERE project_id=?1 AND active=1
         ORDER BY path, symbol, kind, id",
        )
        .bus()?;
    let rows = stmt
        .query_map([project_id], overlap_row)
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    Ok(rows)
}

fn actor_session(ctx: &Ctx, project_id: Id) -> Result<crate::sessions::Row_, BusError> {
    actor_session_in(ctx.tx(), ctx.actor_session_id(), project_id)
}

fn actor_session_in(
    conn: &rusqlite::Connection,
    session_id: Option<Id>,
    project_id: Id,
) -> Result<crate::sessions::Row_, BusError> {
    let id = session_id
        .ok_or_else(|| BusError::actor("overlap mutation requires a bound agent session"))?;
    let row = sessions::by_id(conn, id)?.ok_or_else(|| BusError::actor("bound session vanished"))?;
    if row.session.project_id != project_id {
        return Err(BusError::not_own("project"));
    }
    Ok(row)
}

/// A project-relative path in the one spelling claims and changed files share: its normal
/// components joined by `/`, so `./src//a.rs` is `src/a.rs`, as git status reports it. `None`
/// for an empty, absolute or escaping path. Claims are compared as strings, so every way in
/// must store them through this, `session.claim` included, or `./src/a.rs` never meets a
/// peer's `src/a.rs` (RA-387).
pub(crate) fn normalize_relative(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for component in Path::new(raw.trim()).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn relative_path(raw: &str) -> Result<String, BusError> {
    normalize_relative(raw).ok_or_else(|| BusError::invalid(
        "overlap.path",
        "path must be a project-relative path with no ..",
    ))
}

fn changed_files(worktree: &Path) -> Result<BTreeSet<String>, BusError> {
    let repo = gix::open(worktree).map_err(crate::engine::internal)?;
    let platform = repo
        .status(gix::progress::Discard)
        .map_err(crate::engine::internal)?
        .untracked_files(gix::status::UntrackedFiles::Files);
    let iter = platform
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(crate::engine::internal)?;
    let mut files = BTreeSet::new();
    for item in iter {
        let item = item.map_err(crate::engine::internal)?;
        files.insert(item.location().to_string());
    }
    Ok(files)
}

fn language(path: &Path) -> Option<(&'static str, Language)> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
    {
        "rs" => Some(("rs", tree_sitter_rust::LANGUAGE.into())),
        "ts" => Some(("ts", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())),
        "tsx" => Some(("tsx", tree_sitter_typescript::LANGUAGE_TSX.into())),
        "svelte" => Some(("svelte", tree_sitter_svelte::LANGUAGE.into())),
        "kt" | "kts" => Some(("kt", tree_sitter_kotlin::LANGUAGE.into())),
        _ => None,
    }
}

thread_local! {
    /// One parser per language, kept between files.
    ///
    /// `Parser::set_language` builds the grammar's whole lexer state; a scan that touches a
    /// hundred changed files paid for that a hundred times. Parsers are explicitly reusable
    /// across parses, and there are five of them at most.
    static PARSERS: std::cell::RefCell<std::collections::HashMap<&'static str, Parser>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Run `f` with a parser already configured for `path`'s language, or `None` if there is none.
fn with_parser<T>(path: &Path, f: impl FnOnce(&mut Parser) -> T) -> Option<T> {
    let (key, language) = language(path)?;
    PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        let parser = match parsers.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut parser = Parser::new();
                parser.set_language(&language).ok()?;
                entry.insert(parser)
            }
        };
        Some(f(parser))
    })
}

fn is_symbol(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "mod_item"
            | "function_declaration"
            | "generator_function_declaration"
            | "method_definition"
            | "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "abstract_class_declaration"
            | "object_declaration"
            | "companion_object"
            | "snippet_block"
    )
}

fn node_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    // `impl Display for Foo` has no name; its first identifier is the trait, which every other
    // `impl Display` shares.
    if node.kind() == "impl_item" {
        let text = |field| node.child_by_field_name(field).and_then(|n| n.utf8_text(source).ok());
        let target = text("type")?;
        return Some(match text("trait") {
            Some(of) => format!("impl {of} for {target}"),
            None => format!("impl {target}"),
        });
    }
    if let Some(name) = node.child_by_field_name("name") {
        return name.utf8_text(source).ok().map(str::to_string);
    }
    let mut cursor = node.walk();
    let name = node
        .children(&mut cursor)
        .find(|child| {
            matches!(
                child.kind(),
                "identifier" | "type_identifier" | "simple_identifier"
            )
        })
        .and_then(|child| child.utf8_text(source).ok().map(str::to_string));
    name
}

fn hash_hex(bytes: &[u8]) -> String {
    crate::hex(&Sha256::digest(bytes))
}

/// Every symbol in a file by its qualified name (`impl Foo::new`, `mod a::helper`), and the
/// hash of its text. A bare name was not a key: the `new` of one impl overwrote another's, so
/// an edit to the first went unseen and two sessions in different `new`s looked like one
/// (RA-173). A name that still repeats (two `#[cfg]` twins) is numbered in source order.
fn symbols(path: &Path, source: &[u8]) -> BTreeMap<String, String> {
    if source.len() > 2 * 1024 * 1024 {
        return BTreeMap::new();
    }
    let Some(Some(tree)) = with_parser(path, |parser| parser.parse(source, None)) else {
        return BTreeMap::new();
    };
    let mut stack: Vec<(Node, std::rc::Rc<str>)> = vec![(tree.root_node(), "".into())];
    let mut out = BTreeMap::new();
    while let Some((node, scope)) = stack.pop() {
        let mut inner = scope;
        if is_symbol(node.kind()) {
            if let Some(name) = node_name(node, source) {
                let qualified = if inner.is_empty() { name } else { format!("{inner}::{name}") };
                let mut key = qualified.clone();
                let mut nth = 1;
                while out.contains_key(&key) {
                    nth += 1;
                    key = format!("{qualified}#{nth}");
                }
                out.insert(key.clone(), hash_hex(&source[node.byte_range()]));
                inner = key.into();
            }
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        // Reversed onto the stack, so they come off in source order and numbering is stable.
        stack.extend(children.into_iter().rev().map(|child| (child, inner.clone())));
    }
    out
}

/// Whether a changed symbol is the one a claim names. A claim is free text: the qualified name,
/// or just the item's own name as an agent would write it.
fn claimed(symbol: &str, claim: &str) -> bool {
    let leaf = symbol.rsplit("::").next().unwrap_or(symbol);
    symbol == claim || leaf == claim || leaf.split('#').next() == Some(claim)
}

fn baseline(repo: &gix::Repository, path: &Path) -> Vec<u8> {
    repo.head_commit()
        .ok()
        .and_then(|commit| commit.tree().ok())
        .and_then(|tree| tree.lookup_entry_by_path(path).ok().flatten())
        .and_then(|entry| entry.object().ok())
        .map(|object| object.data.clone())
        .unwrap_or_default()
}

fn changed_symbols(
    worktree: &Path,
    paths: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let Ok(repo) = gix::open(worktree) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for relative in paths {
        let path = Path::new(relative);
        if language(path).is_none() {
            continue;
        }
        // Only a regular file small enough to parse is read: a planted symlink to a device or
        // a huge file must not be pulled into memory just to be refused by `symbols`.
        let file = worktree.join(path);
        let current = std::fs::metadata(&file)
            .ok()
            .filter(|md| md.is_file() && md.len() <= 2 * 1024 * 1024)
            .and_then(|_| std::fs::read(&file).ok())
            .unwrap_or_default();
        let before = baseline(&repo, path);
        let current_symbols = symbols(path, &current);
        let before_symbols = symbols(path, &before);
        let names = current_symbols
            .iter()
            .filter(|(name, hash)| before_symbols.get(*name) != Some(*hash))
            .map(|(name, _)| name.clone())
            .chain(
                before_symbols
                    .keys()
                    .filter(|name| !current_symbols.contains_key(*name))
                    .cloned(),
            )
            .collect::<BTreeSet<_>>();
        if !names.is_empty() {
            out.insert(relative.clone(), names);
        }
    }
    out
}

fn fingerprint(project_id: Id, finding: &Finding) -> String {
    let raw = format!(
        "{project_id}\0{}\0{}\0{}\0{}",
        kind_str(finding.kind),
        finding.sessions.join("\0"),
        finding.path,
        finding.symbol.as_deref().unwrap_or("")
    );
    hash_hex(raw.as_bytes())
}

/// One checkout's changes: the files, and per changed file the symbols that differ from HEAD.
type Changed = std::sync::Arc<(BTreeSet<String>, BTreeMap<String, BTreeSet<String>>)>;

/// Per session: its checkout and that checkout's changes. Sessions sharing a checkout (a PAIR
/// partner, a reviewer on the builder's worktree) share one scan of it.
type Changes = BTreeMap<String, (String, Changed)>;

/// The store half of a scan: which live checkouts to read. Cheap.
fn scan_targets(conn: &rusqlite::Connection, project_id: Id) -> Result<Vec<(String, String)>, BusError> {
    crate::handlers::workspace::get_project(conn, project_id)?;
    let mut stmt = conn.prepare_cached(
        "SELECT name, worktree FROM sessions WHERE project_id=?1 AND state!='closed' ORDER BY name, id",
    ).bus()?;
    let rows = stmt
        .query_map([project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    Ok(rows)
}

/// The expensive half, and the reason both callers are staged: a status of every checkout and
/// a parse of every changed file. It ran inside the transaction, so a flag from one agent held
/// the store mutex — and every keystroke — for 30 ms with two sessions and for seconds with a
/// busy wall (PERF §1.1). Now it runs with no lock held (BUS.md §5.1, D149).
fn collect_changes(targets: &[(String, String)]) -> Result<Changes, BusError> {
    let mut changes = Changes::new();
    let mut scanned = BTreeMap::<String, Changed>::new();
    for (name, worktree) in targets {
        let path = Path::new(worktree);
        if !path.is_dir() {
            continue;
        }
        let checkout = std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or_else(|_| worktree.clone());
        let changed = match scanned.get(&checkout) {
            Some(changed) => changed.clone(),
            None => {
                let files = match changed_files(path) {
                    Ok(files) => files,
                    // One broken checkout (half removed, pruned, unreadable) is left out like a
                    // missing one, not allowed to fail every flag in the project (RA-388).
                    Err(error) => {
                        tracing::warn!(session = %name, worktree = %worktree, error = %error.message, "overlap scan skipped a checkout it could not read");
                        continue;
                    }
                };
                let symbols = changed_symbols(path, &files);
                let changed: Changed = std::sync::Arc::new((files, symbols));
                scanned.insert(checkout.clone(), changed.clone());
                changed
            }
        };
        changes.insert(name.clone(), (checkout, changed));
    }
    Ok(changes)
}

/// How long an inactive overlap row is kept, so an ack survives a collision that comes and goes.
const INACTIVE_KEEP_DAYS: i64 = 30;

/// The store half again: cross the collected changes with the claims and write the findings.
/// The changes were read before the transaction opened, so a session that closed meanwhile is
/// dropped from them first: its close deactivated its overlaps, and a finding from the stale
/// snapshot would turn them back on (RA-389).
fn apply_scan(tx: &Transaction, project_id: Id, now: &str, mut changes: Changes) -> Result<Vec<Overlap>, BusError> {
    crate::handlers::workspace::get_project(tx, project_id)?;
    let live = tx.prepare_cached("SELECT name FROM sessions WHERE project_id=?1 AND state!='closed'").bus()?
        .query_map([project_id], |r| r.get::<_, String>(0)).bus()?
        .collect::<rusqlite::Result<BTreeSet<_>>>().bus()?;
    changes.retain(|name, _| live.contains(name));
    let mut findings = BTreeMap::<String, Finding>::new();
    let names = changes.keys().cloned().collect::<Vec<_>>();
    for left in 0..names.len() {
        for right in left + 1..names.len() {
            let a = &names[left];
            let b = &names[right];
            let (a_checkout, a_changed) = &changes[a];
            let (b_checkout, b_changed) = &changes[b];
            // One checkout's edits are not two sessions colliding: they are the same edits,
            // seen twice (RA-174).
            if a_checkout == b_checkout {
                continue;
            }
            let (af, asyms) = &**a_changed;
            let (bf, bsyms) = &**b_changed;
            for path in af.intersection(bf) {
                let file = Finding {
                    sessions: vec![a.clone(), b.clone()],
                    path: path.clone(),
                    symbol: None,
                    kind: OverlapKind::File,
                };
                findings.insert(fingerprint(project_id, &file), file);
                if let (Some(sa), Some(sb)) = (asyms.get(path), bsyms.get(path)) {
                    for symbol in sa.intersection(sb) {
                        let finding = Finding {
                            sessions: vec![a.clone(), b.clone()],
                            path: path.clone(),
                            symbol: Some(symbol.clone()),
                            kind: OverlapKind::Symbol,
                        };
                        findings.insert(fingerprint(project_id, &finding), finding);
                    }
                }
            }
        }
    }

    let mut claim_stmt = tx.prepare_cached(
        "SELECT c.session, c.path, c.symbol
         FROM claims c
         JOIN sessions s ON s.id=c.session_id
         WHERE c.project_id=?1 AND s.state!='closed'
         ORDER BY c.path, c.symbol, c.session",
    ).bus()?;
    let claims = claim_stmt
        .query_map([project_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    for (session, path, symbol) in &claims {
        let own = Finding {
            sessions: vec![session.clone()],
            path: path.clone(),
            symbol: (!symbol.is_empty()).then(|| symbol.clone()),
            kind: OverlapKind::Claim,
        };
        findings.insert(fingerprint(project_id, &own), own);
        let own_checkout = changes.get(session).map(|(checkout, _)| checkout);
        for (other, (checkout, changed)) in &changes {
            let (files, symbols) = &**changed;
            if other == session || Some(checkout) == own_checkout || !files.contains(path) {
                continue;
            }
            let conflicts = symbol.is_empty()
                || symbols.get(path).is_some_and(|set| set.iter().any(|changed| claimed(changed, symbol)));
            if conflicts {
                let mut pair = vec![session.clone(), other.clone()];
                pair.sort();
                pair.dedup();
                let finding = Finding {
                    sessions: pair,
                    path: path.clone(),
                    symbol: (!symbol.is_empty()).then(|| symbol.clone()),
                    kind: OverlapKind::Claim,
                };
                findings.insert(fingerprint(project_id, &finding), finding);
            }
        }
    }
    for i in 0..claims.len() {
        for j in i + 1..claims.len() {
            let a = &claims[i];
            let b = &claims[j];
            if a.0 == b.0 || a.1 != b.1 {
                continue;
            }
            if !a.2.is_empty() && !b.2.is_empty() && a.2 != b.2 {
                continue;
            }
            let mut pair = vec![a.0.clone(), b.0.clone()];
            pair.sort();
            pair.dedup();
            let symbol = if a.2.is_empty() {
                b.2.clone()
            } else {
                a.2.clone()
            };
            let finding = Finding {
                sessions: pair,
                path: a.1.clone(),
                symbol: (!symbol.is_empty()).then_some(symbol),
                kind: OverlapKind::Claim,
            };
            findings.insert(fingerprint(project_id, &finding), finding);
        }
    }

    // Every scan rewrote every historical row too, and nothing ever removed one, while new
    // session pairs keep minting them (RA-390). A row already inactive and long unseen goes;
    // then only the active rows are touched.
    tx.prepare_cached("DELETE FROM overlaps WHERE project_id=?1 AND active=0 AND last_seen<?2").bus()?
        .execute(params![project_id, crate::time::days_ago(INACTIVE_KEEP_DAYS)]).bus()?;
    tx.prepare_cached("UPDATE overlaps SET active=0 WHERE project_id=?1 AND active=1").bus()?
        .execute([project_id]).bus()?;
    for (fingerprint, finding) in findings {
        tx.execute(
            "INSERT INTO overlaps(project_id, fingerprint, sessions, path, symbol, kind, first_seen, last_seen, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 1)
             ON CONFLICT(fingerprint) DO UPDATE SET sessions=excluded.sessions, path=excluded.path,
               symbol=excluded.symbol, kind=excluded.kind, last_seen=excluded.last_seen, active=1",
            params![project_id, fingerprint, serde_json::to_string(&finding.sessions).bus()?, finding.path, finding.symbol, kind_str(finding.kind), now],
        ).bus()?;
    }
    list(tx, project_id)
}

/// How old the stored scan may be before `overlap.list` refreshes it.
const RESCAN_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
/// (engine, project) → when its last background scan began.
static RESCANNED: std::sync::Mutex<Option<std::collections::HashMap<(usize, Id), std::time::Instant>>> =
    std::sync::Mutex::new(None);

fn rescan_due(engine: &Engine, project_id: Id) -> bool {
    let key = (engine as *const Engine as usize, project_id);
    let mut rescanned = RESCANNED.lock().unwrap_or_else(|poison| poison.into_inner());
    let rescanned = rescanned.get_or_insert_with(Default::default);
    if rescanned.get(&key).is_some_and(|at| at.elapsed() < RESCAN_AFTER) {
        return false;
    }
    rescanned.insert(key, std::time::Instant::now());
    true
}

/// The scan `overlap.flag` runs, on behalf of whoever reads the list. File and symbol overlaps
/// exist only once a scan finds them, and only an agent's flag (or a test's `overlap.scan`) ran
/// one, so the list a person opens was empty or as old as the last flag (RA-175). Same two
/// halves as the staged ops: the checkouts are read with the store unlocked. `overlap.changed`
/// goes out only when the set of overlaps changed, so a window refreshing on it settles.
pub(crate) fn rescan(engine: &Engine, project_id: Id) -> Result<(), BusError> {
    let targets = scan_targets(&engine.store.lock(), project_id)?;
    let changes = collect_changes(&targets)?;
    let now = crate::time::now();
    let (before, after) = {
        let mut conn = engine.store.lock();
        let tx = conn.transaction().map_err(crate::engine::internal)?;
        let before: Vec<Id> = list(&tx, project_id)?.iter().map(|overlap| overlap.id).collect();
        let after = apply_scan(&tx, project_id, &now, changes)?;
        tx.commit().map_err(crate::engine::internal)?;
        (before, after)
    };
    if before != after.iter().map(|overlap| overlap.id).collect::<Vec<_>>() {
        engine.emit_system("overlap.changed", serde_json::json!({"project_id": project_id, "count": after.len()}));
    }
    Ok(())
}

pub fn register(e: &mut Engine) {
    e.register::<List>(|ctx, p| {
        crate::handlers::workspace::get_project(ctx.tx(), p.project_id)?;
        if let Some(sid) = ctx.actor_session_id() {
            let own = sessions::by_id(ctx.tx(), sid)?
                .ok_or_else(|| BusError::actor("bound session vanished"))?;
            if own.session.project_id != p.project_id {
                return Err(BusError::not_own("project"));
            }
        }
        // Answered from what is stored; a stale scan is refreshed behind it, and the window
        // hears `overlap.changed` if that changes anything.
        let project_id = p.project_id;
        ctx.after_commit(move |engine| {
            if engine.instance == crate::Instance::Test || !rescan_due(&engine, project_id) {
                return;
            }
            let _ = std::thread::Builder::new().name("overlap-scan".into()).spawn(move || {
                crate::background_priority();
                if let Err(error) = rescan(&engine, project_id) {
                    tracing::debug!(project_id, error = %error.message, "background overlap scan failed");
                }
            });
        });
        Ok(ListOut {
            overlaps: list(ctx.tx(), p.project_id)?,
        })
    });
    e.register_staged::<Flag, Changes>(
        |ctx, p| {
            let session_id = ctx.actor_session_id();
            relative_path(&p.path)?;
            let targets = ctx.read(|conn| {
                actor_session_in(conn, session_id, p.project_id)?;
                scan_targets(conn, p.project_id)
            })?;
            collect_changes(&targets)
        },
        |ctx: &mut Ctx, p, changes| {
            let own = actor_session(ctx, p.project_id)?;
            let path = relative_path(&p.path)?;
            let symbol = p.symbol.unwrap_or_default().trim().to_string();
            ctx.tx().execute(
                "INSERT INTO claims(project_id, session_id, session, path, symbol, note, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(session_id, path, symbol) DO UPDATE SET note=excluded.note, updated_at=excluded.updated_at",
                params![p.project_id, own.session.id, own.session.name, path, symbol, p.note, ctx.now],
            ).bus()?;
            let overlaps = apply_scan(ctx.tx(), p.project_id, &ctx.now, changes)?;
            // The claim just made: same path and symbol, and the caller's own row before a pair
            // that shares it (RA-391).
            let wanted = (!symbol.is_empty()).then_some(symbol.as_str());
            let overlap = overlaps.into_iter()
                .filter(|overlap| overlap.kind == OverlapKind::Claim && overlap.path == path && overlap.symbol.as_deref() == wanted && overlap.sessions.contains(&own.session.name))
                .min_by_key(|overlap| overlap.sessions.len() != 1)
                .ok_or_else(|| BusError::internal("claim did not produce an overlap row"))?;
            ctx.set_project(p.project_id);
            ctx.emit("overlap.changed", serde_json::to_value(&overlap).bus()?);
            Ok(overlap)
        },
    );
    e.register::<Ack>(|ctx: &mut Ctx, p| {
        let project_id: Id = ctx
            .tx()
            .query_row(
                "SELECT project_id FROM overlaps WHERE id=?1 AND active=1",
                [p.overlap_id],
                |r| r.get(0),
            )
            .optional()
            .bus()?
            .ok_or_else(|| {
                BusError::not_found(
                    "overlap.not_found",
                    format!("no active overlap {}", p.overlap_id),
                )
            })?;
        let own = actor_session(ctx, project_id)?;
        let raw: String = ctx
            .tx()
            .query_row(
                "SELECT acked_by FROM overlaps WHERE id=?1",
                [p.overlap_id],
                |r| r.get(0),
            )
            .bus()?;
        let mut acked: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        if !acked.contains(&own.session.name) {
            acked.push(own.session.name);
            acked.sort();
        }
        ctx.tx()
            .execute(
                "UPDATE overlaps SET acked_by=?1, last_seen=?2 WHERE id=?3",
                params![serde_json::to_string(&acked).bus()?, ctx.now, p.overlap_id],
            )
            .bus()?;
        let overlap = ctx
            .tx()
            .query_row(
                "SELECT * FROM overlaps WHERE id=?1",
                [p.overlap_id],
                overlap_row,
            )
            .bus()?;
        ctx.set_project(project_id);
        ctx.emit("overlap.changed", serde_json::to_value(&overlap).bus()?);
        Ok(overlap)
    });
    e.register_staged::<Scan, Changes>(
        |ctx, p| {
            let targets = ctx.read(|conn| scan_targets(conn, p.project_id))?;
            collect_changes(&targets)
        },
        |ctx: &mut Ctx, p, changes| {
            let overlaps = apply_scan(ctx.tx(), p.project_id, &ctx.now, changes)?;
            ctx.set_project(p.project_id);
            ctx.emit(
                "overlap.changed",
                serde_json::json!({"project_id":p.project_id, "count":overlaps.len()}),
            );
            Ok(ListOut { overlaps })
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_bus::{Actor, Request};
    use std::path::PathBuf;

    #[test]
    fn same_named_items_are_told_apart() {
        let before = b"struct A;\nstruct B;\nimpl A { fn new() -> A { A } }\nimpl B { fn new() -> B { B } }\n\
            impl std::fmt::Display for A { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }\n\
            mod inner { fn helper() {} }\n";
        let path = Path::new("lib.rs");
        let symbols = symbols(path, before);
        for name in ["A", "B", "impl A", "impl A::new", "impl B::new", "impl std::fmt::Display for A::fmt", "inner::helper"] {
            assert!(symbols.contains_key(name), "{name} missing from {:?}", symbols.keys());
        }
        let after = String::from_utf8(before.to_vec()).unwrap().replace("fn new() -> B { B }", "fn new() -> B { let b = B; b }");
        let changed = super::symbols(path, after.as_bytes());
        let differ: Vec<_> = changed.iter().filter(|(name, hash)| symbols.get(*name) != Some(*hash)).map(|(name, _)| name.as_str()).collect();
        assert_eq!(differ, ["impl B", "impl B::new"], "an edit to one `new` is that `new` alone");
        assert!(claimed("impl B::new", "new") && claimed("impl B::new", "impl B::new") && claimed("impl B::new#2", "new"));
        assert!(!claimed("impl B::new", "B"));
    }

    fn git(repo: &Path, args: &[&str]) {
        assert!(std::process::Command::new("git").arg("-C").arg(repo).args(args).status().unwrap().success(), "git {args:?}");
    }

    fn ok(engine: &Engine, op: &str, payload: serde_json::Value) -> serde_json::Value {
        engine.dispatch(Request::new(Actor::User, op, payload), crate::engine::Door::InProcess).into_result()
            .unwrap_or_else(|error| panic!("{op}: {} {}", error.code, error.message))
    }

    #[test]
    fn a_shared_checkout_is_scanned_once_and_never_overlaps_itself_and_a_reader_rescans() {
        let root = tempfile::tempdir().unwrap();
        let ws = std::fs::canonicalize(root.path()).unwrap().join("ws");
        let repo = ws.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("lib.rs"), "pub fn one() -> i32 { 1 }\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let engine = Engine::new(crate::Instance::Test, crate::Store::open_memory().unwrap());
        ok(&engine, "workspace.create", serde_json::json!({"path": ws}));
        ok(&engine, "project.add", serde_json::json!({"workspace_id": 1, "path": repo}));
        let session = |extra: serde_json::Value| {
            let mut payload = serde_json::json!({"project_id": 1, "provider": "codex"});
            payload.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let created = ok(&engine, "session.create", payload);
            (created["name"].as_str().unwrap().to_string(), PathBuf::from(created["worktree"].as_str().unwrap()))
        };
        let (a, a_wt) = session(serde_json::json!({}));
        let (b, _) = session(serde_json::json!({"worktree": a_wt}));
        let (c, c_wt) = session(serde_json::json!({}));
        std::fs::write(a_wt.join("lib.rs"), "pub fn one() -> i32 { 2 }\n").unwrap();
        std::fs::write(c_wt.join("lib.rs"), "pub fn one() -> i32 { 3 }\n").unwrap();

        let mut events = engine.subscribe();
        assert!(ok(&engine, "overlap.list", serde_json::json!({"project_id": 1}))["overlaps"].as_array().unwrap().is_empty());
        rescan(&engine, 1).unwrap();
        assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|event| event.ev == "overlap.changed"));
        let listed = ok(&engine, "overlap.list", serde_json::json!({"project_id": 1}));
        let pairs: BTreeSet<Vec<String>> = listed["overlaps"].as_array().unwrap().iter()
            .filter(|overlap| overlap["kind"] == "file")
            .map(|overlap| serde_json::from_value(overlap["sessions"].clone()).unwrap()).collect();
        let pair = |x: &str, y: &str| { let mut p = vec![x.to_string(), y.to_string()]; p.sort(); p };
        assert!(pairs.contains(&pair(&a, &c)) && pairs.contains(&pair(&b, &c)), "{listed}");
        assert!(!pairs.contains(&pair(&a, &b)), "a checkout overlapped itself: {listed}");
        // Nothing changed: no event for a window to refresh on.
        rescan(&engine, 1).unwrap();
        assert!(!std::iter::from_fn(|| events.try_recv().ok()).any(|event| event.ev == "overlap.changed"));

        // A flag answers with the claim it made, symbol and all, not the first one on the path
        // (RA-391), and `./` names the same file git status does (RA-387).
        let flag = |path: &str, symbol: Option<&str>| engine.dispatch(
            Request::new(Actor::agent(&a), "overlap.flag", serde_json::json!({"project_id": 1, "path": path, "symbol": symbol})),
            crate::engine::Door::InProcess,
        ).into_result().unwrap_or_else(|error| panic!("overlap.flag: {} {}", error.code, error.message));
        let whole = flag("./lib.rs", None);
        assert_eq!((whole["path"].as_str(), whole["symbol"].as_str()), (Some("lib.rs"), None), "{whole}");
        let one = flag("lib.rs", Some("one"));
        assert_eq!(one["symbol"], "one", "{one}");
        assert_eq!(one["sessions"], serde_json::json!([a]), "{one}");

        // A scan read before a session closed must not bring its overlaps back (RA-389), and a
        // checkout that cannot be read is skipped rather than failing the scan (RA-388).
        let broken = root.path().join("not-a-repo");
        std::fs::create_dir_all(&broken).unwrap();
        let mut targets = scan_targets(&engine.store.lock(), 1).unwrap();
        targets.push(("ghost".into(), broken.display().to_string()));
        let stale = collect_changes(&targets).unwrap();
        assert!(stale.contains_key(&c) && !stale.contains_key("ghost"));
        engine.store.lock().execute("UPDATE sessions SET state='closed' WHERE name=?1", [&c]).unwrap();
        let after = {
            let mut conn = engine.store.lock();
            let tx = conn.transaction().unwrap();
            let after = apply_scan(&tx, 1, &crate::time::now(), stale).unwrap();
            tx.commit().unwrap();
            after
        };
        assert!(after.iter().all(|overlap| !overlap.sessions.contains(&c)), "{after:?}");
    }

    #[test]
    fn claim_paths_have_one_spelling() {
        assert_eq!(normalize_relative(" ./src//a.rs ").as_deref(), Some("src/a.rs"));
        assert_eq!(normalize_relative("src/./a.rs/").as_deref(), Some("src/a.rs"));
        for bad in ["", ".", "/etc/passwd", "../a.rs", "src/../../a.rs"] {
            assert_eq!(normalize_relative(bad), None, "{bad:?}");
        }
    }
}
