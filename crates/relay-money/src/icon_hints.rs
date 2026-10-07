//! The icon a new category's name suggests. Port of `IconHints.kt`.
//!
//! "Coffee" takes the cup, "Épicerie" the cart, "Loyer" the house. Words are matched at the start
//! of a word of the name, in English and French, so the owner can still pick any other glyph.
//! Every key returned is one of [`crate::model::ICON_KEYS`].

use crate::bank_statements::normalize;

const HINTS: &[(&str, &[&str])] = &[
    // Before the food words, so "Dog food" is the paw and not the knife and fork.
    ("pets", &["pet", "dog", "chien", "cat ", "chat ", "veterinar", "animal"]),
    ("coffee", &["coffee", "cafe", "espresso"]),
    ("tea", &["tea", "the "]),
    ("pizza", &["pizza"]),
    ("fastfood", &["fast food", "takeout", "take out", "delivery food", "livraison"]),
    ("lunch", &["lunch", "dinner", "diner", "souper", "burger"]),
    ("bakery", &["bakery", "boulangerie", "patisserie", "pastry"]),
    ("beer", &["beer", "biere"]),
    ("wine", &["wine", "vin "]),
    ("liquor", &["alcohol", "alcool", "liquor", "saq"]),
    ("bar", &["bar ", "bars", "drinks", "cocktail"]),
    ("dining", &["dining", "restaurant", "resto", "food", "bouffe", "eating out"]),
    ("cart", &["grocer", "epicerie", "supermarket", "marche"]),
    ("fuel", &["gas", "fuel", "essence"]),
    ("parking", &["parking", "stationnement"]),
    ("ev", &["charging", "recharge"]),
    ("carrepair", &["car repair", "mechanic", "garage", "entretien auto"]),
    ("car", &["car", "auto", "voiture", "vehicle"]),
    ("taxi", &["taxi", "uber", "lyft", "rides"]),
    ("bike", &["bike", "velo", "bixi"]),
    ("train", &["train", "via rail"]),
    ("subway", &["metro", "subway"]),
    ("transport", &["transport", "transit", "bus", "commute"]),
    ("flight", &["flight", "vol ", "vols", "avion", "airline"]),
    ("hotel", &["hotel", "lodging", "hebergement"]),
    ("luggage", &["travel", "voyage", "vacation", "vacances", "trip"]),
    ("home", &["rent", "loyer", "housing", "logement", "mortgage", "hypotheque"]),
    ("apartment", &["condo", "apartment", "appartement"]),
    ("bolt", &["electric", "hydro", "electricite", "power", "utilities"]),
    ("water", &["water", "eau"]),
    ("heat", &["heating", "chauffage"]),
    ("phone", &["phone", "cell", "mobile", "telephone"]),
    ("wifi", &["internet", "wifi"]),
    ("tv", &["tv", "television", "cable"]),
    ("insurance", &["insurance", "assurance"]),
    ("furniture", &["furniture", "meuble"]),
    ("repairs", &["repair", "reparation", "renovation", "tools", "outils"]),
    ("cleaning", &["cleaning", "menage", "nettoyage"]),
    ("laundry", &["laundry", "lavage"]),
    ("garden", &["garden", "jardin", "lawn"]),
    ("clothes", &["clothes", "clothing", "vetement", "apparel", "linge"]),
    ("computer", &["computer", "ordinateur", "tech", "electronic", "electronique"]),
    ("headphones", &["audio", "headphone"]),
    ("toys", &["toy", "jouet"]),
    ("flowers", &["flower", "fleur", "plants", "plantes"]),
    ("jewelry", &["jewel", "bijou"]),
    ("bag", &["shopping", "magasinage", "achats"]),
    ("medication", &["medication", "medicament", "prescription"]),
    ("pharmacy", &["pharmac"]),
    ("hospital", &["hospital", "hopital"]),
    ("medical", &["medical", "doctor", "medecin", "dentist", "dentiste", "clinic", "clinique"]),
    ("eyes", &["glasses", "lunettes", "optometr", "eye"]),
    ("therapy", &["therapy", "therapie", "psych"]),
    ("haircut", &["hair", "coiffure", "barber", "barbier"]),
    ("beauty", &["beauty", "beaute", "spa", "cosmetic", "nails"]),
    ("wellness", &["wellness", "bien etre", "meditation", "yoga"]),
    ("sport", &["gym", "fitness", "sport", "entrainement", "workout"]),
    ("health", &["health", "sante"]),
    ("movie", &["movie", "cinema", "film"]),
    ("games", &["game", "jeu", "jeux", "gaming"]),
    ("music", &["music", "musique", "concert"]),
    ("party", &["party", "fete", "celebration"]),
    ("theatre", &["theatre", "theater", "show", "spectacle"]),
    ("art", &["art", "craft", "hobby", "loisir"]),
    ("outdoors", &["outdoor", "plein air", "camping", "park"]),
    ("hiking", &["hiking", "randonnee"]),
    ("pool", &["pool", "piscine", "swim"]),
    ("casino", &["casino", "lottery", "loterie", "loto"]),
    ("ticket", &["entertainment", "divertissement", "ticket", "billet", "events"]),
    ("baby", &["baby", "bebe", "diaper", "couche"]),
    ("child", &["kids", "child", "enfant", "daycare", "garderie"]),
    ("school", &["school", "ecole", "education", "tuition", "course", "cours", "university", "universite"]),
    ("book", &["book", "livre", "reading", "lecture"]),
    ("family", &["family", "famille"]),
    ("elderly", &["parents", "elderly"]),
    ("charity", &["charity", "donation", "don ", "dons", "organisme"]),
    ("church", &["church", "eglise", "dime", "tithe"]),
    ("birthday", &["birthday", "anniversaire"]),
    ("gift", &["gift", "cadeau", "present"]),
    ("repeat", &["subscription", "abonnement", "streaming"]),
    ("invest", &["invest", "placement", "tfsa", "celi", "rrsp", "reer", "fhsa", "stocks", "etf"]),
    ("crypto", &["crypto", "bitcoin"]),
    ("savings", &["saving", "epargne", "emergency", "urgence"]),
    ("tax", &["tax", "impot", "taxes"]),
    ("loan", &["loan", "pret ", "debt", "dette"]),
    ("card", &["credit card", "carte de credit", "interest charge"]),
    ("fees", &["fee", "frais", "bank charge"]),
    ("legal", &["legal", "lawyer", "avocat", "notaire", "notary"]),
    ("work", &["salary", "salaire", "paycheque", "paycheck", "job", "work", "travail"]),
    ("business", &["business", "entreprise", "freelance", "contract", "contrat"]),
    ("spark", &["bonus", "side", "extra", "prime "]),
    ("refund", &["refund", "remboursement", "rebate"]),
    ("sale", &["sold", "vente", "resale", "marketplace"]),
    ("cash", &["cash", "comptant", "atm", "allowance", "allocation"]),
    ("delivery", &["shipping", "postage", "courier"]),
    ("mail", &["mail", "poste"]),
    ("online", &["online", "en ligne", "domain", "hosting"]),
    ("cloud", &["cloud", "storage", "software", "logiciel", "apps"]),
];

/// The icon `name` suggests, or `None` when no word of it is known.
pub fn suggest(name: &str) -> Option<&'static str> {
    let n = format!(" {} ", normalize(name));
    if n.trim().is_empty() {
        return None;
    }
    HINTS.iter().find(|(_, words)| words.iter().any(|w| n.contains(&format!(" {w}")))).map(|(key, _)| *key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ICON_KEYS;

    #[test]
    fn names_suggest_their_glyph_in_english_and_french() {
        assert_eq!(suggest("Coffee"), Some("coffee"));
        assert_eq!(suggest("Café"), Some("coffee"));
        assert_eq!(suggest("Épicerie"), Some("cart"));
        assert_eq!(suggest("Loyer"), Some("home"));
        assert_eq!(suggest("Dog food"), Some("pets"));
        assert_eq!(suggest("TFSA"), Some("invest"));
        assert_eq!(suggest("Zzyzx"), None);
        assert_eq!(suggest(""), None);
    }

    #[test]
    fn every_suggestion_is_a_key_the_app_can_draw() {
        let words = [
            "coffee", "tea", "pizza", "takeout", "lunch", "bakery", "beer", "wine", "alcohol", "bars", "dining", "groceries",
            "gas", "parking", "charging", "garage", "car", "taxi", "bike", "train", "metro", "transit", "flights", "hotel",
            "travel", "rent", "condo", "hydro", "water", "heating", "phone", "internet", "tv", "insurance", "furniture",
            "repairs", "cleaning", "laundry", "garden", "clothes", "computer", "audio", "toys", "flowers", "jewels",
            "shopping", "medication", "pharmacy", "hospital", "dentist", "glasses", "therapy", "haircut", "beauty", "yoga",
            "gym", "health", "movies", "games", "music", "party", "theatre", "art", "camping", "hiking", "pool", "casino",
            "tickets", "baby", "kids", "school", "books", "pets", "family", "parents", "charity", "church", "birthday",
            "gifts", "subscriptions", "investments", "crypto", "taxes", "loans", "credit card", "fees", "lawyer", "salary",
            "business", "bonus", "refunds", "sold", "cash", "shipping", "mail", "online", "cloud",
        ];
        for w in words {
            let key = suggest(w);
            assert!(key.is_some(), "{w} suggested nothing");
            assert!(key.is_some_and(|k| ICON_KEYS.contains(&k)), "{w} suggested {key:?}, which is not an icon key");
        }
    }

    #[test]
    fn icon_keys_are_unique_and_keep_the_original_thirty_first() {
        let unique: std::collections::HashSet<_> = ICON_KEYS.iter().collect();
        assert_eq!(unique.len(), ICON_KEYS.len());
        assert_eq!(ICON_KEYS.first(), Some(&"cart"));
        assert_eq!(ICON_KEYS[29], "book");
    }

    #[test]
    fn normalizing_drops_accents_brackets_and_punctuation() {
        assert_eq!(normalize("AMOUNT (CAD)"), "amount");
        assert_eq!(normalize("Épicerie / Marché"), "epicerie marche");
        assert_eq!(normalize("Cafe\u{301}s"), "cafes", "a decomposed accent vanishes too");
        assert_eq!(normalize("a (b\nc) d"), "a b c d", "a bracket never closes across a line");
        assert_eq!(normalize("Œuvres, Straße"), "uvres stra e", "no canonical decomposition, so no letter");
    }
}
