//! The Git panel's reading of engine results: which files are staged, unstaged or in conflict,
//! the one-line summary of a change list, where a unified diff's lines land in the file, and the
//! commit graph's lanes. Rows are `git.status` / `git.log` JSON, read as the panel reads them.
use serde_json::Value;

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

/// Whether a file still holds a `<<<<<<<` … `>>>>>>>` block, as git writes for a conflict.
pub fn has_conflict_markers(content: &str) -> bool {
    let mut open = false;
    for line in content.lines() {
        if line.starts_with("<<<<<<< ") || line == "<<<<<<<" {
            open = true;
        } else if open && (line.starts_with(">>>>>>> ") || line == ">>>>>>>") {
            return true;
        }
    }
    false
}

/// Whether a porcelain entry is unmerged: git marks these DD, AU, UD, UA, DU, AA and UU,
/// so either column U, or both added, or both deleted.
pub fn is_conflict(file: &Value) -> bool {
    let (index, worktree) = (text(file, "index"), text(file, "worktree"));
    index == "U" || worktree == "U" || matches!((index, worktree), ("A", "A") | ("D", "D"))
}
pub fn is_staged(file: &Value) -> bool {
    !matches!(text(file, "index").trim(), "" | "?" | "!")
}
pub fn is_unstaged(file: &Value) -> bool {
    !matches!(text(file, "worktree").trim(), "" | "!")
}
/// A repository path as its directory (`""` at the root) and file name.
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((directory, name)) => (directory, name),
        None => ("", path),
    }
}

/// `@@ -a,b +c,d @@` as zero-based first lines and lengths.
pub fn hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let ranges = line.strip_prefix("@@ -")?.split_once(" @@")?.0;
    let (old, new) = ranges.split_once(" +")?;
    let parse = |range: &str| -> Option<(usize, usize)> {
        let (start, length) = range.split_once(',').unwrap_or((range, "1"));
        let (start, length): (usize, usize) = (start.parse().ok()?, length.parse().ok()?);
        // An empty range names the line before it.
        Some((if length == 0 { start } else { start.saturating_sub(1) }, length))
    };
    let (old_start, old_length) = parse(old)?;
    let (new_start, new_length) = parse(new)?;
    Some((old_start, old_length, new_start, new_length))
}

/// Lines as the engine's diff (similar's `from_lines`) and GtkTextBuffer count them: ended by
/// LF, CRLF or a lone CR, which `str::lines` would leave inside a line (RA-454).
pub fn diff_lines(text: &str) -> Vec<&str> {
    let (bytes, mut lines, mut start, mut at) = (text.as_bytes(), Vec::new(), 0, 0);
    while at < bytes.len() {
        if matches!(bytes[at], b'\n' | b'\r') {
            lines.push(&text[start..at]);
            if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
                at += 1;
            }
            start = at + 1;
        }
        at += 1;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Zero-based lines removed from the old text and added to the new one.
pub fn diff_marks(unified: &str) -> (Vec<usize>, Vec<usize>) {
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let (mut old, mut new, mut inside) = (0, 0, false);
    for line in diff_lines(unified) {
        if let Some((old_start, _, new_start, _)) = hunk_header(line) {
            (old, new, inside) = (old_start, new_start, true);
            continue;
        }
        if !inside {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'-') => {
                removed.push(old);
                old += 1;
            }
            Some(b'+') => {
                added.push(new);
                new += 1;
            }
            Some(b'\\') => {}
            _ => {
                old += 1;
                new += 1;
            }
        }
    }
    (removed, added)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    Same,
    Removed,
    Added,
}

/// The whole new file with each hunk's removed lines shown above their replacements.
pub fn inline_diff<'a>(unified: &'a str, new: &'a str) -> Vec<(Mark, &'a str)> {
    let lines = diff_lines(new);
    let mut out = Vec::new();
    let (mut cursor, mut inside) = (0, false);
    for line in diff_lines(unified) {
        if let Some((_, _, new_start, _)) = hunk_header(line) {
            let start = new_start.min(lines.len());
            if start > cursor {
                out.extend(lines[cursor..start].iter().map(|line| (Mark::Same, *line)));
                cursor = start;
            }
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'-') => out.push((Mark::Removed, &line[1..])),
            Some(b'+') => {
                out.push((Mark::Added, &line[1..]));
                cursor += 1;
            }
            Some(b'\\') => {}
            _ => {
                out.push((Mark::Same, line.get(1..).unwrap_or("")));
                cursor += 1;
            }
        }
    }
    if cursor < lines.len() {
        out.extend(lines[cursor..].iter().map(|line| (Mark::Same, *line)));
    }
    out
}

/// Changed files grouped by kind, a few paths each: the tooltip on a change group.
pub fn change_summary(files: &[Value]) -> String {
    let mut groups: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for file in files {
        let statuses = format!("{}{}", text(file, "index"), text(file, "worktree"));
        let category = if is_conflict(file) {
            "Conflicted"
        } else if statuses.contains('R') {
            "Renamed"
        } else if statuses.contains('D') {
            "Deleted"
        } else if statuses.contains('A') || statuses.contains('?') {
            "Added"
        } else {
            "Modified"
        };
        groups.entry(category).or_default().push(text(file, "path"));
    }
    groups
        .into_iter()
        .map(|(kind, paths)| {
            let visible = paths.iter().take(3).copied().collect::<Vec<_>>().join(", ");
            format!(
                "{kind} {}: {visible}{}",
                paths.len(),
                if paths.len() > 3 {
                    format!(" +{} more", paths.len() - 3)
                } else {
                    String::new()
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One commit's row in the graph: its lane, and the lines drawn through the row as
/// `(kind, lane)` — 0 passes straight through, 1 comes in from above, 2 leaves for a parent.
#[derive(Debug)]
pub struct GraphRow {
    pub lane: usize,
    pub links: Vec<(u8, usize)>,
    pub merge: bool,
}
// Relay-2 commitGraph.ts: reserve parent lanes until the corresponding commit arrives.
pub fn commit_graph(commits: &[Value]) -> Vec<GraphRow> {
    let mut active: Vec<Option<String>> = Vec::new();
    let mut result = Vec::new();
    for commit in commits {
        let before = active.clone();
        let waiting: Vec<_> = before
            .iter()
            .enumerate()
            .filter(|(_, sha)| sha.as_deref() == Some(text(commit, "sha")))
            .map(|(lane, _)| lane)
            .collect();
        let lane = waiting.first().copied().unwrap_or_else(|| {
            active
                .iter()
                .position(Option::is_none)
                .unwrap_or(active.len())
        });
        let mut links = Vec::new();
        for &index in &waiting {
            active[index] = None;
            links.push((1, index));
        }
        if lane >= active.len() {
            active.resize(lane + 1, None);
        } else {
            active[lane] = None;
        }
        let parents: Vec<_> = commit["parents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut outgoing = Vec::new();
        for (index, parent) in parents.iter().enumerate() {
            let target = if index == 0 {
                lane
            } else {
                active
                    .iter()
                    .position(|sha| sha.as_deref() == Some(parent))
                    .unwrap_or_else(|| {
                        active
                            .iter()
                            .position(Option::is_none)
                            .unwrap_or(active.len())
                    })
            };
            if target >= active.len() {
                active.resize(target + 1, None);
            }
            active[target] = Some((*parent).to_string());
            if !outgoing.contains(&target) {
                outgoing.push(target);
            }
        }
        for target in outgoing {
            links.push((2, target));
        }
        for index in 0..before.len().max(active.len()) {
            if before.get(index).is_some_and(Option::is_some)
                && active.get(index).is_some_and(Option::is_some)
                && !waiting.contains(&index)
            {
                links.push((0, index));
            }
        }
        while active.last() == Some(&None) {
            active.pop();
        }
        result.push(GraphRow {
            lane,
            links,
            merge: parents.len() > 1,
        });
    }
    result
}

#[cfg(test)]
mod graph_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn precommit_summary_names_changes_and_counts_without_a_model() {
        let rows = vec![
            json!({"path":"new.rs","index":"A"}),
            json!({"path":"old.rs","worktree":"D"}),
            json!({"path":"moved.rs","index":"R"}),
            json!({"path":"edit.rs","worktree":"M"}),
        ];
        let summary = change_summary(&rows);
        assert!(summary.contains("Added 1: new.rs"));
        assert!(summary.contains("Deleted 1: old.rs"));
        assert!(summary.contains("Renamed 1: moved.rs"));
        assert!(summary.contains("Modified 1: edit.rs"));
    }
    #[test]
    fn unified_hunks_mark_the_lines_each_side_changed() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";
        let unified = "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -8,3 +8,4 @@\n h\n i\n j\n+k\n";
        assert_eq!(diff_marks(unified), (vec![1], vec![1, 10]));
        let inline = inline_diff(unified, new);
        assert_eq!(inline.len(), old.lines().count() + 2);
        assert_eq!(inline[1], (Mark::Removed, "b"));
        assert_eq!(inline[2], (Mark::Added, "B"));
        // Untouched lines between hunks come from the new text.
        assert_eq!(inline[5], (Mark::Same, "e"));
        assert_eq!(inline.last(), Some(&(Mark::Added, "k")));
        // A new file and a deleted one use empty ranges.
        assert_eq!(hunk_header("@@ -0,0 +1,2 @@"), Some((0, 0, 0, 2)));
        assert_eq!(hunk_header("@@ -1,2 +0,0 @@ fn main"), Some((0, 2, 0, 0)));
        assert_eq!(hunk_header("@@ -3 +3 @@"), Some((2, 1, 2, 1)));
        assert_eq!(diff_marks("@@ -1,2 +0,0 @@\n-x\n-y\n"), (vec![0, 1], vec![]));
        assert_eq!(
            inline_diff("@@ -0,0 +1,2 @@\n+x\n+y\n\\ No newline at end of file\n", "x\ny"),
            vec![(Mark::Added, "x"), (Mark::Added, "y")]
        );
        // A lone CR ends a line for the engine's diff and the buffer alike (RA-454).
        assert_eq!(diff_lines("a\r\nb\rc\n\nd"), vec!["a", "b", "c", "", "d"]);
        assert_eq!(diff_marks("@@ -1,3 +1,2 @@\n a\r-b\r c\n"), (vec![1], vec![]));
        assert_eq!(
            inline_diff("@@ -1,2 +1,2 @@\n a\r-b\r+B\r", "a\rB\r"),
            vec![(Mark::Same, "a"), (Mark::Removed, "b"), (Mark::Added, "B")]
        );
    }
    #[test]
    fn staged_and_unstaged_halves_of_one_file_are_both_listed() {
        let partial = json!({"path":"a.rs","index":"M","worktree":"M"});
        let untracked = json!({"path":"b.rs","index":"","worktree":"?"});
        let conflict = json!({"path":"c.rs","index":"U","worktree":"U"});
        assert!(is_staged(&partial) && is_unstaged(&partial));
        assert!(!is_staged(&untracked) && is_unstaged(&untracked));
        assert!(is_conflict(&conflict));
        // Both added and both deleted are unmerged too; a lone add or delete is not.
        assert!(is_conflict(&json!({"path":"d.rs","index":"A","worktree":"A"})));
        assert!(is_conflict(&json!({"path":"e.rs","index":"D","worktree":"D"})));
        assert!(!is_conflict(&json!({"path":"f.rs","index":"A","worktree":"M"})));
        assert!(!is_conflict(&json!({"path":"g.rs","index":"D","worktree":""})));
        assert!(change_summary(&[json!({"path":"d.rs","index":"A","worktree":"A"})]).starts_with("Conflicted 1"));
        assert!(has_conflict_markers("a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> theirs\nd\n"));
        // A Markdown underline, or a lone marker, is not a conflict.
        assert!(!has_conflict_markers("Title\n=======\n"));
        assert!(!has_conflict_markers(">>>>>>> quoted\n<<<<<<< later\n"));
    }
    #[test]
    fn merge_lanes_rejoin_at_the_shared_parent() {
        let commits = vec![
            json!({"sha":"merge","parents":["left","right"]}),
            json!({"sha":"left","parents":["base"]}),
            json!({"sha":"right","parents":["base"]}),
            json!({"sha":"base","parents":[]}),
        ];
        let graph = commit_graph(&commits);
        assert!(graph[0].merge);
        assert_eq!(
            graph.iter().map(|row| row.lane).collect::<Vec<_>>(),
            vec![0, 0, 1, 0]
        );
        assert!(graph[3].links.contains(&(1, 0)) && graph[3].links.contains(&(1, 1)));
        assert!(!graph[3].links.iter().any(|(kind, _)| *kind == 2));
    }
}
