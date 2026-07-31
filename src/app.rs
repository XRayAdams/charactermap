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
use std::time::Instant;

use crate::helpers::actions::{AboutAction, WindowActionGroup, create_about_action};
use crate::helpers::character_names::CharacterNames;
use crate::helpers::static_data::{APP_NAME, CELL_SIZE, GRID_FONT_SIZE, LABEL_FONT_SIZE, SPACING_MEDIUM, SPACING_SMALL};
use crate::helpers::utils::{apply_font_preview, bp_with_setters, 
    build_unicode_grid_factory, build_unicode_store, collect_cell_labels, 
    compute_positions_boundaries, font_attr_list, font_covers_range, grid_geometry, update_sticky_header};
use crate::unicode::{UnicodeEntry, UnicodeSet, raw_offset_to_filtered_index};
    

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
    char_label_attrs: Rc<RefCell<gtk4::pango::AttrList>>,
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
            
            let start_time = Instant::now();

            let result: Vec<UnicodeEntry> = self.unicode_set
                .unicode_sections
                .iter()
                .filter(|entry| {
                    font.as_ref().is_some_and(|font| {
                        font_covers_range(font, entry.start_index, entry.end_index)
                    })
                })
                .cloned()
                .collect();
            let elapsed = start_time.elapsed();
            println!(
                "refresh_unicode_sections: font={} filtered {} blocks in {:?}",
                font_name,
                result.len(),
                elapsed
            );

            result
        } else {
            // No coverage filtering: show every block (unsupported glyphs
            // render as tofu boxes, but font switching is near-instant).
            self.unicode_set.unicode_sections.clone()
        };

        // Update the shared cell attributes so freshly-bound cells render in
        // the selected font.
        *self.cell_attrs.borrow_mut() = font_attr_list(&font_name, Some(GRID_FONT_SIZE));
        *self.char_label_attrs.borrow_mut() = font_attr_list(&font_name, Some(LABEL_FONT_SIZE));

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

    /// Scrolls the character grid so the given unicode block
    /// Select first character if no offset was passed
    fn scroll_to_unicode_set(&self, description: &str, offset: Option<u32>) {
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

        // Select character at offset if passed,
        // otherwise select first character of the block
        if let Some(selection) = &self.unicode_selection {
            selection.set_selected(position + offset.unwrap_or(0));
        }
    }

    fn update_character_preview(&mut self) {
        match self.selected_character {
            Some(ch) => {
                self.hex_value = format!("{:X}", ch as u32);
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

    fn find_char(&self, ch: u32) {
        if let (Some(entry), Some(offset)) = self.unicode_set.find_character(ch) {
            let position = raw_offset_to_filtered_index(entry.start_index, entry.end_index, offset);

            if let Some(pos) = position {
                self.scroll_to_unicode_set(&entry.description, Some(pos));
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
                                                    model.cell_attrs.borrow().clone()
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
                                                    connect_activate[sender] => move |_| {
                                                        sender.input(Messages::FindHex);
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
                                                    connect_activate[sender] => move |_| {
                                                        sender.input(Messages::FindDec);
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
                                                    set_attributes: Some(&model.char_label_attrs.borrow()),
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
            char_label_attrs: Rc::new(RefCell::new(gtk4::pango::AttrList::new())),
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
            let label = gtk::Label::builder()
                .label(font_name)
                .xalign(0.0)
                .margin_top(SPACING_SMALL)
                .margin_bottom(SPACING_SMALL)
                .margin_start(SPACING_SMALL)
                .margin_end(SPACING_SMALL)
                .build();

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

                // Clear the grid's visual selection highlight 
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
                self.scroll_to_unicode_set(&description, None);
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
                    self.find_char(code);
                }
            }
            Messages::FindDec => {
                if let Ok(code) = self.dec_value.parse::<u32>() {
                    self.find_char(code);
                }
            }
        }
    }
}

