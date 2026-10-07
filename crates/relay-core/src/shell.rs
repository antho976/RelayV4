//! The one shell reader Relay uses to look inside an agent's command line: the denied-command
//! guardrail, the self-approval check and the device-lease gate all read a line through here, so
//! they agree on where one command ends and the next begins.
//!
//! Deliberately small, and not a shell. It knows enough to tell an argument from quoted data
//! (D103) and to find every command a line runs: the ones joined by `;`, `&&`, `|` and newlines,
//! and the ones nested in `( )`, `$( )`, backticks and `sh -c '…'` / `eval`. A guardrail reader
//! that misses `(git reset --hard)` is a seatbelt that only works when nobody needs it.

use std::path::Path;

/// One shell word plus whether *every* character of it came from inside quotes. A partly
/// quoted word counts as unquoted: when in doubt, still inspect it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    pub text: String,
    pub quoted: bool,
}

/// How deep `$( )` inside double quotes, `sh -c` and `eval` are followed. Real command lines
/// nest one or two levels; the bound only stops a hostile line from recursing without end.
const MAX_DEPTH: usize = 8;

/// Shells whose `-c` argument is itself a command line.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish"];

/// Every simple command `line` runs, each as its words. Commands nested in substitutions,
/// subshells and `sh -c` come out as commands of their own, after the one that holds them.
pub(crate) fn commands(line: &str) -> Vec<Vec<Word>> {
    read(line, 0)
}

/// The basename of a program word: `/usr/bin/rm` and `rm` are the same program.
pub(crate) fn program(word: &str) -> &str {
    Path::new(word).file_name().and_then(|name| name.to_str()).unwrap_or(word)
}

fn read(line: &str, depth: usize) -> Vec<Vec<Word>> {
    let mut reader = Reader::default();
    let chars: Vec<char> = line.chars().collect();
    reader.run(&chars, depth);
    let mut out = Vec::new();
    for command in reader.commands {
        let nested = if depth < MAX_DEPTH { script_of(&command) } else { None };
        out.push(command);
        if let Some(script) = nested {
            out.extend(read(&script, depth + 1));
        }
    }
    out.extend(reader.nested);
    out
}

/// The command line a `sh -c '<script>'` or `eval <words>` runs, if `command` is one.
fn script_of(command: &[Word]) -> Option<String> {
    let at = command.iter().position(|word| {
        let name = program(&word.text);
        SHELLS.contains(&name) || name == "eval"
    })?;
    if program(&command[at].text) == "eval" {
        let rest: Vec<&str> = command[at + 1..].iter().map(|word| word.text.as_str()).collect();
        return (!rest.is_empty()).then(|| rest.join(" "));
    }
    // `-c`, or a cluster that carries it: `bash -lc '…'`, `sh -ec '…'`.
    let flag = command[at + 1..].iter().position(|word| {
        word.text.starts_with('-') && !word.text.starts_with("--") && word.text.contains('c')
    })?;
    command.get(at + 1 + flag + 1).map(|word| word.text.clone())
}

#[derive(Default)]
struct Reader {
    commands: Vec<Vec<Word>>,
    /// Commands found inside double-quoted `$( )` and backticks, read recursively.
    nested: Vec<Vec<Word>>,
    words: Vec<Word>,
    text: String,
    any_bare: bool,
    started: bool,
}

impl Reader {
    fn end_word(&mut self) {
        if self.started {
            let text = std::mem::take(&mut self.text);
            self.words.push(Word { text, quoted: !self.any_bare });
            self.any_bare = false;
            self.started = false;
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        if !self.words.is_empty() {
            self.commands.push(std::mem::take(&mut self.words));
        }
    }

    fn bare(&mut self, c: char) {
        self.started = true;
        self.any_bare = true;
        self.text.push(c);
    }

    fn run(&mut self, chars: &[char], depth: usize) {
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            match c {
                '\'' => {
                    // Single quotes: nothing inside is special, backslashes included.
                    self.started = true;
                    while i < chars.len() && chars[i] != '\'' {
                        self.text.push(chars[i]);
                        i += 1;
                    }
                    i += 1;
                }
                '"' => {
                    self.started = true;
                    i = self.double_quoted(chars, i, depth);
                }
                '\\' => match chars.get(i) {
                    // A line continuation joins two lines into one word stream.
                    Some('\n') => i += 1,
                    Some(&escaped) => {
                        self.bare(escaped);
                        i += 1;
                    }
                    None => {}
                },
                // `&>file` and `2>&1` are redirections, not a background `&`.
                '&' if chars.get(i) == Some(&'>') || self.text.ends_with('>') => self.bare(c),
                ';' | '\n' | '|' | '&' => {
                    // `&&` / `||` are two characters for one separator; a single one separates too.
                    if chars.get(i) == Some(&c) {
                        i += 1;
                    }
                    self.end_command();
                }
                // A subshell, a command substitution or a backtick starts (or ends) a command
                // of its own: `(git reset --hard)` runs `git reset --hard`.
                '(' | ')' | '`' => self.end_command(),
                '$' if chars.get(i) == Some(&'(') => {
                    i += 1;
                    self.end_command();
                }
                c if c.is_whitespace() => self.end_word(),
                c => self.bare(c),
            }
        }
        self.end_command();
    }

    /// Read a double-quoted span starting just after its opening quote; return the index just
    /// past the closing quote. The span is data, but a `$( )` or backtick inside it still runs.
    fn double_quoted(&mut self, chars: &[char], mut i: usize, depth: usize) -> usize {
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            match c {
                '"' => return i,
                // POSIX: inside double quotes a backslash escapes only these.
                '\\' => match chars.get(i) {
                    Some('\n') => i += 1,
                    Some(&escaped @ ('$' | '`' | '"' | '\\')) => {
                        self.text.push(escaped);
                        i += 1;
                    }
                    _ => self.text.push('\\'),
                },
                '$' if chars.get(i) == Some(&'(') => {
                    let start = i + 1;
                    let end = closing_paren(chars, start);
                    self.text.extend(&chars[i - 1..end.min(chars.len())]);
                    self.substitution(&chars[start..end.min(chars.len())], depth);
                    i = (end + 1).min(chars.len());
                }
                '`' => {
                    let start = i;
                    let mut end = start;
                    while end < chars.len() && chars[end] != '`' {
                        end += if chars[end] == '\\' { 2 } else { 1 };
                    }
                    let end = end.min(chars.len());
                    self.text.extend(&chars[i - 1..(end + 1).min(chars.len())]);
                    self.substitution(&chars[start..end], depth);
                    i = (end + 1).min(chars.len());
                }
                c => self.text.push(c),
            }
        }
        i
    }

    fn substitution(&mut self, inner: &[char], depth: usize) {
        if depth < MAX_DEPTH {
            let inner: String = inner.iter().collect();
            self.nested.extend(read(&inner, depth + 1));
        }
    }
}

/// The index of the `)` that closes a `$(` whose body starts at `start`, skipping quoted text.
fn closing_paren(chars: &[char], start: usize) -> usize {
    let mut depth = 1;
    let mut quote: Option<char> = None;
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => i += 1,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') => i += 1,
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

#[cfg(test)]
mod tests {
    use super::commands;

    fn texts(line: &str) -> Vec<Vec<String>> {
        commands(line)
            .into_iter()
            .map(|command| command.into_iter().map(|word| word.text).collect())
            .collect()
    }

    fn has(line: &str, wanted: &[&str]) -> bool {
        texts(line).iter().any(|command| command == wanted)
    }

    #[test]
    fn subshells_and_substitutions_are_commands_of_their_own() {
        assert!(has("(git reset --hard)", &["git", "reset", "--hard"]));
        assert!(has("cd sub && (git clean -fd)", &["git", "clean", "-fd"]));
        assert!(has("echo $(rm -rf build)", &["rm", "-rf", "build"]));
        assert!(has("echo `rm -rf build`", &["rm", "-rf", "build"]));
        assert!(has("echo \"now: $(rm -rf build)\"", &["rm", "-rf", "build"]));
        assert!(has("echo \"now: `rm -rf build`\"", &["rm", "-rf", "build"]));
        assert!(has("x=$(echo \"$(rm -rf b)\")", &["rm", "-rf", "b"]));
    }

    #[test]
    fn shells_and_eval_run_their_argument() {
        assert!(has("sh -c 'rm -rf build'", &["rm", "-rf", "build"]));
        assert!(has("bash -lc \"git reset --hard\"", &["git", "reset", "--hard"]));
        assert!(has("eval 'git push --force'", &["git", "push", "--force"]));
    }

    #[test]
    fn quoting_follows_posix() {
        // An escaped quote inside double quotes does not close them, so what follows is
        // still read as commands rather than swallowed into a string.
        assert!(has(r#"echo "\"" ; zap --all ; echo "\"""#, &["zap", "--all"]));
        assert_eq!(texts(r#"echo "a\nb""#), vec![vec!["echo", r"a\nb"]]);
        assert_eq!(texts(r"echo 'a\'"), vec![vec!["echo", r"a\"]]);
        let quoted = commands("echo \"rm -rf /\"");
        assert_eq!(quoted.len(), 1, "quoted data is not a command");
        assert!(quoted[0][1].quoted);
        assert!(!commands("\"rm\"x")[0][0].quoted, "a partly quoted word counts as bare");
    }

    #[test]
    fn redirections_are_not_separators() {
        assert_eq!(texts("make &> log"), vec![vec!["make", "&>", "log"]]);
        assert_eq!(texts("make 2>&1"), vec![vec!["make", "2>&1"]]);
        assert_eq!(texts("a && b || c; d | e & f").len(), 6);
    }
}
