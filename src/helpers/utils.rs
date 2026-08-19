// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use libadwaita as adw;
use gtk4::{gio, glib::{self, object::{Cast, IsA, ObjectExt}, value::ToValue}, pango::prelude::FontExt, prelude::{ListItemExt, WidgetExt}};

use crate::{helpers::static_data::{CELL_POSITION_KEY, CELL_SIZE}, unicode::{UnicodeCharModel, UnicodeEntry, char_list_model::displayable_count}};



/// Builds a Pango attribute list that selects the given font family (and
/// optionally a point size), for use with `Label`/`Entry` widgets'
/// `set_attributes`.
pub fn font_attr_list(font_name: &str, size_pt: Option<i32>) -> gtk4::pango::AttrList {
    let mut font_desc = gtk4::pango::FontDescription::new();
    font_desc.set_family(font_name);
    if let Some(size_pt) = size_pt {
        font_desc.set_size(size_pt * gtk4::pango::SCALE);
    }
    let attrs = gtk4::pango::AttrList::new();
    attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));
    
    // disable fallback so that unsupported characters render as tofu boxes 
    attrs.insert(gtk4::pango::AttrInt::new_fallback(false));

    attrs
}

/// Returns whether the given already-loaded font has a glyph for at least
/// one codepoint in the inclusive `start..=end` range.
pub fn font_covers_range(font: &gtk4::pango::Font, start: u32, end: u32) -> bool {
    (start..=end).any(|code| {
        char::from_u32(code)
            .filter(|ch| !ch.is_control())
            .is_some_and(|ch| font.has_char(ch))
    })
}

/// Returns a new `adw::Breakpoint` with the given setters applied. Used with OverlaySplitView
pub fn bp_with_setters(
    bp: adw::Breakpoint,
    additions: &[(&impl IsA<glib::Object>, &str, impl ToValue)],
) -> adw::Breakpoint {
    bp.add_setters(additions);
    bp
}


pub fn apply_font_preview(label: &gtk4::Label, font_name: &str, enabled: bool) {
    if enabled {
        let mut font_desc = gtk4::pango::FontDescription::new();
        font_desc.set_family(font_name);
        let attrs = gtk4::pango::AttrList::new();
        attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));
        label.set_attributes(Some(&attrs));
    } else {
        label.set_attributes(None);
    }
}


/// Recursively collects every character-cell `Inscription` currently realized
/// under the grid view (all `Inscription`s in the subtree are cells).
pub fn collect_cell_labels(widget: &gtk4::Widget, out: &mut Vec<gtk4::Inscription>) {
    let mut child = widget.first_child();
    while let Some(w) = child {
        if let Some(label) = w.downcast_ref::<gtk4::Inscription>() {
            out.push(label.clone());
        } else {
            collect_cell_labels(&w, out);
        }
        child = w.next_sibling();
    }
}


/// Updates the sticky "current block" header to the block that owns the
/// top-most visible grid cell
pub fn update_sticky_header(
    grid_view: &gtk4::GridView,
    boundaries: &[(u32, String)],
    header: &gtk4::Label,
) {
    if boundaries.is_empty() {
        return;
    }

    let mut labels = Vec::new();
    collect_cell_labels(grid_view.upcast_ref::<gtk4::Widget>(), &mut labels);

    // Find the geometrically top-most, at-least-half-visible realized cell
    // and use its model position to resolve the block. Skip unmapped
    // (recycled/pooled) cells, and break ties by smallest position, so a
    // block boundary straddling a row resolves deterministically.
    let viewport_height = grid_view.height() as f32;
    let mut best: Option<(f32, u32)> = None;

    for label in &labels {
        // Ignore pooled/recycled cells that are not currently on screen.
        if !label.is_mapped() {
            continue;
        }

        let Some(point) = label.compute_point(grid_view, &gtk4::graphene::Point::new(0.0, 0.0))
        else {
            continue;
        };
        let y = point.y();
        let height = label.height() as f32;

        // Must be at least half visible and inside the viewport (not in the
        // bottom recycling buffer).
        if y + height * 0.5 <= 0.0 || y >= viewport_height {
            continue;
        }

        let Some(position) =
            (unsafe { label.data::<u32>(CELL_POSITION_KEY) }).map(|ptr| unsafe { *ptr.as_ref() })
        else {
            continue;
        };

        // Pick the top-most cell; ties within a row prefer the left-most.
        let replace = match best {
            None => true,
            Some((best_y, best_pos)) => {
                y < best_y - 0.5 || ((y - best_y).abs() <= 0.5 && position < best_pos)
            }
        };
        if replace {
            best = Some((y, position));
        }
    }

    if let Some((_, position)) = best {
        let index = boundaries
            .partition_point(|(start, _)| *start <= position)
            .saturating_sub(1);
        header.set_label(&boundaries[index].1);
    }
}


/// Measures the grid's actual column count and row pitch (px) from its
/// realized cells, since `GtkGridView` computes both itself. Returns `None`
/// if too few cells are realized to measure.
pub fn grid_geometry(grid_view: &gtk4::GridView) -> Option<(u32, f64)> {
    let mut cells = Vec::new();
    collect_cell_labels(grid_view.upcast_ref(), &mut cells);

    let mut ys: Vec<f32> = Vec::new();
    for cell in &cells {
        if !cell.is_mapped() {
            continue;
        }
        if let Some(point) = cell.compute_point(grid_view, &gtk4::graphene::Point::new(0.0, 0.0)) {
            ys.push(point.y());
        }
    }
    if ys.len() < 2 {
        return None;
    }
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Bucket cells into rows (same y within a small tolerance), counting how
    // many cells fall in each row.
    let mut rows: Vec<(f32, u32)> = Vec::new();
    for &y in &ys {
        match rows.last_mut() {
            Some(last) if (y - last.0).abs() < 2.0 => last.1 += 1,
            _ => rows.push((y, 1)),
        }
    }
    if rows.len() < 2 {
        return None;
    }

    // A full row has the maximum cell count; the row pitch is the smallest
    // positive gap between consecutive rows.
    let columns = rows
        .iter()
        .map(|&(_, count)| count)
        .max()
        .unwrap_or(1)
        .max(1);
    let mut pitch = f32::MAX;
    for pair in rows.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        if gap > 1.0 && gap < pitch {
            pitch = gap;
        }
    }
    if pitch == f32::MAX {
        return None;
    }

    Some((columns, f64::from(pitch)))
}



/// Builds the grid's flat data store: a lazy `UnicodeCharModel` that
/// resolves codepoints on demand
pub fn build_unicode_store(sections: &[UnicodeEntry]) -> UnicodeCharModel {
    UnicodeCharModel::new(sections)
}

/// Builds an eager `gio::ListStore` of the given characters, in order.
pub fn build_search_result_store(chars: &[char]) -> gio::ListStore {
    let store = gio::ListStore::new::<gtk4::StringObject>();
    let mut buf = [0u8; 4];
    for &ch in chars {
        store.append(&gtk4::StringObject::new(ch.encode_utf8(&mut buf)));
    }
    store
}

/// Computes block description -> flat start position, and the sorted list
/// of flat-position -> block boundaries, mirroring the grid's model order.
pub fn compute_positions_boundaries(
    sections: &[UnicodeEntry],
) -> (HashMap<String, u32>, Vec<(u32, String)>) {
    let mut positions = HashMap::new();
    let mut boundaries = Vec::new();
    let mut position: u32 = 0;

    for section in sections {
        let count = displayable_count(section.start_index, section.end_index);

        if count == 0 {
            continue;
        }

        positions.insert(section.description.clone(), position);
        boundaries.push((position, section.description.clone()));
        position += count;
    }

    (positions, boundaries)
}

/// Builds the factory that renders each grid cell as a fixed-size character
/// label in the currently selected font (via the shared `cell_attrs`).
pub fn build_unicode_grid_factory(
    cell_attrs: Rc<RefCell<gtk4::pango::AttrList>>,
    block_boundaries: Rc<RefCell<Vec<(u32, String)>>>,
    loaded_font: Rc<RefCell<Option<gtk4::pango::Font>>>,
) -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();

    factory.connect_setup(|_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };

        // GtkInscription (not GtkLabel) has a fixed render area and never
        // resizes to glyph content, preventing columns from reflowing when
        // wide glyphs (emoji, CJK) are realized.
        let label = gtk4::Inscription::builder()
            .width_request(CELL_SIZE)
            .height_request(CELL_SIZE)
            .min_chars(1)
            .nat_chars(1)
            .min_lines(1)
            .nat_lines(1)
            .xalign(0.5)
            .yalign(0.5)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Center)
            .hexpand(true)
            .css_classes(vec!["unicode-cell"])
            .build();

        list_item.set_child(Some(&label));
    });

    factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };

        let Some(string_obj) = list_item
            .item()
            .and_then(|item| item.downcast::<gtk4::StringObject>().ok())
        else {
            return;
        };

        let Some(label) = list_item
            .child()
            .and_then(|w| w.downcast::<gtk4::Inscription>().ok())
        else {
            return;
        };

        label.set_text(Some(string_obj.string().as_str()));
        label.set_attributes(Some(&cell_attrs.borrow()));

        // Record this cell's flat position so the sticky-header scroll handler
        // can map the top-visible cell back to its unicode block.
        let position = list_item.position();
        unsafe {
            label.set_data::<u32>(CELL_POSITION_KEY, position);
        }

        // Shade cells by block parity so adjacent blocks are visually
        // distinguishable (bands can change mid-row).
        let block_alt = {
            let boundaries = block_boundaries.borrow();
            boundaries
                .partition_point(|(start, _)| *start <= position)
                .saturating_sub(1)
                % 2
                == 1
        };
        if block_alt {
            if !label.has_css_class("block-alt") {
                label.add_css_class("block-alt");
            }
        } else if label.has_css_class("block-alt") {
            label.remove_css_class("block-alt");
        }

        // Shade cells the selected font has no glyph for
        apply_no_glyph_class(&label, &loaded_font.borrow());
    });

    factory
}

/// Toggles the "no-glyph" CSS class on a cell based on whether `font` has a
/// glyph 
pub fn apply_no_glyph_class(label: &gtk4::Inscription, font: &Option<gtk4::pango::Font>) {
    let ch = label.text().and_then(|text| text.chars().next());
    let no_glyph = match (font, ch) {
        (Some(font), Some(ch)) => !font.has_char(ch),
        _ => false,
    };
    if no_glyph {
        if !label.has_css_class("no-glyph") {
            label.add_css_class("no-glyph");
        }
    } else if label.has_css_class("no-glyph") {
        label.remove_css_class("no-glyph");
    }
}
