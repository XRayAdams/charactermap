// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.
//
// Custom `gio::ListModel` backed directly by the (small) list of unicode
// block ranges
use gtk4::{gio, glib, subclass::prelude::*};

use super::UnicodeEntry;

const EXCLUDED_RANGES: [(u32, u32); 3] = [(0x00, 0x1F), (0x7F, 0x9F), (0xD800, 0xDFFF)];

/// Number of displayable (non-excluded) codepoints in the inclusive
/// `start..=end` range. O(1) -- no codepoint is ever enumerated.
pub(crate) fn displayable_count(start: u32, end: u32) -> u32 {
    let mut total = 0u32;
    let mut cursor = start;
    for &(excluded_start, excluded_end) in &EXCLUDED_RANGES {
        if cursor > end {
            return total;
        }
        if excluded_start > cursor {
            let segment_end = (excluded_start - 1).min(end);
            total += segment_end - cursor + 1;
        }
        if excluded_end >= cursor {
            cursor = cursor.max(excluded_end + 1);
        }
    }
    if cursor <= end {
        total += end - cursor + 1;
    }
    total
}

/// The `local_offset`-th (0-based) displayable codepoint in the inclusive
/// `start..=end` range, skipping `EXCLUDED_RANGES`. O(1).
fn nth_displayable_char(start: u32, end: u32, mut local_offset: u32) -> Option<char> {
    let mut cursor = start;
    for &(excluded_start, excluded_end) in &EXCLUDED_RANGES {
        if cursor > end {
            return None;
        }
        if excluded_start > cursor {
            let segment_end = (excluded_start - 1).min(end);
            let segment_len = segment_end - cursor + 1;
            if local_offset < segment_len {
                return char::from_u32(cursor + local_offset);
            }
            local_offset -= segment_len;
        }
        if excluded_end >= cursor {
            cursor = cursor.max(excluded_end + 1);
        }
    }
    if cursor <= end {
        let segment_len = end - cursor + 1;
        if local_offset < segment_len {
            return char::from_u32(cursor + local_offset);
        }
    }
    None
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk4::{gio, glib, prelude::*, subclass::prelude::*};

    /// One included block: `cumulative_start` is the flat model position of
    /// its first displayable char; `start`/`end` are its raw codepoint range.
    #[derive(Default)]
    pub struct UnicodeCharModel {
        pub(super) sections: RefCell<Vec<(u32, u32, u32)>>,
        pub(super) total: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UnicodeCharModel {
        const NAME: &'static str = "CharacterMapUnicodeCharModel";
        type Type = super::UnicodeCharModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for UnicodeCharModel {}

    impl ListModelImpl for UnicodeCharModel {
        fn item_type(&self) -> glib::Type {
            gtk4::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.total.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let sections = self.sections.borrow();
            let idx =
                sections.partition_point(|&(cumulative_start, _, _)| cumulative_start <= position);
            if idx == 0 {
                return None;
            }
            let (cumulative_start, start, end) = sections[idx - 1];
            let local_offset = position - cumulative_start;
            let ch = super::nth_displayable_char(start, end, local_offset)?;
            let mut buf = [0u8; 4];
            Some(gtk4::StringObject::new(ch.encode_utf8(&mut buf)).upcast())
        }
    }
}

glib::wrapper! {
    pub struct UnicodeCharModel(ObjectSubclass<imp::UnicodeCharModel>)
        @implements gio::ListModel;
}

impl UnicodeCharModel {
    /// Builds a model for the given (already filtered) blocks, in order.
    /// O(number of blocks), NOT O(number of characters) -- no codepoint is
    /// ever enumerated at construction time, only counted analytically.
    pub fn new(sections: &[UnicodeEntry]) -> Self {
        let model: Self = glib::Object::new();

        let mut table = Vec::with_capacity(sections.len());
        let mut cumulative = 0u32;
        for section in sections {
            let count = displayable_count(section.start_index, section.end_index);
            if count == 0 {
                continue;
            }
            table.push((cumulative, section.start_index, section.end_index));
            cumulative += count;
        }

        model.imp().sections.replace(table);
        model.imp().total.set(cumulative);
        model
    }
}

#[cfg(test)]
mod tests {
    use gtk4::prelude::*;

    use super::*;

    /// Brute-force reference: every displayable char in `start..=end`, in
    /// order (mirrors the old eager `build_unicode_store` loop exactly).
    fn brute_force(start: u32, end: u32) -> Vec<char> {
        (start..=end)
            .filter_map(|code| char::from_u32(code).filter(|ch| !ch.is_control()))
            .collect()
    }

    fn check(start: u32, end: u32) {
        let expected = brute_force(start, end);
        assert_eq!(
            displayable_count(start, end),
            expected.len() as u32,
            "count mismatch for {start:#x}..={end:#x}"
        );
        for (i, &ch) in expected.iter().enumerate() {
            assert_eq!(
                nth_displayable_char(start, end, i as u32),
                Some(ch),
                "mismatch at offset {i} for {start:#x}..={end:#x}"
            );
        }
        assert_eq!(
            nth_displayable_char(start, end, expected.len() as u32),
            None,
            "expected None past the end for {start:#x}..={end:#x}"
        );
    }

    #[test]
    fn basic_latin_and_c0_controls() {
        check(0x00, 0x7F); // C0 controls + printable ASCII + DEL
    }

    #[test]
    fn latin1_supplement_and_c1_controls() {
        check(0x80, 0xFF); // C1 controls + Latin-1 Supplement
    }

    #[test]
    fn plain_block_no_exclusions() {
        check(0x0370, 0x03FF); // Greek and Coptic -- no overlap with any excluded range
    }

    #[test]
    fn surrogates_fully_excluded() {
        check(0xD800, 0xDFFF);
        assert_eq!(displayable_count(0xD800, 0xDFFF), 0);
        assert_eq!(nth_displayable_char(0xD800, 0xDFFF, 0), None);
    }

    #[test]
    fn block_spanning_into_surrogates() {
        check(0xD700, 0xD900);
    }

    #[test]
    fn single_codepoint() {
        check(0x0041, 0x0041); // 'A'
        check(0x0000, 0x0000); // NUL, excluded
        assert_eq!(displayable_count(0x0000, 0x0000), 0);
    }

    /// Diagnostic: validate the analytical count/mapping against the real
    /// production block list (no GTK/GObject involved -- pure logic), to
    /// catch any real-data edge case a synthetic test might miss.
    #[test]
    fn matches_brute_force_over_real_unicode_set() {
        let sections = crate::unicode::UnicodeSet::new().unicode_sections;

        let mut cumulative = 0u32;
        for section in &sections {
            let count = displayable_count(section.start_index, section.end_index);
            let expected: Vec<char> = brute_force(section.start_index, section.end_index);
            assert_eq!(
                count,
                expected.len() as u32,
                "count mismatch for block {:?} ({:#x}..={:#x})",
                section.description,
                section.start_index,
                section.end_index
            );
            for (i, &ch) in expected.iter().enumerate() {
                let got = nth_displayable_char(section.start_index, section.end_index, i as u32);
                assert_eq!(
                    got,
                    Some(ch),
                    "mismatch at local offset {i} (flat position {}) in block {:?} ({:#x}..={:#x})",
                    cumulative + i as u32,
                    section.description,
                    section.start_index,
                    section.end_index
                );
                assert!(
                    !ch.is_control(),
                    "produced a control char {:?} at local offset {i} in block {:?}",
                    ch,
                    section.description
                );
            }
            cumulative += count;
        }
    }

    #[test]
    fn model_matches_brute_force() {
        gtk4::init().expect("gtk4::init should succeed in the test environment");

        let sections = vec![
            UnicodeEntry::new(0x00, "C0 Controls and Basic Latin", 0x7F, true),
            UnicodeEntry::new(0xD800, "Surrogates", 0xDFFF, true), // fully excluded, must be skipped
            UnicodeEntry::new(0x0370, "Greek and Coptic", 0x0373, true),
        ];

        let model = UnicodeCharModel::new(&sections);

        let mut expected = brute_force(0x00, 0x7F);
        expected.extend(brute_force(0xD800, 0xDFFF));
        expected.extend(brute_force(0x0370, 0x0373));

        assert_eq!(model.n_items(), expected.len() as u32);
        for (i, &ch) in expected.iter().enumerate() {
            let item = model.item(i as u32).expect("item should exist");
            let text = item
                .downcast::<gtk4::StringObject>()
                .expect("item should be a StringObject")
                .string();
            assert_eq!(text.chars().next(), Some(ch), "mismatch at flat position {i}");
        }
        assert!(model.item(expected.len() as u32).is_none());

        // Now against the real, unfiltered production block list.
        let real_sections = crate::unicode::UnicodeSet::new().unicode_sections;
        let mut real_expected = Vec::new();
        for section in &real_sections {
            real_expected.extend(brute_force(section.start_index, section.end_index));
        }

        let real_model = UnicodeCharModel::new(&real_sections);
        assert_eq!(real_model.n_items(), real_expected.len() as u32);

        for (i, &ch) in real_expected.iter().enumerate() {
            let item = real_model
                .item(i as u32)
                .unwrap_or_else(|| panic!("item {i} should exist"));
            let text = item
                .downcast::<gtk4::StringObject>()
                .expect("item should be a StringObject")
                .string();
            assert_eq!(text.chars().next(), Some(ch), "mismatch at flat position {i}");
        }
    }
}

