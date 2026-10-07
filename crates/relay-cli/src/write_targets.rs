//! What a tool call is about to write, read from its input before it runs, so the PreToolUse
//! hook can put each file through the same write gate as Claude's Write and Edit tools.
//!
//! Two inputs: a Codex `apply_patch` envelope, which names every file it touches and can be
//! applied in memory to get the new text; and a shell command line, where only the obvious
//! writers are recognised — redirections, `tee`, `sed -i`, `perl -i`, `truncate`, `rm`,
//! `cp`/`mv`/`install`, `dd of=`, and `sh -c` around any of them. A script that opens files
//! itself (`python -c`, a build tool) is not seen: this narrows the gap, it does not close it.

use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- apply_patch

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchOp {
    Add { path: String, text: String },
    Delete { path: String },
    Update { path: String, move_to: Option<String>, chunks: Vec<Chunk> },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chunk {
    /// The `@@ <line>` anchor: the hunk starts after the first line matching it.
    pub context: Option<String>,
    pub old: Vec<String>,
    pub new: Vec<String>,
    pub end_of_file: bool,
}

/// The files in an apply_patch envelope (`*** Begin Patch` … `*** End Patch`). Lines outside it,
/// such as a heredoc wrapper, are ignored.
pub fn parse_patch(patch: &str) -> Result<Vec<PatchOp>> {
    let lines: Vec<&str> = patch.lines().collect();
    let begin = lines.iter().position(|l| l.trim() == "*** Begin Patch").ok_or_else(|| anyhow!("apply_patch input has no *** Begin Patch line"))?;
    let end = lines.iter().rposition(|l| l.trim() == "*** End Patch").filter(|end| *end > begin)
        .ok_or_else(|| anyhow!("apply_patch input has no *** End Patch line"))?;
    let mut ops = Vec::new();
    let mut i = begin + 1;
    while i < end {
        let line = lines[i];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let mut text = String::new();
            i += 1;
            while i < end && !lines[i].starts_with("*** ") {
                let added = lines[i].strip_prefix('+').ok_or_else(|| anyhow!("added file {path} has a line without '+': {:?}", lines[i]))?;
                text.push_str(added);
                text.push('\n');
                i += 1;
            }
            ops.push(PatchOp::Add { path: path.trim().to_string(), text });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(PatchOp::Delete { path: path.trim().to_string() });
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = None;
            if let Some(to) = lines.get(i).and_then(|l| l.strip_prefix("*** Move to: ")).filter(|_| i < end) {
                move_to = Some(to.trim().to_string());
                i += 1;
            }
            let mut chunks: Vec<Chunk> = Vec::new();
            while i < end && (!lines[i].starts_with("*** ") || lines[i] == "*** End of File") {
                let l = lines[i];
                if l == "*** End of File" {
                    if let Some(chunk) = chunks.last_mut() { chunk.end_of_file = true; }
                } else if l == "@@" || l.starts_with("@@ ") {
                    let context = l.strip_prefix("@@ ").map(str::to_string).filter(|c| !c.trim().is_empty());
                    chunks.push(Chunk { context, ..Chunk::default() });
                } else {
                    // The first hunk may leave out its @@ line.
                    if chunks.is_empty() { chunks.push(Chunk::default()); }
                    let chunk = chunks.last_mut().expect("pushed above");
                    match l.chars().next() {
                        Some('+') => chunk.new.push(l[1..].to_string()),
                        Some('-') => chunk.old.push(l[1..].to_string()),
                        Some(' ') => { chunk.old.push(l[1..].to_string()); chunk.new.push(l[1..].to_string()); }
                        // A blank context line whose leading space an editor trimmed.
                        None => { chunk.old.push(String::new()); chunk.new.push(String::new()); }
                        Some(_) => bail!("update of {path} has an unexpected line {l:?}"),
                    }
                }
                i += 1;
            }
            ops.push(PatchOp::Update { path: path.trim().to_string(), move_to, chunks });
        } else if line.trim().is_empty() || line.starts_with("*** Environment ID:") {
            i += 1;
        } else {
            bail!("apply_patch input has an unexpected line {line:?}");
        }
    }
    Ok(ops)
}

/// The file after an update's hunks, the way apply_patch finds them: in order, each after the
/// previous, matching exactly, then ignoring trailing and then surrounding whitespace. `None`
/// when a hunk is not found (apply_patch would then refuse too).
pub fn apply_chunks(old: &str, chunks: &[Chunk]) -> Option<String> {
    let mut lines: Vec<String> = old.split('\n').map(str::to_string).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut from = 0;
    for chunk in chunks {
        if let Some(context) = &chunk.context {
            from = seek(&lines, std::slice::from_ref(context), from, false)? + 1;
        }
        if chunk.old.is_empty() {
            replacements.push((lines.len(), 0, chunk.new.clone()));
            continue;
        }
        let (mut old_lines, mut new_lines) = (chunk.old.as_slice(), chunk.new.as_slice());
        let mut found = seek(&lines, old_lines, from, chunk.end_of_file);
        if found.is_none() && old_lines.last().is_some_and(String::is_empty) {
            // A trailing blank line in the hunk stands for the file's final newline.
            old_lines = &old_lines[..old_lines.len() - 1];
            if new_lines.last().is_some_and(String::is_empty) { new_lines = &new_lines[..new_lines.len() - 1]; }
            found = seek(&lines, old_lines, from, chunk.end_of_file);
        }
        let at = found?;
        replacements.push((at, old_lines.len(), new_lines.to_vec()));
        from = at + old_lines.len();
    }
    replacements.sort_by_key(|(at, _, _)| *at);
    for (at, len, new) in replacements.into_iter().rev() {
        lines.splice(at..at + len, new);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    Some(text)
}

/// A diff with one '-' per line a patch takes out and one '+' per line it puts in, for when the
/// hunks cannot be applied. Context lines, present on both sides, cancel.
pub fn chunk_diff(chunks: &[Chunk]) -> String {
    let mut diff = String::new();
    for chunk in chunks {
        let mut new: Vec<&String> = chunk.new.iter().collect();
        for line in &chunk.old {
            match new.iter().position(|n| *n == line) {
                Some(k) => { new.swap_remove(k); }
                None => diff.push_str("-\n"),
            }
        }
        diff.push_str(&"+\n".repeat(new.len()));
    }
    diff
}

fn seek(lines: &[String], pattern: &[String], from: usize, end_of_file: bool) -> Option<usize> {
    if pattern.len() > lines.len() {
        return None;
    }
    let last = lines.len() - pattern.len();
    let start = if end_of_file { last } else { from };
    let compare: [fn(&str, &str) -> bool; 3] = [|a, b| a == b, |a, b| a.trim_end() == b.trim_end(), |a, b| a.trim() == b.trim()];
    compare.iter().find_map(|same| {
        (start.min(last + 1)..=last).find(|&at| pattern.iter().enumerate().all(|(k, p)| same(&lines[at + k], p)))
    })
}

// ---------------------------------------------------------------- shell commands

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Replaced by content nobody can see yet (`>`, `cp` onto it, `truncate`).
    Overwrite,
    /// Grows (`>>`, `tee -a`).
    Append,
    /// Edited in place (`sed -i`).
    Modify,
    /// Removed (`rm`, the source of `mv`).
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellWrite {
    pub path: PathBuf,
    pub effect: Effect,
}

/// Files a command line would write, as absolute paths. Relative targets resolve against `cwd`,
/// following a literal `cd` earlier on the line. A target built from a variable, a glob or a
/// command substitution is skipped, as is anything under /dev or /proc.
pub fn shell_writes(command: &str, cwd: &Path) -> Vec<ShellWrite> {
    let mut out = Vec::new();
    let mut dir = Some(cwd.to_path_buf());
    for simple in split_commands(&tokenize(command)) {
        writes_of(&simple, &mut dir, &mut out, 0);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word { text: String, dynamic: bool },
    /// `;`, `&&`, `||`, `|`, `|&`, `&`, a newline, `(` or `)`: a command ends here.
    Sep,
    /// A redirection operator, without its fd number.
    Redir(&'static str),
}

/// A word and whether it holds an expansion that cannot be known before the shell runs.
type Word = (String, bool);

#[derive(Debug, Default)]
struct Simple {
    words: Vec<Word>,
    redirects: Vec<(&'static str, String, bool)>,
}

fn tokenize(line: &str) -> Vec<Token> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut dynamic = false;
    let mut in_word = false;
    let mut quoted = false;
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();
    let mut i = 0;
    macro_rules! flush {
        () => {
            if in_word {
                tokens.push(Token::Word { text: std::mem::take(&mut word), dynamic });
                in_word = false;
                dynamic = false;
                quoted = false;
            }
        };
    }
    // `$HOME`/`${HOME}` expand; any other expansion makes the word unknowable.
    let expand = |i: &mut usize, word: &mut String, dynamic: &mut bool| {
        let rest: String = chars[*i..].iter().take(8).collect();
        let name_end = |s: &str| s.chars().nth(5).is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        if rest.starts_with("${HOME}") {
            word.push_str(&home);
            *i += 7;
        } else if rest.starts_with("$HOME") && name_end(&rest) {
            word.push_str(&home);
            *i += 5;
        } else {
            *dynamic = true;
            word.push('$');
            *i += 1;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                in_word = true;
                quoted = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' { word.push(chars[i]); i += 1; }
                i += 1;
            }
            '"' => {
                in_word = true;
                quoted = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    match chars[i] {
                        '\\' if i + 1 < chars.len() && matches!(chars[i + 1], '"' | '\\' | '$' | '`') => { word.push(chars[i + 1]); i += 2; }
                        '$' => expand(&mut i, &mut word, &mut dynamic),
                        '`' => { dynamic = true; word.push('`'); i += 1; }
                        ch => { word.push(ch); i += 1; }
                    }
                }
                i += 1;
            }
            '\\' => {
                in_word = true;
                if let Some(&next) = chars.get(i + 1) {
                    if next != '\n' { word.push(next); }
                }
                i += 2;
            }
            '#' if !in_word => {
                while i < chars.len() && chars[i] != '\n' { i += 1; }
            }
            ' ' | '\t' => { flush!(); i += 1; }
            '\n' => {
                flush!();
                tokens.push(Token::Sep);
                i += 1;
                // Here-document bodies are data, not commands.
                for (delimiter, strip_tabs) in std::mem::take(&mut heredocs) {
                    while i < chars.len() {
                        let end = chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
                        let body: String = chars[i..end].iter().collect();
                        i = end + 1;
                        let body = if strip_tabs { body.trim_start_matches('\t') } else { &body };
                        if body == delimiter { break; }
                    }
                }
            }
            ';' | '(' | ')' | '`' => { flush!(); tokens.push(Token::Sep); i += 1; }
            '|' => {
                flush!();
                tokens.push(Token::Sep);
                i += if matches!(chars.get(i + 1), Some('|' | '&')) { 2 } else { 1 };
            }
            '&' => {
                flush!();
                match (chars.get(i + 1), chars.get(i + 2)) {
                    (Some('>'), Some('>')) => { tokens.push(Token::Redir("&>>")); i += 3; }
                    (Some('>'), _) => { tokens.push(Token::Redir("&>")); i += 2; }
                    (Some('&'), _) => { tokens.push(Token::Sep); i += 2; }
                    _ => { tokens.push(Token::Sep); i += 1; }
                }
            }
            '>' | '<' => {
                // A word made only of digits right before it is the fd number, not an argument.
                if in_word && !quoted && !word.is_empty() && word.chars().all(|c| c.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                    dynamic = false;
                }
                flush!();
                let rest: String = chars[i..].iter().take(3).collect();
                let op: &'static str = if c == '>' {
                    ["&>>", ">>", ">|", ">&"].into_iter().find(|op| rest.starts_with(op)).unwrap_or(">")
                } else {
                    ["<<<", "<<-", "<<", "<>", "<&"].into_iter().find(|op| rest.starts_with(op)).unwrap_or("<")
                };
                i += op.len();
                tokens.push(Token::Redir(op));
                if op == "<<" || op == "<<-" {
                    // Read the delimiter now so the body can be skipped at the next newline.
                    while chars.get(i).is_some_and(|c| *c == ' ' || *c == '\t') { i += 1; }
                    let mut delimiter = String::new();
                    while let Some(&ch) = chars.get(i) {
                        if ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>' | '(' | ')') { break; }
                        if ch != '\'' && ch != '"' && ch != '\\' { delimiter.push(ch); }
                        i += 1;
                    }
                    tokens.push(Token::Word { text: delimiter.clone(), dynamic: false });
                    heredocs.push((delimiter, op == "<<-"));
                }
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                // A command substitution: its inside is parsed as commands of its own.
                flush!();
                tokens.push(Token::Sep);
                i += 2;
            }
            '$' => { in_word = true; expand(&mut i, &mut word, &mut dynamic); }
            '*' | '?' | '[' | '{' => { in_word = true; dynamic = true; word.push(c); i += 1; }
            '~' if !in_word && matches!(chars.get(i + 1), None | Some('/')) => { in_word = true; word.push_str(&home); i += 1; }
            _ => { in_word = true; word.push(c); i += 1; }
        }
    }
    if in_word {
        tokens.push(Token::Word { text: word, dynamic });
    }
    tokens
}

fn split_commands(tokens: &[Token]) -> Vec<Simple> {
    let mut out = vec![Simple::default()];
    let mut pending: Option<&'static str> = None;
    for token in tokens {
        match token {
            Token::Sep => {
                pending = None;
                out.push(Simple::default());
            }
            Token::Redir(op) => pending = Some(op),
            Token::Word { text, dynamic } => {
                let current = out.last_mut().expect("never empty");
                match pending.take() {
                    Some(op) => current.redirects.push((op, text.clone(), *dynamic)),
                    None => current.words.push((text.clone(), *dynamic)),
                }
            }
        }
    }
    out.retain(|s| !s.words.is_empty() || !s.redirects.is_empty());
    out
}

fn writes_of(simple: &Simple, dir: &mut Option<PathBuf>, out: &mut Vec<ShellWrite>, depth: u8) {
    // `[[ a > b ]]` compares strings; nothing is redirected.
    if simple.words.first().is_some_and(|(w, _)| w == "[[") {
        return;
    }
    let mut push = |word: &(String, bool), effect: Effect, dir: &Option<PathBuf>| {
        if let Some(path) = resolve(word, dir) {
            out.push(ShellWrite { path, effect });
        }
    };
    for (op, target, dynamic) in &simple.redirects {
        let effect = match *op {
            ">" | ">|" | "&>" => Effect::Overwrite,
            ">>" | "&>>" => Effect::Append,
            "<>" => Effect::Modify,
            // `>&2` duplicates a descriptor; `>& file` is bash for `&>`.
            ">&" if !(target == "-" || target.chars().all(|c| c.is_ascii_digit())) => Effect::Overwrite,
            _ => continue,
        };
        push(&(target.clone(), *dynamic), effect, dir);
    }

    // The command proper: skip assignments, wrappers and keywords to reach the program.
    let mut words: &[(String, bool)] = &simple.words;
    loop {
        let Some((first, _)) = words.first() else { return };
        let assignment = first.split_once('=').is_some_and(|(name, _)| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let wrapper = matches!(first.as_str(), "sudo" | "env" | "command" | "builtin" | "exec" | "nohup" | "time" | "nice" | "stdbuf" | "ionice" | "doas"
            | "if" | "then" | "else" | "elif" | "do" | "while" | "until" | "!" | "{" | "}");
        if assignment || wrapper {
            words = &words[1..];
            // Options of a wrapper (`sudo -u x`, `nice -n 5`) are not the program either.
            if wrapper {
                while let Some((option, _)) = words.first().filter(|(w, _)| w.starts_with('-')) {
                    let takes_value = matches!(option.as_str(), "-u" | "-g" | "-C" | "-D" | "-h" | "-p" | "-r" | "-t" | "-U" | "-n" | "-c");
                    words = &words[(if takes_value { 2 } else { 1 }).min(words.len())..];
                }
            }
            continue;
        }
        break;
    }
    let program = Path::new(&words[0].0).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let args = &words[1..];
    let operands = |takes_value: &[&str]| -> Vec<&(String, bool)> {
        let mut v = Vec::new();
        let mut options = true;
        let mut skip = false;
        for arg in args {
            if skip { skip = false; continue; }
            if options && arg.0 == "--" { options = false; continue; }
            if options && arg.0.starts_with('-') && arg.0.len() > 1 {
                skip = takes_value.contains(&arg.0.as_str());
                continue;
            }
            v.push(arg);
        }
        v
    };
    match program.as_str() {
        "cd" => {
            *dir = match args.first() {
                Some(target) => resolve(target, dir),
                None => std::env::var_os("HOME").map(PathBuf::from),
            };
        }
        "tee" => {
            let append = args.iter().any(|(a, _)| a == "-a" || a == "--append" || (a.starts_with('-') && !a.starts_with("--") && a.contains('a')));
            for target in operands(&[]) { push(target, if append { Effect::Append } else { Effect::Overwrite }, dir); }
        }
        "truncate" => for target in operands(&["-s", "-r", "--size", "--reference"]) { push(target, Effect::Overwrite, dir); },
        "rm" | "unlink" | "shred" => for target in operands(&[]) { push(target, Effect::Delete, dir); },
        "sed" | "perl" => {
            // In place when a short option cluster holds `i` (its suffix may follow) or --in-place.
            let mut in_place = false;
            let mut script_given = false;
            let mut files = Vec::new();
            let mut k = 0;
            let mut options = true;
            while k < args.len() {
                let a = &args[k].0;
                if options && a == "--" { options = false; k += 1; continue; }
                if options && a.starts_with("--") {
                    if a.starts_with("--in-place") { in_place = true; }
                    if a == "--expression" || a == "--file" { script_given = true; k += 1; }
                    if a.starts_with("--expression=") || a.starts_with("--file=") { script_given = true; }
                    k += 1;
                    continue;
                }
                if options && a.starts_with('-') && a.len() > 1 {
                    for (n, flag) in a[1..].char_indices() {
                        match flag {
                            'i' => { in_place = true; break; }
                            // perl -MModule, -Idir, -x…: the rest of the cluster is a value.
                            'M' | 'm' | 'I' | 'x' | 'C' | 'd' | 'D' | 'l' | '0' => break,
                            // -e and -f take the rest of the cluster, or the next word.
                            'e' | 'f' => { script_given = true; if n + 2 == a.len() { k += 1; } break; }
                            _ => {}
                        }
                    }
                    k += 1;
                    continue;
                }
                if script_given { files.push(&args[k]); } else { script_given = true; }
                k += 1;
            }
            if in_place {
                for target in files { push(target, Effect::Modify, dir); }
            }
        }
        "cp" | "mv" | "install" => {
            if program == "install" && args.iter().any(|(a, _)| a == "-d" || a == "--directory") { return; }
            let target_dir = args.iter().position(|(a, _)| a == "-t").and_then(|p| args.get(p + 1)).cloned()
                .or_else(|| args.iter().find_map(|(a, d)| a.strip_prefix("--target-directory=").map(|t| (t.to_string(), *d))));
            let named = operands(&["-t", "-S", "-m", "-o", "-g", "--suffix", "--mode", "--owner", "--group"]);
            let (sources, dest): (Vec<&Word>, Option<Word>) = match target_dir {
                Some(t) => (named, Some(t)),
                None if named.len() >= 2 => (named[..named.len() - 1].to_vec(), Some(named[named.len() - 1].clone())),
                None => (Vec::new(), None),
            };
            let Some(dest) = dest else { return };
            let Some(dest_path) = resolve(&dest, dir) else { return };
            for source in &sources {
                if program == "mv" { push(source, Effect::Delete, dir); }
                let into_dir = dest_path.is_dir();
                let target = match Path::new(&source.0).file_name() {
                    Some(name) if into_dir => dest_path.join(name),
                    _ => dest_path.clone(),
                };
                if !(source.1 && into_dir) {
                    out_push(&mut push, target, Effect::Overwrite, dir);
                }
            }
        }
        "dd" => {
            for (a, d) in args {
                if let Some(target) = a.strip_prefix("of=") { push(&(target.to_string(), *d), Effect::Overwrite, dir); }
            }
        }
        "sh" | "bash" | "zsh" | "dash" | "ksh" if depth < 3 => {
            if let Some(p) = args.iter().position(|(a, _)| a == "-c" || (a.starts_with('-') && !a.starts_with("--") && a.ends_with('c'))) {
                if let Some((script, false)) = args.get(p + 1) {
                    let mut inner_dir = dir.clone();
                    for inner in split_commands(&tokenize(script)) {
                        let mut found = Vec::new();
                        writes_of(&inner, &mut inner_dir, &mut found, depth + 1);
                        for w in found { out_push(&mut push, w.path, w.effect, dir); }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Push an already-resolved path through the same closure as words.
fn out_push(push: &mut impl FnMut(&(String, bool), Effect, &Option<PathBuf>), path: PathBuf, effect: Effect, dir: &Option<PathBuf>) {
    push(&(path.to_string_lossy().into_owned(), false), effect, dir);
}

fn resolve((word, dynamic): &(String, bool), dir: &Option<PathBuf>) -> Option<PathBuf> {
    if *dynamic || word.is_empty() || word == "-" {
        return None;
    }
    let path = Path::new(word);
    let path = if path.is_absolute() { path.to_path_buf() } else { dir.as_ref()?.join(path) };
    let path = normalize(&path);
    if path.starts_with("/dev") || path.starts_with("/proc") {
        return None;
    }
    Some(path)
}

/// `a/./b/../c` → `a/c`, without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => { out.pop(); }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writes(command: &str) -> Vec<(String, Effect)> {
        shell_writes(command, Path::new("/w"))
            .into_iter()
            .map(|w| (w.path.display().to_string(), w.effect))
            .collect()
    }

    #[test]
    fn redirections_are_writes_and_descriptor_copies_are_not() {
        use Effect::*;
        assert_eq!(writes("printf 'KEY=leak\\n' >> config/.env"), [("/w/config/.env".into(), Append)]);
        assert_eq!(writes(": > important.rs"), [("/w/important.rs".into(), Overwrite)]);
        assert_eq!(writes("cargo test 2>&1 | tail -5"), []);
        assert_eq!(writes("cargo build > /dev/null 2> err.log"), [("/w/err.log".into(), Overwrite)]);
        assert_eq!(writes("make &> build.log"), [("/w/build.log".into(), Overwrite)]);
        assert_eq!(writes("echo 'a > b' \"c >> d\""), []);
        assert_eq!(writes("cmd >out.txt"), [("/w/out.txt".into(), Overwrite)]);
        assert_eq!(writes("echo x >&2"), []);
        assert_eq!(writes("grep -c x < in.txt"), []);
    }

    #[test]
    fn heredoc_bodies_are_skipped_and_their_target_is_written() {
        let line = "cat > notes.md <<'EOF'\nrm -rf / ; echo > nope\nEOF\necho done >> log.txt";
        assert_eq!(writes(line), [("/w/notes.md".into(), Effect::Overwrite), ("/w/log.txt".into(), Effect::Append)]);
        let tabbed = "cat <<-END > a\n\techo > b\n\tEND\n";
        assert_eq!(writes(tabbed), [("/w/a".into(), Effect::Overwrite)]);
    }

    #[test]
    fn file_writing_programs_name_their_targets() {
        use Effect::*;
        assert_eq!(writes("sed -i 's/.*/x/' src/critical.rs"), [("/w/src/critical.rs".into(), Modify)]);
        assert_eq!(writes("sed -i.bak -e 's/a/b/' a.txt b.txt"), [("/w/a.txt".into(), Modify), ("/w/b.txt".into(), Modify)]);
        assert_eq!(writes("sed -n '1,5p' file.rs"), []);
        assert_eq!(writes("sed -Ei 's/a/b/' x"), [("/w/x".into(), Modify)]);
        assert_eq!(writes("perl -pi -e 's/a/b/' y"), [("/w/y".into(), Modify)]);
        assert_eq!(writes("perl -MFile::Find -e 'print 1' z"), []);
        assert_eq!(writes("[[ a > b ]] && echo yes"), []);
        assert_eq!(writes("echo hi | tee -a one two"), [("/w/one".into(), Append), ("/w/two".into(), Append)]);
        assert_eq!(writes("truncate -s0 f"), [("/w/f".into(), Overwrite)]);
        assert_eq!(writes("truncate -s 0 f"), [("/w/f".into(), Overwrite)]);
        assert_eq!(writes("rm -f a.txt b/c.txt"), [("/w/a.txt".into(), Delete), ("/w/b/c.txt".into(), Delete)]);
        assert_eq!(writes("cp template.json config.json"), [("/w/config.json".into(), Overwrite)]);
        assert_eq!(writes("mv old.rs new.rs"), [("/w/old.rs".into(), Delete), ("/w/new.rs".into(), Overwrite)]);
        assert_eq!(writes("dd if=/dev/zero of=disk.img bs=1M count=1"), [("/w/disk.img".into(), Overwrite)]);
        assert_eq!(writes("install -d out"), []);
    }

    #[test]
    fn wrappers_cd_and_nested_shells_are_followed() {
        use Effect::*;
        assert_eq!(writes("cd sub && echo x > f"), [("/w/sub/f".into(), Overwrite)]);
        assert_eq!(writes("cd /tmp; rm -f y"), [("/tmp/y".into(), Delete)]);
        assert_eq!(writes("cd \"$DIR\" && rm f"), [], "after an unknowable cd, relative targets are unknown");
        assert_eq!(writes("sudo -u me tee /etc/hosts"), [("/etc/hosts".into(), Overwrite)]);
        assert_eq!(writes("FOO=1 env BAR=2 rm z"), [("/w/z".into(), Delete)]);
        assert_eq!(writes("bash -c 'echo a > inner.txt'"), [("/w/inner.txt".into(), Overwrite)]);
        assert_eq!(writes("x=$(cat a > b)"), [("/w/b".into(), Overwrite)]);
        assert_eq!(writes("if true; then rm q; fi"), [("/w/q".into(), Delete)]);
    }

    #[test]
    fn unknowable_targets_are_skipped() {
        assert_eq!(writes("echo x > \"$OUT\""), []);
        assert_eq!(writes("rm *.log"), []);
        assert_eq!(writes("echo x > $(mktemp)"), []);
        let home = std::env::var("HOME").unwrap();
        assert_eq!(writes("echo x > ~/notes"), [(format!("{home}/notes"), Effect::Overwrite)]);
        assert_eq!(writes("echo x > \"$HOME/notes\""), [(format!("{home}/notes"), Effect::Overwrite)]);
        assert_eq!(writes("echo x > ../up/./f"), [("/up/f".into(), Effect::Overwrite)]);
    }

    #[test]
    fn a_patch_names_every_file_it_touches() {
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+one\n+two\n*** Delete File: gone.rs\n*** Update File: src/a.rs\n*** Move to: src/b.rs\n@@ fn main() {\n-    old();\n+    new();\n*** End of File\n*** End Patch\n";
        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops[0], PatchOp::Add { path: "new.txt".into(), text: "one\ntwo\n".into() });
        assert_eq!(ops[1], PatchOp::Delete { path: "gone.rs".into() });
        let PatchOp::Update { path, move_to, chunks } = &ops[2] else { panic!("{ops:?}") };
        assert_eq!((path.as_str(), move_to.as_deref()), ("src/a.rs", Some("src/b.rs")));
        assert_eq!(chunks[0], Chunk { context: Some("fn main() {".into()), old: vec!["    old();".into()], new: vec!["    new();".into()], end_of_file: true });
        assert!(parse_patch("no envelope").is_err());
        assert!(parse_patch("*** Begin Patch\n*** Frobnicate: x\n*** End Patch").is_err());
    }

    #[test]
    fn hunks_apply_in_memory_like_apply_patch() {
        let old = "fn main() {\n    a();\n    b();\n}\n\nfn other() {\n    b();\n}\n";
        let patch = "*** Begin Patch\n*** Update File: x\n@@ fn other() {\n-    b();\n+    c();\n@@\n }\n+// end\n*** End Patch";
        let PatchOp::Update { chunks, .. } = &parse_patch(patch).unwrap()[0] else { panic!() };
        assert_eq!(apply_chunks(old, chunks).unwrap(), "fn main() {\n    a();\n    b();\n}\n\nfn other() {\n    c();\n}\n// end\n");
        // Trailing whitespace differences still match; a hunk that is not there does not.
        let loose = vec![Chunk { old: vec!["    a();   ".into()], new: vec!["    z();".into()], ..Chunk::default() }];
        assert_eq!(apply_chunks(old, &loose).unwrap(), old.replace("    a();", "    z();"));
        let missing = vec![Chunk { old: vec!["nowhere".into()], new: vec![], ..Chunk::default() }];
        assert_eq!(apply_chunks(old, &missing), None);
        // A hunk with no old lines appends.
        let append = vec![Chunk { new: vec!["tail".into()], ..Chunk::default() }];
        assert_eq!(apply_chunks("a\n", &append).unwrap(), "a\ntail\n");
    }
}
