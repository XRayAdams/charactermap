// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

//! Look up official Unicode names for characters.
//!
//! Names come from a compact table generated from the Unicode Character
//! Database (see `tools/generate_char_names.py` and `character_names_data.rs`).
//! Directly assigned characters are looked up by binary search; algorithmically
//! named ranges (CJK ideographs, Hangul syllables, ...) are derived on demand
//! following the rules in the Unicode Standard.

use super::character_names_data::{NAMES, RANGES};

use crate::unicode::UnicodeEntry;

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

    /// Finds up to `max_results` displayable characters within `sections`
    /// whose name contains `query` (case-insensitive).
    ///
    /// This deliberately does NOT enumerate every codepoint in `sections`
    /// (as naively calling `name()` per codepoint would) -- most of the
    /// Unicode range is covered by a handful of huge algorithmically-named
    /// `RANGES` (CJK ideographs, private use, ...)
    pub fn search(&self, query: &str, sections: &[UnicodeEntry], max_results: usize) -> Vec<char> {
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();

        for section in sections {
            let (start, end) = (section.start_index, section.end_index);

            let lo = NAMES.partition_point(|&(code, _)| code < start);
            let hi = NAMES.partition_point(|&(code, _)| code <= end);
            for &(code, name) in &NAMES[lo..hi] {
                let Some(ch) = char::from_u32(code).filter(|ch| !ch.is_control()) else {
                    continue;
                };
                if name.to_lowercase().contains(&query_lower) {
                    matches.push(ch);
                    if matches.len() >= max_results {
                        return matches;
                    }
                }
            }

            for &(range_start, range_end, label) in RANGES {
                let overlap_start = start.max(range_start);
                let overlap_end = end.min(range_end);
                if overlap_start > overlap_end || !label.to_lowercase().contains(&query_lower) {
                    continue;
                }
                for code in overlap_start..=overlap_end {
                    let Some(ch) = char::from_u32(code).filter(|ch| !ch.is_control()) else {
                        continue;
                    };
                    matches.push(ch);
                    if matches.len() >= max_results {
                        return matches;
                    }
                }
            }
        }

        matches
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
