//! The ledger's vocabulary. Port of `Model.kt`.
//!
//! Every enum is stored by NAME (Room, the backup and this store alike), so the serde names are
//! the Kotlin constant names and renaming one is a migration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TxType {
    Expense,
    Income,
    Transfer,
}

/// `Investment` holds money put to work (a TFSA, an RRSP, a brokerage account); its value can be
/// updated by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AccountType {
    Cash,
    Chequing,
    Savings,
    Credit,
    Investment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CategoryKind {
    Expense,
    Income,
}

/// What a goal measures.
///
/// - `Savings`: a pot filled by contributions, toward a target and an optional date.
/// - `Balance`: an account (or everything you own, net of what you owe) reaching an amount by a date.
/// - `Invest`: each month, money moved into investment accounts: a share of what came in, or a set amount.
/// - `Save`: each month, what is kept of what came in: a share of it, or a set amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoalKind {
    Savings,
    Balance,
    Invest,
    Save,
}

/// A category seeded on first run. `icon` is a key the UI maps to a drawn glyph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CategorySeed {
    pub name: &'static str,
    pub kind: CategoryKind,
    pub color: i32,
    pub icon: &'static str,
}

/// Twelve hues tuned for the warm near-black ground; [`CategorySeed::color`] indexes into them.
pub const PALETTE_SIZE: i32 = 12;

const fn seed(name: &'static str, kind: CategoryKind, color: i32, icon: &'static str) -> CategorySeed {
    CategorySeed { name, kind, color, icon }
}

pub const DEFAULT_CATEGORIES: [CategorySeed; 17] = [
    seed("Groceries", CategoryKind::Expense, 0, "cart"),
    seed("Dining", CategoryKind::Expense, 6, "dining"),
    seed("Transport", CategoryKind::Expense, 2, "transport"),
    seed("Housing", CategoryKind::Expense, 9, "home"),
    seed("Utilities", CategoryKind::Expense, 7, "bolt"),
    seed("Phone & internet", CategoryKind::Expense, 1, "wifi"),
    seed("Shopping", CategoryKind::Expense, 4, "bag"),
    seed("Health", CategoryKind::Expense, 5, "health"),
    seed("Entertainment", CategoryKind::Expense, 3, "ticket"),
    seed("Subscriptions", CategoryKind::Expense, 10, "repeat"),
    seed("Travel", CategoryKind::Expense, 8, "flight"),
    seed("Gifts", CategoryKind::Expense, 11, "gift"),
    seed("Other", CategoryKind::Expense, 10, "dots"),
    seed("Salary", CategoryKind::Income, 0, "work"),
    seed("Side income", CategoryKind::Income, 1, "spark"),
    seed("Refunds", CategoryKind::Income, 2, "refund"),
    seed("Other income", CategoryKind::Income, 10, "dots"),
];

/// Every icon key a client must be able to draw. Appended, never reordered: the keys are stored.
pub const ICON_KEYS: &[&str] = &[
    "cart", "dining", "coffee", "transport", "car", "fuel", "home", "bolt", "wifi", "bag",
    "health", "ticket", "repeat", "flight", "gift", "dots", "work", "spark", "refund", "pets",
    "school", "child", "sport", "beauty", "savings", "bank", "card", "cash", "fees", "book",
    // Added with the grouped picker.
    "pizza", "fastfood", "lunch", "ramen", "bakery", "icecream", "bar", "beer", "wine", "liquor", "tea", "kitchen",
    "train", "subway", "taxi", "bike", "scooter", "ev", "parking", "carrepair", "boat",
    "apartment", "water", "heat", "furniture", "repairs", "cleaning", "laundry", "garden", "insurance", "phone", "tv",
    "clothes", "computer", "headphones", "toys", "flowers", "jewelry", "store", "watch", "camera",
    "hospital", "medication", "pharmacy", "medical", "haircut", "wellness", "therapy", "eyes",
    "movie", "games", "music", "party", "outdoors", "soccer", "hiking", "pool", "casino", "theatre", "art", "hotel", "luggage", "beach",
    "baby", "family", "elderly", "charity", "church", "birthday",
    "invest", "crypto", "tax", "wallet", "loan", "legal", "business", "atm", "sale",
    "star", "idea", "online", "cloud", "delivery", "mail",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_serialize_by_their_kotlin_names() {
        assert_eq!(serde_json::to_string(&TxType::Transfer).unwrap(), "\"TRANSFER\"");
        assert_eq!(serde_json::to_string(&AccountType::Chequing).unwrap(), "\"CHEQUING\"");
        assert_eq!(serde_json::from_str::<GoalKind>("\"SAVE\"").unwrap(), GoalKind::Save);
    }

    #[test]
    fn seeded_categories_use_known_icons_and_colours() {
        for c in &DEFAULT_CATEGORIES {
            assert!(ICON_KEYS.contains(&c.icon), "{}", c.icon);
            assert!((0..PALETTE_SIZE).contains(&c.color));
        }
    }

    #[test]
    fn icon_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for k in ICON_KEYS {
            assert!(seen.insert(k), "{k} twice");
        }
    }
}
