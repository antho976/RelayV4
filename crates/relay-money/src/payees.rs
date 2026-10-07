//! What a bank's description becomes in the ledger, and the category it most likely belongs to.
//! Port of `Payees.kt`.
//!
//! Banks write "ACHAT - METRO PLUS #123 MONTREAL QC"; the ledger wants "Metro Plus Montreal QC"
//! filed under Groceries. All of it is a guess the owner can change, and the import says how many
//! lines it filed itself.
//!
//! Kotlin's `Char` tests (`isLetter`, `isDigit`, `isLetterOrDigit`) are Unicode general
//! categories over UTF-16 units, so a character outside the BMP is never a letter or a digit
//! there; [`is_kt`] reproduces that with the regex crate's category tables.

use crate::bank_statements::normalize;
use crate::csv::{kt_is_blank, kt_trim};
use regex::Regex;
use std::sync::LazyLock;

/// The channel words banks put before the payee, in normalized form, longest first where they overlap.
const PREFIXES: &[&str] = &[
    "paiement facture accesd internet", "paiement facture accesd mobile", "paiement facture accesd",
    "paiement de facture", "paiement facture", "paiement direct", "achat interac", "achat",
    "accesd internet", "accesd mobile", "accesd",
    "interac purchase", "visa debit purchase", "debit card purchase", "pos purchase", "point of sale", "purchase", "pos",
    "online bill payment", "bill payment", "pre authorized debit", "preauthorized debit",
    "prelevement automatique", "prelevement", "depot direct", "direct deposit from", "direct deposit",
    "retrait au ga", "retrait",
];

/// Words kept in capitals when a shouted description is set in title case.
const ACRONYMS: &[&str] = &[
    "IGA", "SAQ", "SAAQ", "STM", "RTC", "STS", "RTL", "TTC", "SQDC", "IKEA", "KFC", "DQ", "PC", "TD", "RBC", "BMO",
    "CIBC", "HSBC", "UPS", "ATM", "USA", "CA", "QC", "ON", "BC", "AB", "MB", "SK", "NB", "NS", "PE", "NL", "YT", "NT",
    "NU", "TFSA", "RRSP", "FHSA", "CELI", "REER", "GST", "TPS", "TVQ", "H&M", "A&W", "TV", "MEC", "SAIL", "CO",
];

// Java's `\d` and `\s` are ASCII.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:[#*xX]{0,4}[0-9]{4,}|#[ \t\n\x0B\x0C\r]?[0-9]+)").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\n\x0B\x0C\r]+").unwrap());
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]+").unwrap());
static LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{L}$").unwrap());
static DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{Nd}$").unwrap());

/// Whether `c` is in the category `class` matches, as Kotlin sees it: a UTF-16 unit, so nothing
/// outside the BMP is.
fn is_kt(class: &Regex, c: char) -> bool {
    (c as u32) < 0x1_0000 && class.is_match(c.encode_utf8(&mut [0; 4]))
}

/// The payee as the ledger shows it: the channel words dropped, card and reference numbers
/// dropped, and a description in all capitals set in title case. Never empty when `raw` is not.
pub fn clean(raw: &str) -> String {
    let mut text = kt_trim(&SPACES.replace_all(raw, " ")).to_string();
    if text.is_empty() {
        return text;
    }
    loop {
        let norm = normalize(&text);
        let Some(prefix) = PREFIXES.iter().find(|p| norm == **p || norm.starts_with(&format!("{p} "))) else { break };
        let rest = drop_words(&text, prefix.split(' ').count()).trim_start_matches([' ', '-', '/', ':', '.']);
        if kt_is_blank(rest) {
            break;
        }
        text = rest.to_string();
    }
    let text = REFERENCE.replace_all(&text, " ");
    let text = SPACES.replace_all(&text, " ");
    let text = kt_trim(&text).trim_matches(['-', '/', ':', '.', ' ']);
    if text.is_empty() {
        return kt_trim(raw).to_string();
    }
    let (mut letters, mut upper) = (0usize, 0usize);
    for c in text.chars().filter(|&c| is_kt(&LETTER, c)) {
        letters += 1;
        upper += usize::from(c.is_uppercase());
    }
    if letters > 0 && upper as f64 >= letters as f64 * 0.8 { title_case(text) } else { text.to_string() }
}

/// `text` with its first `n` words gone, where a word ends at a space, a dash or a slash.
fn drop_words(text: &str, n: usize) -> &str {
    let word = |c: char| is_kt(&LETTER, c) || is_kt(&DIGIT, c);
    let mut rest = text;
    for _ in 0..n {
        if rest.is_empty() {
            break;
        }
        rest = rest.trim_start_matches(|c| !word(c));
        rest = rest.trim_start_matches(word);
    }
    rest
}

/// Kotlin's `Char.titlecase()`: a character whose capital is several letters keeps the first one
/// capital ("ß" reads "Ss"); the four Latin digraphs have a title form of their own, and
/// Georgian has no capital in title case.
fn titlecase(c: char) -> String {
    let upper: Vec<char> = c.to_uppercase().collect();
    if upper.len() > 1 {
        if c == '\u{149}' {
            return upper.into_iter().collect();
        }
        let rest: String = upper[1..].iter().collect();
        return format!("{}{}", upper[0], rest.to_lowercase());
    }
    match c {
        '\u{1C4}'..='\u{1C6}' => '\u{1C5}',
        '\u{1C7}'..='\u{1C9}' => '\u{1C8}',
        '\u{1CA}'..='\u{1CC}' => '\u{1CB}',
        '\u{1F1}'..='\u{1F3}' => '\u{1F2}',
        '\u{10D0}'..='\u{10FA}' | '\u{10FD}'..='\u{10FF}' => c,
        _ => upper[0],
    }
    .to_string()
}

fn title_case(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let up = word.to_uppercase();
            if ACRONYMS.contains(&up.as_str()) {
                up
            } else if word.chars().any(|c| is_kt(&DIGIT, c)) {
                word.to_string()
            } else {
                word.to_lowercase()
                    .split('-')
                    .map(|part| {
                        let mut cs = part.chars();
                        match cs.next() {
                            // Kotlin's `replaceFirstChar` works on a UTF-16 unit: half a pair stays as it is.
                            Some(f) if (f as u32) < 0x1_0000 => titlecase(f) + cs.as_str(),
                            _ => part.to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("-")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The words that name a payee for learning from the ledger: "Metro Plus Montreal QC" and
/// "METRO PLUS #4411" both key as "metro plus". Empty when nothing is left.
pub fn key(note: &str) -> String {
    DIGITS.replace_all(&normalize(note), " ").split(' ').filter(|w| w.len() > 1).take(2).collect::<Vec<_>>().join(" ")
}

/// Moves between the owner's own accounts: card payments, transfers to savings. Never an Interac e-Transfer to a person.
const TRANSFER_WORDS: &[&str] = &[
    "virement", "transfer", "transfert", "paiement carte", "paiement visa", "paiement mastercard", "paiement de carte",
    "carte de credit", "credit card payment", "card payment", "payment thank you", "payment received", "paiement recu",
    "merci pour votre paiement", "paiement merci", "contribution", "cotisation",
];
const TRANSFER_CODES: &[&str] = &["TRFIN", "TRFOUT", "TRFINTF", "TRF_IN", "TRF_OUT", "TRANSFER", "INTERNAL_TRANSFER"];

pub fn looks_like_transfer(description: &str, code: Option<&str>) -> bool {
    if code.is_some_and(|c| TRANSFER_CODES.contains(&c.to_uppercase().as_str())) {
        return true;
    }
    let n = format!(" {} ", normalize(description));
    if n.contains("interac") || n.contains(" e transfer") || n.contains(" etransfer") {
        return false;
    }
    TRANSFER_WORDS.iter().any(|w| n.contains(&format!(" {w}")))
}

/// The default category a description most likely belongs to, by name ("Groceries"), or `None`.
/// Keys are matched at the start of a word; one ending in a space must be the whole word, so
/// "metro " never files a "Metropolis" ticket under Groceries. Order matters: subscriptions
/// before shopping (Amazon Prime), dining before transport (Uber Eats).
pub fn guess_category(description: &str, income: bool, code: Option<&str>) -> Option<&'static str> {
    let n = format!(" {} ", normalize(description));
    let first = |rules: &[(&'static str, &[&str])]| {
        rules.iter().find(|(_, keys)| keys.iter().any(|k| n.contains(&format!(" {k}")))).map(|(name, _)| *name)
    };
    if income {
        if code.is_some_and(|c| c.to_uppercase() == "INT") {
            return Some("Other income");
        }
        return first(INCOME_RULES);
    }
    first(SPENDING_RULES)
}

const SPENDING_RULES: &[(&str, &[&str])] = &[
    ("Subscriptions", &[
        "netflix", "spotify", "disney plus", "disneyplus", "crave", "prime video", "primevideo", "amazon prime",
        "apple com bill", "apple music", "icloud", "google one", "google storage", "youtube premium", "youtubepremium",
        "patreon", "adobe", "microsoft 365", "dropbox", "chatgpt", "openai", "anthropic", "audible", "paramount",
        "club illico", "illico", "tou tv", "deezer", "playstation plus", "xbox game pass",
    ]),
    ("Dining", &[
        "restaurant", "resto ", "mcdonald", "tim hortons", "tims ", "starbucks", "second cup", "subway ", "pizza", "sushi",
        "uber eats", "ubereats", "doordash", "skipthedishes", "skip the dishes", "cafe ", "bistro", "brasserie",
        "poulet rouge", "st hubert", "benny", "la cage", "pret a manger", "domino", "burger", "bagel", "shawarma",
        "poke", "pho ", "thai ", "kfc ", "popeyes", "harvey", "mary brown", "dairy queen", "dq ", "boston pizza",
        "cora ", "chocolato", "poutine", "valentine", "booster juice", "freshii", "chipotle", "five guys",
        "la belle province", "dic ann", "a w ", "wendy", "taco",
    ]),
    ("Groceries", &[
        "metro ", "iga ", "provigo", "maxi ", "super c ", "loblaws", "sobeys", "adonis", "marche ", "epicerie",
        "fruiterie", "boucherie", "poissonnerie", "boulangerie", "no frills", "food basics", "freshco", "foodland",
        "voila ", "avril ", "rachelle bery", "intermarche", "costco", "mayrand", "kim phat", "whole foods",
        "farm boy", "longo", "save on foods", "safeway", "superstore", "t t supermarket", "segal",
    ]),
    ("Transport", &[
        "stm ", "opus", "exo ", "rtc ", "sts ", "rtl ", "ttc ", "translink", "presto", "uber ", "lyft", "taxi",
        "communauto", "bixi", "via rail", "viarail", "petro canada", "petrocan", "shell ", "esso ", "ultramar",
        "couche tard", "irving", "pioneer", "husky", "sunoco", "parking", "stationnement", "indigo park", "saaq ",
        "car wash", "lave auto", "amigo express", "orleans express", "mobilite",
    ]),
    ("Housing", &[
        "loyer", "rent ", "hypotheque", "mortgage", "frais de condo", "condo fees", "assurance habitation",
        "home insurance", "taxes municipales", "property tax",
    ]),
    ("Utilities", &["hydro quebec", "hydro ", "energir", "gaz metro", "enbridge", "fortis", "epcor", "bc hydro", "water "]),
    ("Phone & internet", &[
        "videotron", "bell ", "bell canada", "rogers", "fido", "telus", "koodo", "fizz", "virgin plus", "virgin mobile",
        "public mobile", "freedom mobile", "cogeco", "ebox", "oxio", "distributel", "teksavvy", "chatr", "lucky mobile",
    ]),
    ("Health", &[
        "pharmaprix", "jean coutu", "uniprix", "familiprix", "shoppers drug", "proxim", "brunet", "pharmacie", "pharmacy",
        "clinique", "clinic", "dentist", "dentaire", "dental", "optometr", "lunetterie", "physio", "massotherap", "chiropr",
        "energie cardio", "econofitness", "nautilus plus", "goodlife", "gym ", "yoga", "crossfit", "psycholog",
    ]),
    ("Entertainment", &[
        "cineplex", "cinema", "steam ", "steampowered", "playstation", "xbox", "nintendo", "ticketmaster", "spectra",
        "evenko", "bar ", "pub ", "billard", "bowling", "quilles", "musee", "museum", "theatre", "concert",
        "loto quebec", "festival",
    ]),
    ("Travel", &[
        "air canada", "westjet", "porter air", "air transat", "sunwing", "flair air", "expedia", "booking com",
        "airbnb", "hotel", "marriott", "hilton", "hyatt", "best western", "holiday inn", "aeroport", "airport",
    ]),
    ("Shopping", &[
        "amazon", "amzn", "walmart", "canadian tire", "winners", "simons", "la baie", "hudson s bay", "best buy",
        "ikea", "dollarama", "home depot", "rona ", "reno depot", "structube", "apple store", "aliexpress", "shein",
        "etsy", "ebay", "sports experts", "sport expert", "decathlon", "uniqlo", "h m ", "zara", "old navy", "gap ",
        "indigo", "renaud bray", "archambault", "bureau en gros", "staples", "jysk", "giant tiger", "ardene", "aldo",
        "la vie en rose", "atmosphere", "value village", "village des valeurs", "dollar tree", "michaels", "lego",
    ]),
];

const INCOME_RULES: &[(&str, &[&str])] = &[
    ("Refunds", &["refund", "remboursement", "retour", "return", "reversal", "annulation"]),
    ("Salary", &["paie ", "payroll", "salaire", "pay ", "depot de paie", "employeur"]),
    ("Other income", &[
        "interet", "interest", "dividend", "dividende", "cashback", "remise", "gouv", "gouvernement", "canada fed",
        "gst ", "tps ", "ccb ", "allocation",
    ]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_words_and_references_go_shouting_is_set_in_title_case() {
        assert_eq!("Metro Plus Montreal QC", clean("ACHAT - METRO PLUS #123 MONTREAL QC"));
        assert_eq!("Hydro-Quebec", clean("Paiement facture - AccèsD Internet /HYDRO-QUEBEC"));
        assert_eq!("IGA Extra", clean("Paiement direct - IGA EXTRA 0045123"));
        assert_eq!("Spotify", clean("POS Purchase SPOTIFY ****4821"));
    }

    #[test]
    fn a_description_that_is_all_channel_words_keeps_its_text() {
        assert_eq!("Retrait", clean("Retrait"));
        assert_eq!("Café Olimpico", clean("  Café   Olimpico "));
    }

    #[test]
    fn keys_group_the_same_payee_across_stores() {
        assert_eq!("metro plus", key("Metro Plus Montreal QC"));
        assert_eq!("metro plus", key("METRO PLUS #4411"));
        assert_eq!("", key("12345"));
    }

    #[test]
    fn known_merchants_file_themselves() {
        assert_eq!(Some("Groceries"), guess_category("METRO PLUS MONTREAL", false, None));
        assert_eq!(Some("Dining"), guess_category("Uber Eats", false, None));
        assert_eq!(Some("Transport"), guess_category("UBER TRIP", false, None));
        assert_eq!(Some("Subscriptions"), guess_category("Amazon Prime Video", false, None));
        assert_eq!(Some("Shopping"), guess_category("AMZN Mktp CA", false, None));
        assert_eq!(Some("Utilities"), guess_category("Hydro-Québec", false, None));
        assert_eq!(Some("Phone & internet"), guess_category("VIDEOTRON LTEE", false, None));
        assert_eq!(Some("Health"), guess_category("Jean Coutu #12", false, None));
        assert_eq!(Some("Salary"), guess_category("Dépôt de paie", true, None));
        assert_eq!(Some("Other income"), guess_category("Interest", true, Some("INT")));
    }

    #[test]
    fn a_whole_word_key_never_matches_inside_a_longer_word() {
        assert_eq!(None, guess_category("Metropolis Books", false, None));
        assert_eq!(None, guess_category("Paiement reçu", true, None));
    }

    #[test]
    fn card_payments_and_transfers_are_moves_an_e_transfer_to_a_person_is_not() {
        assert!(looks_like_transfer("Paiement carte VISA", None));
        assert!(looks_like_transfer("Transfer out to Chequing", None));
        assert!(looks_like_transfer("Virement entre comptes", None));
        assert!(looks_like_transfer("anything", Some("TRFOUT")));
        assert!(!looks_like_transfer("Interac e-Transfer to Sam", None));
        assert!(!looks_like_transfer("Virement Interac reçu", None));
        assert!(!looks_like_transfer("Metro", None));
    }

    #[test]
    fn title_case_follows_kotlin_titlecase() {
        assert_eq!("Strasse", title_case("STRASSE"));
        assert_eq!("Ssüd", title_case("ßÜD"));
        assert_eq!("ǅemal", title_case("ǄEMAL"));
    }
}
