use adw::prelude::*;
use gtk4::{gio, glib, prelude::*};
use libadwaita as adw;
use relm4::actions::RelmActionGroup;
use relm4::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use crate::helpers::actions::{AboutAction, WindowActionGroup, create_about_action};
use crate::helpers::character_names::CharacterNames;
use crate::helpers::static_data::APP_NAME;
use crate::unicode::char_list_model::displayable_count;
use crate::unicode::{UnicodeCharModel, UnicodeEntry, UnicodeSet};

const SPACING_MEDIUM: i32 = 12;
const SPACING_SMALL: i32 = 6;

/// Cell width/height (in pixels) used to render each character cell. The
/// number of columns is computed automatically by `GtkGridView` from the
/// available width and this per-cell size.
const CELL_SIZE: i32 = 36;

/// Point size used to render each character in the grid cells.
const GRID_FONT_SIZE: i32 = 16;

/// Builds a Pango attribute list that selects the given font family (and
/// optionally a point size), for use with `Label`/`Entry` widgets'
/// `set_attributes`.
fn font_attr_list(font_name: &str, size_pt: Option<i32>) -> gtk4::pango::AttrList {
    let mut font_desc = gtk4::pango::FontDescription::new();
    font_desc.set_family(font_name);
    if let Some(size_pt) = size_pt {
        font_desc.set_size(size_pt * gtk4::pango::SCALE);
    }
    let attrs = gtk4::pango::AttrList::new();
    attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));
    attrs
}

#[derive(Serialize, Deserialize)]
struct AppSettings {
    render_font_preview: bool,
    filter_unicode_pages: bool,
}

pub struct App {
    selected_font: String,
    is_collapsed: bool,
    fonts: Vec<String>,
    render_font_preview: bool,
    /// When on, the character grid only shows unicode blocks the selected font
    /// actually covers; when off, all blocks are shown (which also makes font
    /// switching much faster, as the grid model no longer changes).
    filter_unicode_pages: bool,
    font_list: Option<gtk::ListBox>,
    unicode_set: UnicodeSet,
    unicode_grid_view: Option<gtk::GridView>,
    /// The grid's native single-selection model; its inner store is swapped on
    /// font change. GTK draws the selection highlight and handles keyboard nav.
    unicode_selection: Option<gtk::SingleSelection>,
    unicode_set_list: Option<gtk::ListBox>,
    section_positions: HashMap<String, u32>,
    /// Codepoint ranges of the blocks currently loaded into the grid model.
    /// Lets us skip the expensive `set_model` rebuild when a font change
    /// doesn't change which characters are shown (e.g. with filtering off).
    displayed_ranges: Vec<(u32, u32)>,
    selected_character: Option<char>,
    hex_value: String,
    dec_value: String,
    collected_text: String,
    /// Shared Pango attributes (selected font + size) applied to every grid
    /// cell; updated in place on font change so recycled cells pick it up.
    cell_attrs: Rc<RefCell<gtk4::pango::AttrList>>,
    /// Flat-position -> unicode-block-description boundaries (sorted ascending
    /// by start position), used to resolve the sticky "current block" header
    /// from the top-visible cell during scrolling.
    block_boundaries: Rc<RefCell<Vec<(u32, String)>>>,
    sticky_header: Option<gtk::Label>,
    character_names: CharacterNames,
    character_name: String,
    hex_entry: Option<gtk::Entry>,
    dec_entry: Option<gtk::Entry>,
}

#[derive(Debug)]
pub enum Messages {
    FontSelected(String),
    CharacterSelected(i32),
    CharacterDoubleClicked(i32),
    ClearCollectedText,
    SetCollapsed(bool),
    SetFontPreview(bool),
    SetFilterUnicodePages(bool),
    JumpToUnicodeSet(String),
    SetHexValue(String),
    SetDecValue(String),
    FindHex,
    FindDec,
}

impl App {
    fn get_app_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn get_config_path() -> PathBuf {
        let mut path = gtk4::glib::user_config_dir();
        path.push("charactermap");
        std::fs::create_dir_all(&path).ok();
        path.push("config.json");
        path
    }

    fn save_config(&self) {
        let settings = AppSettings {
            render_font_preview: self.render_font_preview,
            filter_unicode_pages: self.filter_unicode_pages,
        };
        if let Ok(content) = serde_json::to_string_pretty(&settings) {
            let path = Self::get_config_path();
            let _ = fs::write(path, content);
        }
    }

    fn load_config() -> AppSettings {
        let path = Self::get_config_path();
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
                return settings;
            }
        }
        AppSettings {
            render_font_preview: true,
            filter_unicode_pages: false,
        }
    }

    /// Recomputes which unicode blocks the currently selected font supports,
    /// rebuilds the character grid model and the "Jump to Unicode Set" list.
    fn refresh_unicode_sections(&mut self) {
        let Some(font_list) = self.font_list.clone() else {
            return;
        };
        let context = font_list.pango_context();
        let font_name = self.selected_font.clone();

        self.unicode_set.filtered_unicode_sections = if self.filter_unicode_pages {
            // Keep only the blocks the selected font actually covers.
            let mut font_desc = gtk4::pango::FontDescription::new();
            font_desc.set_family(&font_name);
            let font = context.load_font(&font_desc);

            self.unicode_set
                .unicode_sections
                .iter()
                .filter(|entry| {
                    font.as_ref().is_some_and(|font| {
                        font_covers_range(font, entry.start_index, entry.end_index)
                    })
                })
                .cloned()
                .collect()
        } else {
            // No coverage filtering: show every block (unsupported glyphs
            // render as tofu boxes, but font switching is near-instant).
            self.unicode_set.unicode_sections.clone()
        };

        // Update the shared cell attributes so freshly-bound cells render in
        // the selected font.
        *self.cell_attrs.borrow_mut() = font_attr_list(&font_name, Some(GRID_FONT_SIZE));

        if let Some(grid_view) = &self.unicode_grid_view {
            // Positions/boundaries mirror the model order (covered blocks in
            // ascending order, each contributing its non-control chars).
            // Update them BEFORE swapping the model so the factory's `bind`
            // callback resolves the correct block for every cell.
            let (positions, boundaries) =
                compute_positions_boundaries(&self.unicode_set.filtered_unicode_sections);

            self.section_positions = positions;

            let first_block = boundaries
                .first()
                .map(|(_, description)| description.clone())
                .unwrap_or_default();
            *self.block_boundaries.borrow_mut() = boundaries;

            // Rebuilding the grid (`set_model`) re-realizes every cell and
            // costs time proportional to the number of characters shown, so it
            // is only worth doing when the visible set of characters actually
            // changes. With filtering off (and between two fonts of identical
            // coverage) the character set is unchanged, so we skip the rebuild
            // entirely and just repaint the already-realized cells in the new
            // font — making those switches near-instant.
            let new_ranges: Vec<(u32, u32)> = self
                .unicode_set
                .filtered_unicode_sections
                .iter()
                .map(|entry| (entry.start_index, entry.end_index))
                .collect();

            if new_ranges != self.displayed_ranges {
                self.displayed_ranges = new_ranges;

                if let Some(selection) = &self.unicode_selection {
                    let store = build_unicode_store(&self.unicode_set.filtered_unicode_sections);
                    selection.set_model(Some(&store));
                }

                if let Some(header) = &self.sticky_header {
                    header.set_label(&first_block);
                }

                grid_view.scroll_to(0, gtk::ListScrollFlags::empty(), None);
            } else {
                // Same characters, only the font changed: push the new font
                // onto every realized cell (cells realized later pick it up
                // from the shared `cell_attrs` when they bind).
                let mut cells = Vec::new();
                collect_cell_labels(grid_view.upcast_ref(), &mut cells);
                let attrs = self.cell_attrs.borrow();
                for cell in &cells {
                    cell.set_attributes(Some(&attrs));
                }
            }
        }

        if let Some(list) = &self.unicode_set_list {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }

            for section in &self.unicode_set.filtered_unicode_sections {
                let label = gtk::Label::new(Some(&section.description));
                label.set_xalign(0.0);
                label.set_margin_top(SPACING_SMALL);
                label.set_margin_bottom(SPACING_SMALL);
                label.set_margin_start(SPACING_SMALL);
                label.set_margin_end(SPACING_SMALL);

                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&label));
                list.append(&row);
            }
        }
    }

    /// Scrolls the character grid so the given unicode block's row is visible.
    fn scroll_to_unicode_set(&self, description: &str) {
        let (Some(grid_view), Some(&position)) = (
            &self.unicode_grid_view,
            self.section_positions.get(description),
        ) else {
            return;
        };

        // Align the block to the TOP of the viewport rather than using
        // `scroll_to` (which only scrolls the item just into view, landing it at
        // the bottom edge). The sticky header tracks the top row, so the target
        // block must be at the top for the header to agree. The column count and
        // row pitch are MEASURED from the realized cells (GridView adds its own
        // spacing, so they can't be derived from CELL_SIZE).
        if let Some(vadjustment) = grid_view.vadjustment() {
            let (columns, row_pitch) = grid_geometry(grid_view).unwrap_or_else(|| {
                (
                    (grid_view.width() / CELL_SIZE).clamp(1, 100) as u32,
                    f64::from(CELL_SIZE),
                )
            });
            let row = position / columns;
            let target = f64::from(row) * row_pitch;
            let max = (vadjustment.upper() - vadjustment.page_size()).max(vadjustment.lower());
            vadjustment.set_value(target.clamp(vadjustment.lower(), max));
        }

        // The scroll-driven header update reads the realized cells' positions,
        // which aren't updated yet at the jump target, so set the header
        // directly to the block we just jumped to for immediate feedback.
        if let Some(header) = &self.sticky_header {
            header.set_label(description);
        }

        // Also select the first character of the block we just jumped to.
        // This triggers `SingleSelection`'s "selection-changed" signal, whose
        // handler (connected in `init`) sends `Messages::CharacterSelected`
        // and updates the character preview/info panel accordingly.
        if let Some(selection) = &self.unicode_selection {
            selection.set_selected(position);
        }
    }

    fn update_character_preview(&mut self) {
        match self.selected_character {
            Some(ch) => {
                self.hex_value = format!("{:04X}", ch as u32);
                self.dec_value = (ch as u32).to_string();
                self.dec_entry
                    .as_ref()
                    .map(|entry| entry.set_text(&self.dec_value));
                self.hex_entry
                    .as_ref()
                    .map(|entry| entry.set_text(&self.hex_value));
                self.character_name = self.character_names.name(ch).unwrap_or_default();
            }
            None => {
                self.hex_value.clear();
                self.dec_value.clear();
                self.character_name.clear();
            }
        }
    }
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    type Init = ();
    type Input = Messages;
    type Output = ();

    menu! {
        main_menu: {
            section! {
                "_About" => AboutAction,
            }
        }
    }

    view! {
        #[root]
        main_window = adw::ApplicationWindow {
            set_title: Some("Character Map"),
            set_default_size: (1100, 800),
            set_resizable: true,

            #[name = "toast_overlay"]
            adw::ToastOverlay {

                #[name = "split_view"]
                adw::OverlaySplitView {
                    connect_collapsed_notify[sender] => move |sv| {
                        sender.input(Messages::SetCollapsed(sv.is_collapsed()));
                    },
                    set_max_sidebar_width: 280.0,

                    #[wrap(Some)]
                    set_sidebar = &adw::NavigationPage {
                        set_title: "Fonts",
                        #[wrap(Some)]
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {

                            },
                            #[wrap(Some)]
                            set_content = &gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_margin_bottom: SPACING_MEDIUM,
                                set_margin_start: SPACING_MEDIUM,
                                set_margin_end: SPACING_MEDIUM,

                                // search box to filter list of fonts, and font preview toggle
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_spacing: SPACING_SMALL,

                                    #[name = "font_search_entry"]
                                    gtk::SearchEntry {
                                        set_placeholder_text: Some("Search fonts…"),
                                        set_hexpand: true,
                                    },

                                    // segmented buttons to toggle font preview rendering
                                    gtk::Box {
                                        set_orientation: gtk::Orientation::Horizontal,
                                        add_css_class: "linked",

                                        #[name = "font_preview_off_button"]
                                        gtk::ToggleButton {
                                            set_icon_name: "format-text-plaintext-symbolic",
                                            set_tooltip_text: Some("Show plain font names"),
                                            #[watch]
                                            set_active: !model.render_font_preview,
                                            connect_toggled[sender] => move |btn| {
                                                if btn.is_active() {
                                                    sender.input(Messages::SetFontPreview(false));
                                                }
                                            },
                                        },

                                        #[name = "font_preview_on_button"]
                                        gtk::ToggleButton {
                                            set_icon_name: "format-text-italic-symbolic",
                                            set_tooltip_text: Some("Preview names using each font"),
                                            set_group: Some(&font_preview_off_button),
                                            #[watch]
                                            set_active: model.render_font_preview,
                                            connect_toggled[sender] => move |btn| {
                                                if btn.is_active() {
                                                    sender.input(Messages::SetFontPreview(true));
                                                }
                                            },
                                        },
                                    },
                                },

                                // list of installed fonts
                                gtk::ScrolledWindow {
                                    set_vexpand: true,
                                    set_hscrollbar_policy: gtk::PolicyType::Automatic,
                                    set_margin_top: SPACING_MEDIUM,

                                    #[name = "font_list"]
                                    gtk::ListBox {
                                        set_selection_mode: gtk::SelectionMode::Single,
                                        connect_row_selected[sender] => move |_, row| {
                                            if let Some(row) = row {
                                                if let Some(label) = row
                                                    .child()
                                                    .and_then(|w| w.downcast::<gtk::Label>().ok())
                                                {
                                                    sender.input(Messages::FontSelected(label.text().to_string()));
                                                }
                                            }
                                        },
                                    }
                                },

                                // toggle: filter grid to blocks the font covers
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_spacing: SPACING_SMALL,
                                    set_margin_top: SPACING_MEDIUM,

                                    gtk::Label {
                                        set_label: "Filter Unicode pages",
                                        set_xalign: 0.0,
                                        set_hexpand: true,
                                    },

                                    gtk::Switch {
                                        set_valign: gtk::Align::Center,
                                        set_tooltip_text: Some("Show only Unicode blocks the selected font supports"),
                                        #[watch]
                                        set_active: model.filter_unicode_pages,
                                        connect_active_notify[sender] => move |sw| {
                                            sender.input(Messages::SetFilterUnicodePages(sw.is_active()));
                                        },
                                    },
                                },
                            }
                        }
                    },

                    #[wrap(Some)]
                    set_content = &adw::NavigationPage {
                        set_title: APP_NAME,

                        #[wrap(Some)]
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {
                                pack_start = &gtk4::Button {
                                    set_icon_name: "sidebar-show-symbolic",
                                    set_can_focus: false,
                                    #[watch]
                                    set_visible: model.is_collapsed,
                                    connect_clicked[split_view] => move |_| {
                                        split_view.set_show_sidebar(true);
                                    },
                                },
                                pack_end = &gtk::MenuButton {
                                    set_icon_name: "open-menu-symbolic",
                                    set_menu_model: Some(&main_menu),
                                    set_direction: gtk::ArrowType::Down,
                                    set_can_focus: false,
                                }
                            },

                            #[wrap(Some)]
                            set_content = &gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_vexpand: true,

                                // sticky header showing the unicode block that
                                // owns the top-most currently-visible cell
                                #[name = "sticky_header"]
                                gtk::Label {
                                    set_xalign: 0.0,
                                    add_css_class: "heading",
                                    set_margin_start: SPACING_MEDIUM,
                                    set_margin_end: SPACING_MEDIUM,
                                    set_margin_top: SPACING_MEDIUM,
                                    set_margin_bottom: SPACING_SMALL,
                                },

                                // virtualized grid of characters (columns are
                                // computed automatically from the width)
                                #[name = "unicode_scroller"]
                                gtk::ScrolledWindow {
                                    set_vexpand: true,
                                    set_hscrollbar_policy: gtk::PolicyType::Never,

                                    #[name = "unicode_grid_view"]
                                    gtk::GridView {
                                        set_min_columns: 1,
                                        set_max_columns: 100,
                                        set_single_click_activate: false,
                                        set_enable_rubberband: false,
                                    }
                                },

                                // bottom bar
                                gtk::Frame {
                                    set_margin_start: SPACING_SMALL,
                                    set_margin_end: SPACING_SMALL,
                                    set_margin_top: SPACING_SMALL,
                                    set_margin_bottom: SPACING_SMALL,
                                    set_label: Some("Character Information"),

                                    #[wrap(Some)]
                                    set_child = &gtk::Box {
                                        set_orientation: gtk::Orientation::Horizontal,
                                        set_margin_start: SPACING_SMALL,
                                        set_height_request: 200,

                                        gtk::Box {
                                            set_orientation: gtk::Orientation::Vertical,
                                            set_spacing: SPACING_SMALL,
                                            set_valign: gtk4::Align::Start,

                                            #[name = "jump_to_set_button"]
                                            gtk::MenuButton {
                                                set_valign: gtk::Align::Center,
                                                set_label: "Jump to Unicode Set",

                                                #[wrap(Some)]
                                                set_popover = &gtk::Popover {
                                                    #[wrap(Some)]
                                                    set_child = &gtk::ScrolledWindow {
                                                        set_min_content_height: 300,
                                                        set_min_content_width: 250,
                                                        set_hscrollbar_policy: gtk::PolicyType::Never,

                                                        #[name = "unicode_set_list"]
                                                        gtk::ListBox {
                                                            set_selection_mode: gtk::SelectionMode::Single,
                                                            connect_row_activated[sender, jump_to_set_button] => move |_, row| {
                                                                if let Some(label) = row
                                                                    .child()
                                                                    .and_then(|w| w.downcast::<gtk::Label>().ok())
                                                                {
                                                                    sender.input(Messages::JumpToUnicodeSet(label.text().to_string()));
                                                                }
                                                                jump_to_set_button.popdown();
                                                            },
                                                        }
                                                    }
                                                },
                                            },

                                            gtk::Entry {
                                                set_placeholder_text: Some("double click character to add here"),
                                                set_icon_from_icon_name[Some("edit-clear-symbolic")]: gtk::EntryIconPosition::Secondary,
                                                set_icon_activatable[true]: gtk::EntryIconPosition::Secondary,
                                                #[watch]
                                                set_icon_sensitive[!model.collected_text.is_empty()]: gtk::EntryIconPosition::Secondary,
                                                connect_icon_release[sender] => move |_, pos| {
                                                    if pos == gtk::EntryIconPosition::Secondary {
                                                        sender.input(Messages::ClearCollectedText);
                                                    }
                                                },
                                                #[watch]
                                                set_text: &model.collected_text,
                                                #[watch]
                                                set_attributes: &if model.collected_text.is_empty() {
                                                    gtk4::pango::AttrList::new()
                                                } else {
                                                    font_attr_list(&model.selected_font, None)
                                                },
                                            },

                                            gtk::Box {
                                                set_orientation: gtk::Orientation::Horizontal,
                                                set_spacing: SPACING_SMALL,

                                                gtk::Label {
                                                    set_label: "Hex:",
                                                    set_valign: gtk::Align::Center,
                                                },

                                                #[name = "hex_entry"]
                                                gtk::Entry {
                                                    set_width_request: 70,
                                                    set_valign: gtk::Align::Center,
                                                    set_max_length: 7,
                                                    connect_changed[sender] => move |entry| {
                                                        sender.input(Messages::SetHexValue(entry.text().to_string()));
                                                    },
                                                },

                                                gtk::Button {
                                                    set_label: "Find",
                                                    set_valign: gtk::Align::Center,
                                                    #[watch]
                                                    set_sensitive: !model.hex_value.is_empty(),
                                                    connect_clicked[sender] => move |_| {
                                                        sender.input(Messages::FindHex);
                                                    },
                                                },
                                            },

                                            gtk::Box {
                                                set_orientation: gtk::Orientation::Horizontal,
                                                set_spacing: SPACING_SMALL,

                                                gtk::Label {
                                                    set_label: "Dec:",
                                                    set_valign: gtk::Align::Center,
                                                },

                                                #[name = "dec_entry"]
                                                gtk::Entry {
                                                    set_width_request: 70,
                                                    set_valign: gtk::Align::Center,
                                                    set_max_length: 7,
                                                    connect_changed[sender] => move |entry|  {
                                                        sender.input(Messages::SetDecValue(entry.text().to_string()));
                                                    },
                                                },

                                                gtk::Button {
                                                    set_label: "Find",
                                                    set_valign: gtk::Align::Center,
                                                    #[watch]
                                                    set_sensitive: !model.dec_value.is_empty(),
                                                    connect_clicked[sender] => move |_| {
                                                        sender.input(Messages::FindDec);
                                                    },
                                                },
                                            },
                                        },

                                        gtk::Box {
                                            set_hexpand: true,
                                        },

                                        gtk::Box {
                                                set_orientation: gtk::Orientation::Vertical,
                                                set_margin_end: SPACING_SMALL,
                                                set_halign: gtk::Align::Start,
                                                set_valign: gtk::Align::Start,

                                            gtk::Box {
                                                set_halign: gtk::Align::End,

                                                gtk::Label {
                                                    #[watch]
                                                    set_label: &model.selected_character.map(|ch| ch.to_string()).unwrap_or_default(),
                                                    set_width_request: 150,
                                                    set_height_request: 150,
                                                    add_css_class: "card",
                                                    set_justify: gtk::Justification::Center,
                                                    #[watch]
                                                    set_attributes: Some(&font_attr_list(&model.selected_font, Some(50))),
                                                },
                                            },

                                            gtk::Label {
                                                #[watch]
                                                set_label: &model.character_name,
                                                set_margin_top: SPACING_SMALL,
                                                set_halign: gtk::Align::End,
                                                set_valign: gtk::Align::End,
                                                set_wrap: true,
                                            }
                                        },

                                },
                            },
                            }
                        },

                    },

                },
            },
            add_breakpoint = bp_with_setters(
                    adw::Breakpoint::new(
                        adw::BreakpointCondition::new_length(
                            adw::BreakpointConditionLengthType::MaxWidth,
                            680.0,
                            adw::LengthUnit::Px,
                        )
                    ),
                    &[(&split_view, "collapsed", true)]
                ),
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let settings = Self::load_config();

        if let Some(display) = gtk4::gdk::Display::default() {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(
                ".unicode-cell { border-radius: 6px; background-color: alpha(currentColor, 0.08); }\n\
                 .unicode-cell.block-alt { background-color: alpha(currentColor, 0.16); }",
            );
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let mut fonts: Vec<String> = root
            .pango_context()
            .list_families()
            .iter()
            .map(|family| family.name().to_string())
            .collect();
        fonts.sort_by_key(|name| name.to_lowercase());
        fonts.dedup();

        let mut model = App {
            selected_font: String::new(),
            is_collapsed: false,
            fonts,
            render_font_preview: settings.render_font_preview,
            filter_unicode_pages: settings.filter_unicode_pages,
            font_list: None,
            unicode_set: UnicodeSet::new(),
            unicode_grid_view: None,
            unicode_selection: None,
            unicode_set_list: None,
            section_positions: HashMap::new(),
            displayed_ranges: Vec::new(),
            selected_character: None,
            hex_value: String::new(),
            dec_value: String::new(),
            collected_text: String::new(),
            cell_attrs: Rc::new(RefCell::new(gtk4::pango::AttrList::new())),
            block_boundaries: Rc::new(RefCell::new(Vec::new())),
            sticky_header: None,
            character_names: CharacterNames::new(),
            character_name: String::new(),
            hex_entry: None,
            dec_entry: None,
        };

        let widgets = view_output!();

        model.unicode_grid_view = Some(widgets.unicode_grid_view.clone());
        model.unicode_set_list = Some(widgets.unicode_set_list.clone());
        model.sticky_header = Some(widgets.sticky_header.clone());
        model.hex_entry = Some(widgets.hex_entry.clone());
        model.dec_entry = Some(widgets.dec_entry.clone());

        let grid_factory =
            build_unicode_grid_factory(model.cell_attrs.clone(), model.block_boundaries.clone());
        widgets.unicode_grid_view.set_factory(Some(&grid_factory));

        // Use GTK's native single-selection: the GridView highlights the
        // selected cell and handles keyboard navigation itself. The store is
        // (re)built and swapped in by `refresh_unicode_sections` on font change.
        let selection = gtk::SingleSelection::new(None::<gio::ListStore>);
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        widgets.unicode_grid_view.set_model(Some(&selection));
        model.unicode_selection = Some(selection.clone());

        // Selecting a cell (click or keyboard) updates the character preview.
        selection.connect_selection_changed({
            let sender = sender.clone();
            move |selection, _, _| {
                if let Some(ch) = selection
                    .selected_item()
                    .and_then(|item| item.downcast::<gtk::StringObject>().ok())
                    .and_then(|obj| obj.string().chars().next())
                {
                    sender.input(Messages::CharacterSelected(ch as i32));
                }
            }
        });

        // Activating a cell (double-click or Enter) appends it to the text.
        widgets.unicode_grid_view.connect_activate({
            let sender = sender.clone();
            move |grid_view, position| {
                if let Some(ch) = grid_view
                    .model()
                    .and_then(|m| m.item(position))
                    .and_then(|item| item.downcast::<gtk::StringObject>().ok())
                    .and_then(|obj| obj.string().chars().next())
                {
                    sender.input(Messages::CharacterDoubleClicked(ch as i32));
                }
            }
        });

        // Keep the sticky "current block" header in sync with the scroll
        // position (the top-most visible cell's unicode block).
        widgets
            .unicode_scroller
            .vadjustment()
            .connect_value_changed({
                let grid_view = widgets.unicode_grid_view.clone();
                let boundaries = model.block_boundaries.clone();
                let header = widgets.sticky_header.clone();
                move |_adjustment| {
                    update_sticky_header(&grid_view, &boundaries.borrow(), &header);
                }
            });

        for font_name in &model.fonts {
            let label = gtk::Label::new(Some(font_name));
            label.set_xalign(0.0);
            label.set_margin_top(SPACING_SMALL);
            label.set_margin_bottom(SPACING_SMALL);
            label.set_margin_start(SPACING_SMALL);
            label.set_margin_end(SPACING_SMALL);

            apply_font_preview(&label, font_name, model.render_font_preview);

            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&label));
            widgets.font_list.append(&row);
        }

        model.font_list = Some(widgets.font_list.clone());

        if let Some(first_row) = widgets.font_list.row_at_index(0) {
            widgets.font_list.select_row(Some(&first_row));
        }

        let search_filter = Rc::new(RefCell::new(String::new()));
        widgets.font_list.set_filter_func({
            let search_filter = search_filter.clone();
            move |row| {
                let filter = search_filter.borrow();
                if filter.is_empty() {
                    return true;
                }
                row.child()
                    .and_then(|w| w.downcast::<gtk::Label>().ok())
                    .map(|label| label.text().to_lowercase().contains(filter.as_str()))
                    .unwrap_or(true)
            }
        });

        widgets.font_search_entry.connect_search_changed({
            let font_list = widgets.font_list.clone();
            move |entry| {
                *search_filter.borrow_mut() = entry.text().to_lowercase();
                font_list.invalidate_filter();
            }
        });

        // Type-ahead: while the font list has keyboard focus, typing letters
        // accumulates a prefix and jumps the selection to the first (visible)
        // row starting with it. The prefix resets after a short pause between
        // keystrokes (so typing "arial" quickly narrows down to "Arial"
        // instead of restarting on every letter), or immediately if the
        // selection changes some other way (e.g. clicking a different row).
        let type_ahead_buffer = Rc::new(RefCell::new(String::new()));
        let type_ahead_reset: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        // Set for the duration of the `select_row` call inside the type-ahead
        // handler itself, so the `row-selected` listener below (which clears
        // the buffer on any OTHER selection change) knows to leave it alone.
        let type_ahead_selecting = Rc::new(std::cell::Cell::new(false));

        widgets.font_list.connect_row_selected({
            let buffer = type_ahead_buffer.clone();
            let reset_source = type_ahead_reset.clone();
            let selecting = type_ahead_selecting.clone();
            move |_, _| {
                if selecting.get() {
                    return;
                }
                // The selection changed for a reason other than our own
                // type-ahead jump (e.g. a mouse click) -- start the next
                // search from scratch. Only cancel the timeout if it hasn't
                // already fired (it clears itself to `None` when it does),
                // since removing an already-fired source panics.
                if let Some(source) = reset_source.borrow_mut().take() {
                    source.remove();
                }
                buffer.borrow_mut().clear();
            }
        });

        let type_ahead_controller = gtk::EventControllerKey::new();
        type_ahead_controller.connect_key_pressed({
            let font_list = widgets.font_list.clone();
            let buffer = type_ahead_buffer.clone();
            let reset_source = type_ahead_reset.clone();
            let selecting = type_ahead_selecting.clone();
            move |_, keyval, _keycode, state| {
                // Let modified keys (Ctrl/Alt) and non-printable keys (arrows,
                // Tab, Enter, Backspace, ...) fall through to normal handling
                // instead of being captured as search text.
                if state.intersects(
                    gtk4::gdk::ModifierType::CONTROL_MASK | gtk4::gdk::ModifierType::ALT_MASK,
                ) {
                    return glib::Propagation::Proceed;
                }
                let Some(ch) = keyval.to_unicode().filter(|ch| !ch.is_control()) else {
                    return glib::Propagation::Proceed;
                };

                // Cancel the pending reset only if it hasn't already fired
                // (it clears itself to `None` when it does, since removing an
                // already-fired/self source panics).
                if let Some(source) = reset_source.borrow_mut().take() {
                    source.remove();
                }

                buffer.borrow_mut().extend(ch.to_lowercase());

                *reset_source.borrow_mut() = Some(glib::timeout_add_local_once(
                    std::time::Duration::from_millis(1000),
                    {
                        let buffer = buffer.clone();
                        let reset_source = reset_source.clone();
                        move || {
                            buffer.borrow_mut().clear();
                            // The source has now fired (and GLib has already
                            // removed it); forget it so a later keystroke
                            // doesn't try to remove it again and panic.
                            *reset_source.borrow_mut() = None;
                        }
                    },
                ));

                let query = buffer.borrow().clone();
                let mut child = font_list.first_child();
                while let Some(widget) = child {
                    let next = widget.next_sibling();
                    if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>() {
                        // `ListBox`'s filter function toggles `child-visible`,
                        // not the `visible` property itself, so a filtered-out
                        // row still reports `is_visible() == true`. Checking
                        // `is_child_visible()` is what actually reflects
                        // whether the search-box filter currently hides it.
                        if row.is_child_visible() {
                            let matches = row
                                .child()
                                .and_then(|w| w.downcast::<gtk::Label>().ok())
                                .is_some_and(|label| {
                                    label.text().to_lowercase().starts_with(&query)
                                });
                            if matches {
                                selecting.set(true);
                                font_list.select_row(Some(row));
                                selecting.set(false);
                                row.grab_focus();
                                break;
                            }
                        }
                    }
                    child = next;
                }

                glib::Propagation::Stop
            }
        });
        widgets.font_list.add_controller(type_ahead_controller);

        let about_action =
            create_about_action(widgets.main_window.clone(), Self::get_app_version());

        let mut window_actions = RelmActionGroup::<WindowActionGroup>::new();
        window_actions.add_action(about_action);
        window_actions.register_for_widget(&widgets.main_window);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            Messages::FontSelected(font_name) => {
                self.selected_font = font_name;
                self.selected_character = None;
                self.refresh_unicode_sections();
                self.update_character_preview();

                // Clear the grid's visual selection highlight too: swapping
                // the store (or, on the fast "same characters" path, leaving
                // it alone) doesn't reset `SingleSelection`'s selected index
                // on its own, so the previously selected cell could stay
                // highlighted even after `selected_character` above is reset.
                if let Some(selection) = &self.unicode_selection {
                    selection.set_selected(gtk::INVALID_LIST_POSITION);
                }
            }
            Messages::CharacterSelected(char_code) => {
                self.selected_character = char::from_u32(char_code as u32);
                self.update_character_preview();
            }
            Messages::CharacterDoubleClicked(char_code) => {
                if let Some(ch) = char::from_u32(char_code as u32) {
                    self.collected_text.push(ch);
                }
            }
            Messages::ClearCollectedText => {
                self.collected_text.clear();
            }
            Messages::SetCollapsed(is_collapsed) => {
                self.is_collapsed = is_collapsed;
            }
            Messages::SetFilterUnicodePages(enabled) => {
                self.filter_unicode_pages = enabled;
                self.save_config();
                self.refresh_unicode_sections();
            }
            Messages::JumpToUnicodeSet(description) => {
                self.scroll_to_unicode_set(&description);
            }
            Messages::SetFontPreview(enabled) => {
                self.render_font_preview = enabled;
                self.save_config();

                if let Some(font_list) = &self.font_list {
                    let mut child = font_list.first_child();
                    while let Some(row) = child {
                        let next = row.next_sibling();

                        if let Some(label) = row
                            .downcast_ref::<gtk::ListBoxRow>()
                            .and_then(|row| row.child())
                            .and_then(|w| w.downcast::<gtk::Label>().ok())
                        {
                            let font_name = label.text().to_string();
                            apply_font_preview(&label, &font_name, enabled);
                        }

                        child = next;
                    }
                }
            }
            Messages::SetDecValue(dec) => {
                self.dec_value = dec;
            }
            Messages::SetHexValue(hex) => {
                self.hex_value = hex;
            }
            Messages::FindHex => {
                if let Ok(code) = u32::from_str_radix(&self.hex_value, 16) {
                    if let Some(ch) = char::from_u32(code) {}
                }
            }
            Messages::FindDec => {
                if let Ok(code) = self.dec_value.parse::<u32>() {
                    if let Some(ch) = char::from_u32(code) {}
                }
            }
        }
    }
}

fn apply_font_preview(label: &gtk::Label, font_name: &str, enabled: bool) {
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

/// Returns whether the given already-loaded font has a glyph for at least
/// one codepoint in the inclusive `start..=end` range.
fn font_covers_range(font: &gtk4::pango::Font, start: u32, end: u32) -> bool {
    (start..=end).any(|code| {
        char::from_u32(code)
            .filter(|ch| !ch.is_control())
            .is_some_and(|ch| font.has_char(ch))
    })
}

/// Builds the character grid's flat data store for the given (filtered)
/// blocks: a lazy `UnicodeCharModel` that resolves each displayable codepoint
/// on demand instead of eagerly allocating a `GtkStringObject` for all of
/// them up front. Swapped into the grid's `SingleSelection` on every font
/// change.
fn build_unicode_store(sections: &[UnicodeEntry]) -> UnicodeCharModel {
    UnicodeCharModel::new(sections)
}

/// Computes, for the given (filtered) blocks, a map of block description ->
/// flat start position and the sorted list of flat start-position -> block
/// description boundaries. These mirror the filtered order in the grid (covered
/// blocks in ascending order, each contributing its non-control chars) and are
/// derived purely from the section list, without touching the model.
fn compute_positions_boundaries(
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

/// Builds the (created-once, reused-forever) factory that renders each grid
/// cell as a single fixed-size character `Label`. `GtkGridView` handles the
/// column layout and virtualization; each cell just shows one character in
/// the currently selected font (via the shared `cell_attrs`).
fn build_unicode_grid_factory(
    cell_attrs: Rc<RefCell<gtk4::pango::AttrList>>,
    block_boundaries: Rc<RefCell<Vec<(u32, String)>>>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        // GtkInscription (not GtkLabel) renders text in a FIXED area and
        // never resizes to its content. GtkGridView sizes every cell to the
        // largest natural size among realized cells, so a Label whose glyph
        // is wider/taller than CELL_SIZE (color emoji, CJK, fallback fonts)
        // would make every cell grow and the columns reflow while
        // scrolling. Pinning nat-chars/nat-lines to 1 keeps the natural
        // size below the CELL_SIZE request, so all cells stay CELL_SIZE.
        let label = gtk::Inscription::new(None);
        label.set_width_request(CELL_SIZE);
        label.set_height_request(CELL_SIZE);
        label.set_min_chars(1);
        label.set_nat_chars(1);
        label.set_min_lines(1);
        label.set_nat_lines(1);
        label.set_xalign(0.5);
        label.set_yalign(0.5);
        label.set_halign(gtk::Align::Fill);
        label.set_valign(gtk::Align::Center);
        label.set_hexpand(true);
        label.add_css_class("unicode-cell");

        list_item.set_child(Some(&label));
    });

    factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(text) = list_item
            .item()
            .and_then(|item| item.downcast::<gtk::StringObject>().ok())
            .map(|obj| obj.string())
        else {
            return;
        };
        let Some(label) = list_item
            .child()
            .and_then(|w| w.downcast::<gtk::Inscription>().ok())
        else {
            return;
        };

        label.set_text(Some(&text));
        label.set_attributes(Some(&cell_attrs.borrow()));

        // Record this cell's flat position so the sticky-header scroll handler
        // can map the top-visible cell back to its unicode block.
        let position = list_item.position();
        unsafe {
            label.set_data::<u32>(CELL_POSITION_KEY, position);
        }

        // Shade cells by their unicode block's parity so adjacent blocks are
        // visually distinguishable (bands can change mid-row where a block
        // starts, since blocks aren't row-aligned in a flat grid).
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
    });

    factory
}

/// Qdata key under which each grid cell `Inscription` stores its flat model
/// position (a `u32`), used by [`update_sticky_header`].
const CELL_POSITION_KEY: &str = "cell-position";

/// Measures the character grid's ACTUAL layout from its currently-realized
/// cells: the number of columns and the row pitch (vertical distance between
/// consecutive rows, in px). `GtkGridView` derives its column count and adds
/// inter-cell spacing itself, so neither can be reliably guessed from
/// `CELL_SIZE` — they must be read back from real cell geometry. Returns
/// `None` if too few cells are realized to measure.
fn grid_geometry(grid_view: &gtk::GridView) -> Option<(u32, f64)> {
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

/// Recursively collects every character-cell `Inscription` currently realized
/// under the grid view (all `Inscription`s in the subtree are cells).
fn collect_cell_labels(widget: &gtk::Widget, out: &mut Vec<gtk::Inscription>) {
    let mut child = widget.first_child();
    while let Some(w) = child {
        if let Some(label) = w.downcast_ref::<gtk::Inscription>() {
            out.push(label.clone());
        } else {
            collect_cell_labels(&w, out);
        }
        child = w.next_sibling();
    }
}

/// Updates the sticky "current block" header to the unicode block that owns
/// the top-most currently-visible grid cell.
///
/// `GtkGridView` implements `GtkScrollable`, so it is allocated only the
/// viewport size and positions its realized children relative to the visible
/// area — the top edge of the viewport is therefore always `y == 0` in the
/// grid view's own coordinate space (it is NOT the absolute scroll offset).
fn update_sticky_header(
    grid_view: &gtk::GridView,
    boundaries: &[(u32, String)],
    header: &gtk::Label,
) {
    if boundaries.is_empty() {
        return;
    }

    let mut labels = Vec::new();
    collect_cell_labels(grid_view.upcast_ref(), &mut labels);

    // The GridView scrolls its own children, so `compute_point` returns each
    // cell's y RELATIVE TO THE VISIBLE VIEWPORT (top edge = 0). Find the
    // geometrically TOP-MOST cell that is at least half visible and use its
    // model position to resolve the block.
    //
    // Two pitfalls are guarded against here:
    //  * GridView keeps a pool of recycled, *unmapped* cell widgets in its
    //    widget tree; those report stale positions and bogus (0,0) coordinates.
    //    They must be skipped or they hijack the result and show a wrong,
    //    far-earlier block. Hence the `is_mapped()` and viewport-bounds checks.
    //  * Selecting by geometry (smallest y) rather than smallest position
    //    avoids a partially-scrolled-off row (smaller position, above the top)
    //    winning and dragging the header back to the previous block.
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

        // Must be at least half visible from the top (a row almost fully
        // scrolled off can't win) and actually inside the viewport, not in the
        // bottom recycling buffer.
        if y + height * 0.5 <= 0.0 || y >= viewport_height {
            continue;
        }

        let Some(position) =
            (unsafe { label.data::<u32>(CELL_POSITION_KEY) }).map(|ptr| unsafe { *ptr.as_ref() })
        else {
            continue;
        };

        // Pick the top-most cell (smallest y); on ties within the same row,
        // prefer the left-most (smallest position) so a block boundary that
        // straddles a row resolves deterministically.
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

fn bp_with_setters(
    bp: adw::Breakpoint,
    additions: &[(&impl IsA<glib::Object>, &str, impl ToValue)],
) -> adw::Breakpoint {
    bp.add_setters(additions);
    bp
}
