// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

//! Look up official Unicode names for characters.
//!
//! Names come from a compact table generated from the Unicode Character
//! Database (see `tools/generate_char_names.py` and `character_names_data.rs`).
//! Directly assigned characters are looked up by binary search; algorithmically
//! named ranges (CJK ideographs, Hangul syllables, ...) are derived on demand
//! following the rules in the Unicode Standard.

use super::character_names_data::{NAMES, RANGES};

// Jamo short-name tables used to derive Hangul syllable names
// (see the Unicode Standard, "Hangul Syllable Name Generation").
const JAMO_LEADING: [&str; 19] = [
    "G", "GG", "N", "D", "DD", "R", "M", "B", "BB", "S", "SS", "", "J", "JJ", "C", "K", "T", "P",
    "H",
];
const JAMO_VOWEL: [&str; 21] = [
    "A", "AE", "YA", "YAE", "EO", "E", "YEO", "YE", "O", "WA", "WAE", "OE", "YO", "U", "WEO", "WE",
    "WI", "YU", "EU", "YI", "I",
];
const JAMO_TRAILING: [&str; 28] = [
    "", "G", "GG", "GS", "N", "NJ", "NH", "D", "L", "LG", "LM", "LB", "LS", "LT", "LP", "LH", "M",
    "B", "BS", "S", "SS", "NG", "J", "C", "K", "T", "P", "H",
];

/// Derives the name of a character inside an algorithmically named range.
fn range_name(label: &str, code: u32) -> String {
    if label.starts_with("CJK Ideograph") {
        format!("CJK Ideograph-{code:04X}")
    } else if label.starts_with("Tangut Ideograph") {
        format!("Tangut Ideograph-{code:04X}")
    } else if label == "Hangul Syllable" {
        hangul_syllable_name(code)
    } else {
        // Ranges without a formal per-character name (e.g. Private Use).
        format!("{}-{code:04X}", label.to_lowercase())
    }
}

/// Derives the canonical "HANGUL SYLLABLE ..." name for a precomposed syllable.
fn hangul_syllable_name(code: u32) -> String {
    let index = code - 0xAC00;
    let leading = (index / 588) as usize;
    let vowel = ((index % 588) / 28) as usize;
    let trailing = (index % 28) as usize;
    format!(
        "Hangul syllable {}{}{}",
        JAMO_LEADING[leading], JAMO_VOWEL[vowel], JAMO_TRAILING[trailing]
    )
}

/// Provides Unicode names for characters.
pub struct CharacterNames;

impl CharacterNames {
    pub fn new() -> Self {
        Self
    }

    /// Returns the Unicode name of `ch`, if one is defined.
    pub fn name(&self, ch: char) -> Option<String> {
        let code = ch as u32;
        if let Ok(index) = NAMES.binary_search_by_key(&code, |&(entry, _)| entry) {
            return Some(NAMES[index].1.to_string());
        }
        RANGES
            .iter()
            .find(|(start, end, _)| code >= *start && code <= *end)
            .map(|(_, _, label)| range_name(label, code))
    }
}

impl Default for CharacterNames {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::CharacterNames;

    #[test]
    fn resolves_names() {
        let names = CharacterNames::new();
        assert_eq!(names.name('A').as_deref(), Some("Latin capital letter a"));
        assert_eq!(names.name('€').as_deref(), Some("Euro sign"));
        assert_eq!(names.name('中').as_deref(), Some("cjk ideograph-4E2D"));
        assert_eq!(names.name('가').as_deref(), Some("hangul syllable-AC00"));
        assert_eq!(names.name('힣').as_deref(), Some("hangul syllable-D7A3"));
    }
}
