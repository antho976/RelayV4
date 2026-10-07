//! Case-insensitive matching that SQLite can run for every alphabet. Port of `TextMatch.kt`.
//!
//! SQLite's LIKE and COLLATE NOCASE fold case for ASCII only, so a search for "épicerie" never
//! finds "Épicerie". GLOB is case-sensitive but takes character sets, so each letter of the query
//! becomes the set of its case forms ("[éÉ]") and the match still runs in SQL over every row, not
//! over a capped list. GLOB's own wildcards in the query are escaped, so "50%" or "a*b" match as
//! typed.
//!
//! The case forms are the JDK's `Character.toLowerCase`/`toUpperCase`/`toTitleCase` on a UTF-16
//! unit: single-character (simple) mappings, never the multi-character ones `char::to_uppercase`
//! can give ("ß" stays "ß", not "SS"), and none at all for characters outside the BMP, which
//! Kotlin walks as two surrogate halves that have no case.

/// A GLOB pattern for text that contains `query` in any case; empty when `query` is blank.
pub fn contains_pattern(query: &str) -> String {
    let q = query.trim_matches(kotlin_whitespace);
    if q.is_empty() { String::new() } else { format!("*{}*", caseless(q)) }
}

/// A GLOB pattern for text that starts with `prefix` in any case; empty when `prefix` is blank.
pub fn prefix_pattern(prefix: &str) -> String {
    let p = prefix.trim_matches(kotlin_whitespace);
    if p.is_empty() { String::new() } else { format!("{}*", caseless(p)) }
}

/// Kotlin's `Char.isWhitespace`, which `String.trim` uses: the space separators (no-break spaces
/// included), the line and paragraph separators, the ASCII controls 9..13 and 28..31. Not U+0085.
fn kotlin_whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || ('\u{1C}'..='\u{1F}').contains(&c)
}

fn caseless(text: &str) -> String {
    let mut sb = String::new();
    for c in text.chars() {
        let mut forms: Vec<char> = Vec::with_capacity(4);
        for f in [c, lowercase_char(c), uppercase_char(c), titlecase_char(c)] {
            if !forms.contains(&f) {
                forms.push(f);
            }
        }
        if forms.len() > 1 {
            sb.push('[');
            sb.extend(forms);
            sb.push(']');
        } else if matches!(c, '*' | '?' | '[') {
            // The three characters GLOB reads as syntax outside a set; inside one they are plain.
            sb.push('[');
            sb.push(c);
            sb.push(']');
        } else {
            sb.push(c);
        }
    }
    sb
}

/// The one character a full mapping gives, or `None` when it gives several.
fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

fn bmp(c: char) -> bool {
    (c as u32) <= 0xFFFF
}

/// Greek small letters with ypogegrammeni, whose full uppercase is two letters ("ᾳ" to "ΑΙ") but
/// whose simple uppercase and titlecase is the one capital with prosgegrammeni ("ᾼ").
fn greek_iota_capital(c: char) -> Option<char> {
    let u = c as u32;
    let mapped = match u {
        0x1F80..=0x1F87 | 0x1F90..=0x1F97 | 0x1FA0..=0x1FA7 => u + 8,
        0x1FB3 | 0x1FC3 | 0x1FF3 => u + 9,
        _ => return None,
    };
    char::from_u32(mapped)
}

/// `Character.toLowerCase`: the full mapping when it is one character. The only multi-character
/// full lowercase is "İ" (an i and a combining dot), whose simple mapping is the plain "i".
fn lowercase_char(c: char) -> char {
    if !bmp(c) {
        return c;
    }
    if c == '\u{130}' {
        return 'i';
    }
    single(c.to_lowercase()).unwrap_or(c)
}

/// `Character.toUpperCase`: the full mapping when it is one character, else the simple one.
fn uppercase_char(c: char) -> char {
    if !bmp(c) {
        return c;
    }
    single(c.to_uppercase()).or_else(|| greek_iota_capital(c)).unwrap_or(c)
}

/// `Character.toTitleCase`: the uppercase, except for the Latin digraphs ("ǆ" to "ǅ"), the Greek
/// capitals with prosgegrammeni that are already titlecase, and Georgian Mkhedruli, whose title
/// form is itself.
fn titlecase_char(c: char) -> char {
    if !bmp(c) {
        return c;
    }
    match c as u32 {
        0x1C4..=0x1C6 => '\u{1C5}',
        0x1C7..=0x1C9 => '\u{1C8}',
        0x1CA..=0x1CC => '\u{1CB}',
        0x1F1..=0x1F3 => '\u{1F2}',
        0x1F88..=0x1F8F | 0x1F98..=0x1F9F | 0x1FA8..=0x1FAF | 0x1FBC | 0x1FCC | 0x1FFC => c,
        0x10D0..=0x10FA | 0x10FD..=0x10FF => c,
        _ => greek_iota_capital(c).unwrap_or_else(|| uppercase_char(c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_letter_becomes_the_set_of_its_case_forms() {
        assert_eq!(contains_pattern("metro"), "*[mM][eE][tT][rR][oO]*");
        assert_eq!(contains_pattern("épi"), "*[éÉ][pP][iI]*");
        assert_eq!(contains_pattern("École"), "*[Éé][cC][oO][lL][eE]*");
    }

    #[test]
    fn digits_spaces_and_marks_stay_as_they_are() {
        assert_eq!(contains_pattern("55 st"), "*55 [sS][tT]*");
        assert_eq!(contains_pattern("%"), "*%*");
    }

    #[test]
    fn glob_syntax_in_the_query_is_matched_literally() {
        assert_eq!(contains_pattern("*"), "*[*]*");
        assert_eq!(contains_pattern("?"), "*[?]*");
        assert_eq!(contains_pattern("["), "*[[]*");
        assert_eq!(contains_pattern("]"), "*]*");
    }

    #[test]
    fn a_blank_query_is_the_empty_pattern_the_queries_skip() {
        assert_eq!(contains_pattern("   "), "");
        assert_eq!(prefix_pattern(""), "");
    }

    #[test]
    fn a_prefix_pattern_anchors_at_the_start_and_trims() {
        assert_eq!(prefix_pattern(" Me "), "[Mm][eE]*");
    }

    #[test]
    fn case_forms_are_single_characters_as_the_jdk_maps_them() {
        assert_eq!(contains_pattern("ß"), "*ß*", "no simple uppercase, never SS");
        assert_eq!(contains_pattern("İ"), "*[İi]*");
        assert_eq!(contains_pattern("ǆ"), "*[ǆǄǅ]*");
        assert_eq!(contains_pattern("ᾳ"), "*[ᾳᾼ]*");
        assert_eq!(contains_pattern("𐐨"), "*𐐨*", "outside the BMP Kotlin sees two caseless halves");
    }
}
