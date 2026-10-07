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

fn relative_path(raw: &str) -> Result<String, BusError> {
    let path = Path::new(raw);
    if raw.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(BusError::invalid(
            "overlap.path",
            "path must be a normalized project-relative path",
        ));
    }
    Ok(path.to_string_lossy().replace('\\', "/"))
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

fn symbols(path: &Path, source: &[u8]) -> BTreeMap<String, String> {
    if source.len() > 2 * 1024 * 1024 {
        return BTreeMap::new();
    }
    let Some(Some(tree)) = with_parser(path, |parser| parser.parse(source, None)) else {
        return BTreeMap::new();
    };
    let mut stack = vec![tree.root_node()];
    let mut out = BTreeMap::new();
    while let Some(node) = stack.pop() {
        if is_symbol(node.kind()) {
            if let Some(name) = node_name(node, source) {
                let bytes = &source[node.byte_range()];
                out.insert(name, hash_hex(bytes));
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    out
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

/// Per session: the files its checkout changed, and per changed file the symbols that differ
/// from HEAD.
type Changes = BTreeMap<String, (BTreeSet<String>, BTreeMap<String, BTreeSet<String>>)>;

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
    for (name, worktree) in targets {
        let path = Path::new(worktree);
        if !path.is_dir() {
            continue;
        }
        let files = changed_files(path)?;
        let symbols = changed_symbols(path, &files);
        changes.insert(name.clone(), (files, symbols));
    }
    Ok(changes)
}

/// The store half again: cross the collected changes with the claims and write the findings.
/// A session that closed while the scan ran simply has no row to collide with.
fn apply_scan(tx: &Transaction, project_id: Id, now: &str, changes: Changes) -> Result<Vec<Overlap>, BusError> {
    crate::handlers::workspace::get_project(tx, project_id)?;
    let mut findings = BTreeMap::<String, Finding>::new();
    let names = changes.keys().cloned().collect::<Vec<_>>();
    for left in 0..names.len() {
        for right in left + 1..names.len() {
            let a = &names[left];
            let b = &names[right];
            let (af, asyms) = &changes[a];
            let (bf, bsyms) = &changes[b];
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
        for (other, (files, symbols)) in &changes {
            if other == session || !files.contains(path) {
                continue;
            }
            let conflicts =
                symbol.is_empty() || symbols.get(path).is_some_and(|set| set.contains(symbol));
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

    tx.execute(
        "UPDATE overlaps SET active=0 WHERE project_id=?1",
        [project_id],
    )
    .bus()?;
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
            let overlap = overlaps.into_iter().find(|overlap| overlap.kind == OverlapKind::Claim && overlap.path == path && overlap.sessions.contains(&own.session.name))
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
