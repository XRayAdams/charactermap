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
use crate::helpers::static_data::{
    APP_NAME, CELL_SIZE, GRID_FONT_SIZE, LABEL_FONT_SIZE, SPACING_MEDIUM, SPACING_SMALL,
};
use crate::helpers::utils::{
    apply_font_preview, bp_with_setters, build_search_result_store, build_unicode_grid_factory,
    build_unicode_store, collect_cell_labels, compute_positions_boundaries, font_attr_list,
    font_covers_range, grid_geometry, update_sticky_header,
};
use crate::tr;
use crate::unicode::{UnicodeEntry, UnicodeSet, raw_offset_to_filtered_index};
use crate::widgets::{HelpAction, create_help_action};

/// Caps how many characters a name search can display.
const MAX_SEARCH_RESULTS: usize = 500;

#[derive(Serialize, Deserialize)]
struct AppSettings {
    render_font_preview: bool,
    filter_unicode_pages: bool,
}

pub struct App {
    selected_font: String,
    is_collapsed: bool,
    is_search_visible: bool,
    fonts: Vec<String>,
    render_font_preview: bool,
    /// When on, only shows unicode blocks the selected font covers.
    filter_unicode_pages: bool,
    font_list: Option<gtk::ListBox>,
    unicode_set: UnicodeSet,
    unicode_grid_view: Option<gtk::GridView>,
    /// Shared handle to `unicode_grid_view`, so signal closures stay valid
    /// after the REBUILD path replaces the grid view widget.
    grid_view_handle: Rc<RefCell<Option<gtk::GridView>>>,
    /// Parent of `unicode_grid_view`; the REBUILD path swaps the whole
    /// widget here (see `replace_grid_view` for why).
    unicode_scroller: Option<gtk::ScrolledWindow>,
    /// The grid's native single-selection model; its inner store is swapped on
    /// font change. GTK draws the selection highlight and handles keyboard nav.
    unicode_selection: Option<gtk::SingleSelection>,
    unicode_set_list: Option<gtk::ListBox>,
    section_positions: HashMap<String, u32>,
    /// Codepoint ranges currently loaded into the grid model, used to skip
    /// rebuilding when a font change doesn't affect what's shown.
    displayed_ranges: Vec<(u32, u32)>,
    selected_character: Option<char>,
    hex_value: String,
    dec_value: String,
    collected_text: String,
    /// Shared Pango attributes (selected font + size) applied to every grid
    /// cell; updated in place on font change so recycled cells pick it up.
    cell_attrs: Rc<RefCell<gtk4::pango::AttrList>>,
    char_label_attrs: Rc<RefCell<gtk4::pango::AttrList>>,
    /// Flat-position -> unicode-block boundaries, used to resolve the
    /// sticky "current block" header from the top-visible cell.
    block_boundaries: Rc<RefCell<Vec<(u32, String)>>>,
    sticky_header: Option<gtk::Label>,
    character_names: CharacterNames,
    character_name: String,
    hex_entry: Option<gtk::Entry>,
    dec_entry: Option<gtk::Entry>,
    search_entry: Option<gtk::SearchEntry>,
    /// Whether the character grid is currently showing name-search results
    /// instead of the normal font-filtered blocks.
    is_showing_search_results: bool,
    /// Cached browse-grid model/state, so leaving search restores it
    /// instantly instead of recomputing everything.
    browse_store: Option<gio::ListModel>,
    browse_section_positions: HashMap<String, u32>,
    browse_block_boundaries: Vec<(u32, String)>,
    browse_header: String,
    toast_overlay: Option<adw::ToastOverlay>,
}

#[derive(Debug)]
pub enum Messages {
    FontSelected(String),
    CharacterSelected(u32),
    CharacterDoubleClicked(u32),
    ClearCollectedText,
    SetCollapsed(bool),
    SetFontPreview(bool),
    SetFilterUnicodePages(bool),
    JumpToUnicodeSet(String),
    SetHexValue(String),
    SetDecValue(String),
    FindHex,
    FindDec,
    ShowHideSearch,
    SearchChanged(String),
    CopySelectedCharacter,
}

/// Wires "selecting a cell updates the character preview" on a grid
/// selection model (reused for freshly-rebuilt selections).
fn connect_grid_selection_changed(selection: &gtk::SingleSelection, sender: ComponentSender<App>) {
    selection.connect_selection_changed(move |selection, _, _| {
        if let Some(ch) = selection
            .selected_item()
            .and_then(|item| item.downcast::<gtk::StringObject>().ok())
            .and_then(|obj| obj.string().chars().next())
        {
            sender.input(Messages::CharacterSelected(ch as u32));
        }
    });
}

/// Builds a fresh character `GridView` (mirrors the one in `view!`), used
/// by the REBUILD path to replace the whole widget, not just its model.
fn build_unicode_grid_view(sender: ComponentSender<App>) -> gtk::GridView {
    let grid_view = gtk::GridView::builder()
        .min_columns(1)
        .max_columns(100)
        .single_click_activate(false)
        .enable_rubberband(false)
        .build();

    grid_view.connect_activate(move |grid_view, position| {
        if let Some(ch) = grid_view
            .model()
            .and_then(|m| m.item(position))
            .and_then(|item| item.downcast::<gtk::StringObject>().ok())
            .and_then(|obj| obj.string().chars().next())
        {
            sender.input(Messages::CharacterDoubleClicked(ch as u32));
        }
    });

    grid_view
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
    fn refresh_unicode_sections(&mut self, sender: &ComponentSender<Self>) {
        let Some(font_list) = self.font_list.clone() else {
            return;
        };
        // Force a rebuild if search results were showing, even if the
        // filtered ranges themselves are unchanged.
        let restoring_from_search = self.is_showing_search_results;
        self.is_showing_search_results = false;

        let context = font_list.pango_context();
        let font_name = self.selected_font.clone();

        self.unicode_set.filtered_unicode_sections = if self.filter_unicode_pages {
            // Keep only the blocks the selected font actually covers.
            let mut font_desc = gtk4::pango::FontDescription::new();
            font_desc.set_family(&font_name);
            let font = context.load_font(&font_desc);

            let result: Vec<UnicodeEntry> = self
                .unicode_set
                .unicode_sections
                .iter()
                .filter(|entry| {
                    font.as_ref().is_some_and(|font| {
                        font_covers_range(font, entry.start_index, entry.end_index)
                    })
                })
                .cloned()
                .collect();

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

        if let Some(mut grid_view) = self.unicode_grid_view.clone() {
            // Update positions/boundaries BEFORE swapping the model so the
            // factory's `bind` callback resolves the correct block.
            let (positions, boundaries) =
                compute_positions_boundaries(&self.unicode_set.filtered_unicode_sections);

            self.section_positions = positions;

            let first_block = boundaries
                .first()
                .map(|(_, description)| description.clone())
                .unwrap_or_default();
            *self.block_boundaries.borrow_mut() = boundaries;

            // Cache this browse state so leaving search later can restore it
            // by simple assignment, without recomputing any of the above.
            self.browse_section_positions = self.section_positions.clone();
            self.browse_block_boundaries = self.block_boundaries.borrow().clone();
            self.browse_header = first_block.clone();

            // Only rebuild when the visible character set actually changes;
            // otherwise just repaint realized cells in the new font.
            let new_ranges: Vec<(u32, u32)> = self
                .unicode_set
                .filtered_unicode_sections
                .iter()
                .map(|entry| (entry.start_index, entry.end_index))
                .collect();

            if restoring_from_search || new_ranges != self.displayed_ranges {
                self.displayed_ranges = new_ranges;

                let store = build_unicode_store(&self.unicode_set.filtered_unicode_sections);
                let model = store.upcast::<gio::ListModel>();

                self.replace_grid_view(model.clone(), sender);
                self.browse_store = Some(model);
                if let Some(new_grid_view) = self.unicode_grid_view.clone() {
                    grid_view = new_grid_view;
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

    /// Swaps in a new grid model by replacing the whole `GridView` widget
    /// (GTK never shrinks a grown recycled-widget pool otherwise). Used for
    /// font changes and entering/leaving search.
    fn replace_grid_view(&mut self, model: gio::ListModel, sender: &ComponentSender<Self>) {
        let Some(scroller) = self.unicode_scroller.clone() else {
            return;
        };

        let new_selection = gtk::SingleSelection::new(Some(model));
        new_selection.set_autoselect(false);
        new_selection.set_can_unselect(true);
        connect_grid_selection_changed(&new_selection, sender.clone());

        let new_grid_view = build_unicode_grid_view(sender.clone());
        let new_factory =
            build_unicode_grid_factory(self.cell_attrs.clone(), self.block_boundaries.clone());
        new_grid_view.set_factory(Some(&new_factory));
        new_grid_view.set_model(Some(&new_selection));

        scroller.set_child(Some(&new_grid_view));

        self.unicode_selection = Some(new_selection);
        self.unicode_grid_view = Some(new_grid_view.clone());
        *self.grid_view_handle.borrow_mut() = Some(new_grid_view);
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

        // Align the block to the viewport TOP (not just scrolled into view)
        // so the sticky header agrees. Column count/row pitch are measured
        // from realized cells, not derived from CELL_SIZE.
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

        // Set the header directly for immediate feedback; realized cells
        // haven't updated their positions yet at the jump target.
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
                self.dec_entry.as_ref().map(|entry| entry.set_text(""));
                self.hex_entry.as_ref().map(|entry| entry.set_text(""));
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

    /// Switches the grid to show displayable characters (within the current
    /// filtered/covered blocks) whose name matches `query`.
    fn refresh_search_results(&mut self, query: &str, sender: &ComponentSender<Self>) {
        let matches = self.character_names.search(
            query,
            &self.unicode_set.filtered_unicode_sections,
            MAX_SEARCH_RESULTS,
        );

        self.is_showing_search_results = true;
        self.section_positions.clear();
        self.block_boundaries.borrow_mut().clear();

        // The previously selected character no longer has any meaning
        // against the search-result model, so clear its preview.
        self.selected_character = None;
        self.update_character_preview();

        let store = build_search_result_store(&matches);
        let model = store.upcast::<gio::ListModel>();
        self.replace_grid_view(model, sender);

        if let Some(selection) = &self.unicode_selection {
            selection.set_selected(gtk::INVALID_LIST_POSITION);
        }

        if let Some(header) = &self.sticky_header {
            let label = tr!("Search results ({})").replace("{}", &matches.len().to_string());
            header.set_label(&label);
        }

        if let Some(grid_view) = &self.unicode_grid_view {
            grid_view.scroll_to(0, gtk::ListScrollFlags::empty(), None);
        }
    }

    /// Re-runs the active name search after a font or filter change, since
    /// those can change which characters are covered.
    fn rerun_search_if_active(&mut self, sender: &ComponentSender<Self>) {
        if !self.is_search_visible {
            return;
        }

        let query = self
            .search_entry
            .as_ref()
            .map(|entry| entry.text().to_string());
        if let Some(query) = query.filter(|query| query.chars().count() >= 2) {
            self.refresh_search_results(&query, sender);
        }
    }

    /// Restores the grid to what was showing before search, replacing the
    /// widget so search's bloated recycled-widget pool isn't reused.
    fn restore_browse_grid(&mut self, sender: &ComponentSender<Self>) {
        if !self.is_showing_search_results {
            return;
        }
        self.is_showing_search_results = false;

        // Same reasoning as entering search: the selection belonged to the
        // search-result model, not the restored browse model.
        self.selected_character = None;
        self.update_character_preview();

        if let Some(model) = self.browse_store.clone() {
            self.replace_grid_view(model, sender);
        }

        if let Some(selection) = &self.unicode_selection {
            selection.set_selected(gtk::INVALID_LIST_POSITION);
        }

        self.section_positions = self.browse_section_positions.clone();
        *self.block_boundaries.borrow_mut() = self.browse_block_boundaries.clone();

        if let Some(header) = &self.sticky_header {
            header.set_label(&self.browse_header);
        }

        if let Some(grid_view) = &self.unicode_grid_view {
            grid_view.scroll_to(0, gtk::ListScrollFlags::empty(), None);
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
                &tr!("_Help") => HelpAction,
                &tr!("_About") => AboutAction,
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
                        set_title: &tr!("Fonts"),
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
                                        set_placeholder_text: Some(&tr!("Search fonts…")),
                                        set_hexpand: true,
                                    },

                                    // segmented buttons to toggle font preview rendering
                                    gtk::Box {
                                        set_orientation: gtk::Orientation::Horizontal,
                                        add_css_class: "linked",

                                        #[name = "font_preview_off_button"]
                                        gtk::ToggleButton {
                                            set_icon_name: "format-text-plaintext-symbolic",
                                            set_tooltip_text: Some(&tr!("Show plain font names")),
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
                                            set_tooltip_text: Some(&tr!("Preview names using each font")),
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
                                        connect_realize => |listbox| {
                                            listbox.grab_focus();
                                        },
                                    }
                                },

                                // toggle: filter grid to blocks the font covers
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_spacing: SPACING_SMALL,
                                    set_margin_top: SPACING_MEDIUM,

                                    gtk::Label {
                                        set_label: &tr!("Filter Unicode pages"),
                                        set_xalign: 0.0,
                                        set_hexpand: true,
                                    },

                                    gtk::Switch {
                                        set_valign: gtk::Align::Center,
                                        set_tooltip_text: Some(&tr!("Show only Unicode blocks the selected font supports")),
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
                                #[wrap(Some)]
                                set_title_widget = if model.is_search_visible {
                                    #[name = "search_entry"]
                                    gtk::SearchEntry {
                                        set_placeholder_text: Some(&tr!("Single letter or character name")),
                                        set_hexpand: true,
                                        connect_search_changed[sender] => move |entry| {
                                            sender.input(Messages::SearchChanged(entry.text().to_string()));
                                        },
                                    }
                                } else {
                                    adw::WindowTitle {
                                        set_title: APP_NAME,
                                    }
                                },

                                pack_start = &gtk4::Button {
                                    set_icon_name: "sidebar-show-symbolic",
                                    set_can_focus: false,
                                    #[watch]
                                    set_visible: model.is_collapsed,
                                    connect_clicked[split_view] => move |_| {
                                        split_view.set_show_sidebar(true);
                                    },
                                },
                                pack_end = &gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_spacing: SPACING_SMALL,

                                    gtk::Button {
                                        set_icon_name: "search-symbolic",
                                        set_can_focus: false,
                                        connect_clicked[sender] => move |_| {
                                            sender.input(Messages::ShowHideSearch);
                                        },
                                    },
                                    gtk::MenuButton {
                                        set_icon_name: "open-menu-symbolic",
                                        set_menu_model: Some(&main_menu),
                                        set_direction: gtk::ArrowType::Down,
                                        set_can_focus: false,
                                    },

                                },

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

                                // virtualized grid of characters, the
                                // actual GridView child is built in init()
                                // via `replace_grid_view`, same as font changes.
                                #[name = "unicode_scroller"]
                                gtk::ScrolledWindow {
                                    set_vexpand: true,
                                    set_hscrollbar_policy: gtk::PolicyType::Never,

                                    // Ctrl+C anywhere in the grid copies the selected character.
                                    add_controller = gtk::EventControllerKey {
                                        connect_key_pressed[sender] => move |_, keyval, _keycode, state| {
                                            if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
                                                && matches!(keyval, gtk4::gdk::Key::c | gtk4::gdk::Key::C)
                                            {
                                                sender.input(Messages::CopySelectedCharacter);
                                                glib::Propagation::Stop
                                            } else {
                                                glib::Propagation::Proceed
                                            }
                                        }
                                    },
                                },

                                // bottom bar
                                gtk::Frame {
                                    set_margin_start: SPACING_SMALL,
                                    set_margin_end: SPACING_SMALL,
                                    set_margin_top: SPACING_SMALL,
                                    set_margin_bottom: SPACING_SMALL,
                                    set_label: Some(&tr!("Character Information")),

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
                                                set_label: &tr!("Jump to Unicode Set"),
                                                #[watch]
                                                set_sensitive: !model.is_showing_search_results,

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
                                                set_placeholder_text: Some(&tr!("double click character to add here")),
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
                                                    set_label: &tr!("Hex:"),
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
                                                    set_label: &tr!("Find"),
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
                                                    set_label: &tr!("Dec:"),
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
                                                    set_label: &tr!("Find"),
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
            is_search_visible: false,
            fonts,
            render_font_preview: settings.render_font_preview,
            filter_unicode_pages: settings.filter_unicode_pages,
            font_list: None,
            unicode_set: UnicodeSet::new(),
            unicode_grid_view: None,
            grid_view_handle: Rc::new(RefCell::new(None)),
            unicode_scroller: None,
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
            search_entry: None,
            is_showing_search_results: false,
            browse_store: None,
            browse_section_positions: HashMap::new(),
            browse_block_boundaries: Vec::new(),
            browse_header: String::new(),
            toast_overlay: None,
        };

        let widgets = view_output!();

        model.unicode_scroller = Some(widgets.unicode_scroller.clone());
        model.unicode_set_list = Some(widgets.unicode_set_list.clone());
        model.sticky_header = Some(widgets.sticky_header.clone());
        model.hex_entry = Some(widgets.hex_entry.clone());
        model.dec_entry = Some(widgets.dec_entry.clone());
        model.search_entry = Some(widgets.search_entry.clone());
        model.toast_overlay = Some(widgets.toast_overlay.clone());

        // Build the initial GridView through the same path font changes use,
        // instead of duplicating its construction as a one-off view! widget.
        let empty_model = gio::ListStore::new::<gtk::StringObject>().upcast::<gio::ListModel>();
        model.replace_grid_view(empty_model, &sender);

        // Keep the sticky "current block" header in sync with the scroll
        // position (the top-most visible cell's unicode block).
        widgets
            .unicode_scroller
            .vadjustment()
            .connect_value_changed({
                let grid_view_handle = model.grid_view_handle.clone();
                let boundaries = model.block_boundaries.clone();
                let header = widgets.sticky_header.clone();
                move |_adjustment| {
                    let Some(grid_view) = grid_view_handle.borrow().clone() else {
                        return;
                    };
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

        // Type-ahead: typing while the font list has focus accumulates a
        // prefix and jumps to the first matching row; the prefix resets
        // after a pause or on any other selection change.
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
                // Selection changed for a reason other than our type-ahead
                // jump -- reset the search prefix.
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
                // Let modified/non-printable keys fall through to normal
                // handling instead of being captured as search text.
                if state.intersects(
                    gtk4::gdk::ModifierType::CONTROL_MASK | gtk4::gdk::ModifierType::ALT_MASK,
                ) {
                    return glib::Propagation::Proceed;
                }
                let Some(ch) = keyval.to_unicode().filter(|ch| !ch.is_control()) else {
                    return glib::Propagation::Proceed;
                };

                // Cancel the pending reset if it hasn't already fired.
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
                            // Already fired/removed by GLib; forget it.
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
        let help_action = create_help_action(widgets.main_window.clone());

        let mut window_actions = RelmActionGroup::<WindowActionGroup>::new();
        window_actions.add_action(about_action);
        window_actions.add_action(help_action);
        window_actions.register_for_widget(&widgets.main_window);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            Messages::FontSelected(font_name) => {
                self.selected_font = font_name;
                self.selected_character = None;
                self.refresh_unicode_sections(&sender);
                self.update_character_preview();

                // Re-run an active search under the new font.
                self.rerun_search_if_active(&sender);

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
            Messages::CopySelectedCharacter => {
                if let Some(ch) = self.selected_character {
                    if let Some(scroller) = &self.unicode_scroller {
                        scroller.clipboard().set_text(&ch.to_string());
                    }
                    if let Some(overlay) = &self.toast_overlay {
                        overlay.add_toast(adw::Toast::new(&tr!("Copied to clipboard")));
                    }
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
                self.refresh_unicode_sections(&sender);
                self.rerun_search_if_active(&sender);
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
            Messages::ShowHideSearch => {
                self.is_search_visible = !self.is_search_visible;

                if self.is_search_visible {
                    if let Some(search_entry) = &self.search_entry {
                        search_entry.grab_focus();
                    }
                } else if self.is_showing_search_results {
                    self.restore_browse_grid(&sender);
                }
            }
            Messages::SearchChanged(search) => {
                let len = search.chars().count();
                if len <= 1 {
                    if self.is_showing_search_results {
                        self.restore_browse_grid(&sender);
                    }
                    if let Some(code) = search.chars().next().map(|ch| ch as u32) {
                        self.find_char(code);
                    }
                } else {
                    self.refresh_search_results(&search, &sender);
                }
            }
        }
    }
}
