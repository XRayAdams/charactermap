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
use crate::helpers::static_data::APP_NAME;
use crate::unicode::{UnicodeEntry, UnicodeSet};

const SPACING_MEDIUM: i32 = 12;
const SPACING_LARGE: i32 = 18;
const SPACING_SMALL: i32 = 6;

/// Fixed size of the reusable label pool created per virtualized grid row.
/// The number of these labels actually shown per row is dynamic (see
/// `App::grid_columns`) and adapts to the available width, so this only
/// needs to be a generous upper bound.
const MAX_GRID_COLUMNS: usize = 48;

/// Cell width/height (in pixels) used both to render each character cell and
/// to compute how many columns fit in the available width.
const CELL_SIZE: i32 = 36;

/// Builds a Pango attribute list that selects the given font family, for use
/// with `Label`/`Entry` widgets' `set_attributes`.
fn font_attr_list(font_name: &str) -> gtk4::pango::AttrList {
    let mut font_desc = gtk4::pango::FontDescription::new();
    font_desc.set_family(font_name);
    let attrs = gtk4::pango::AttrList::new();
    attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));
    attrs
}

/// One virtualized row of the character grid: a chunk of characters that
/// belongs to a single unicode block, plus enough info to render/label it.
struct GridRow {
    description: String,
    font_name: String,
    chars: Vec<char>,
    grid_columns: usize,
}

#[derive(Serialize, Deserialize)]
struct AppSettings {
    render_font_preview: bool,
}

pub struct App {
    selected_font: String,
    is_collapsed: bool,
    fonts: Vec<String>,
    render_font_preview: bool,
    font_list: Option<gtk::ListBox>,
    unicode_set: UnicodeSet,
    unicode_grid_view: Option<gtk::ListView>,
    unicode_set_list: Option<gtk::ListBox>,
    section_positions: HashMap<String, u32>,
    selected_character: Option<char>,
    character_preview: Option<gtk::Label>,
    hex_value: String,
    dec_value: String,
    collected_text: String,
    highlighted_char: Rc<RefCell<Option<char>>>,
    grid_columns: usize,
}

#[derive(Debug)]
pub enum Messages {
    FontSelected(String),
    CharacterSelected(i32),
    CharacterDoubleClicked(i32),
    SetCollapsed(bool),
    SetFontPreview(bool),
    JumpToUnicodeSet(String),
    GridWidthChanged(f64),
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

        self.unicode_set.filtered_unicode_sections = self
            .unicode_set
            .unicode_sections
            .iter()
            .filter(|entry| {
                font_supports_range(&context, &font_name, entry.start_index, entry.end_index)
            })
            .cloned()
            .collect();

        if let Some(list_view) = &self.unicode_grid_view {
            let (selection_model, positions) = build_unicode_model(
                &self.unicode_set.filtered_unicode_sections,
                &font_name,
                self.grid_columns,
            );
            list_view.set_model(Some(&selection_model));
            self.section_positions = positions;
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
        let (Some(list_view), Some(&position)) = (
            &self.unicode_grid_view,
            self.section_positions.get(description),
        ) else {
            return;
        };

        list_view.scroll_to(position, gtk::ListScrollFlags::empty(), None);
    }

    /// Renders the currently selected character, big, in the Character Information box.
    fn update_character_preview(&mut self) {
        match self.selected_character {
            Some(ch) => {
                self.hex_value = format!("{:04X}", ch as u32);
                self.dec_value = (ch as u32).to_string();
            }
            None => {
                self.hex_value.clear();
                self.dec_value.clear();
            }
        }

        let Some(label) = &self.character_preview else {
            return;
        };

        match self.selected_character {
            Some(ch) => {
                label.set_label(&ch.to_string());

                let mut font_desc = gtk4::pango::FontDescription::new();
                font_desc.set_family(&self.selected_font);
                font_desc.set_size(48 * gtk4::pango::SCALE);
                let attrs = gtk4::pango::AttrList::new();
                attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));
                label.set_attributes(Some(&attrs));
            }
            None => {
                label.set_label("");
                label.set_attributes(None);
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

                                // virtualized grid list of characters grouped by unicode blocks
                                #[name = "unicode_scroller"]
                                gtk::ScrolledWindow {
                                    set_vexpand: true,
                                    set_hscrollbar_policy: gtk::PolicyType::Automatic,

                                    #[name = "unicode_grid_view"]
                                    gtk::ListView {
                                        set_show_separators: false,
                                    }
                                },

                                // bottom bar
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_spacing: SPACING_SMALL,
                                    set_margin_start: SPACING_MEDIUM,
                                    set_margin_end: SPACING_MEDIUM,
                                    set_margin_top: SPACING_SMALL,
                                    set_margin_bottom: SPACING_MEDIUM,

                                    gtk::Box {
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_spacing: SPACING_SMALL,
                                        set_valign: gtk::Align::Start,

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
                                            #[watch]
                                            set_text: &model.collected_text,
                                            #[watch]
                                            set_attributes: &if model.collected_text.is_empty() {
                                                gtk4::pango::AttrList::new()
                                            } else {
                                                font_attr_list(&model.selected_font)
                                            },
                                        },

                                        gtk::Box {
                                            set_orientation: gtk::Orientation::Horizontal,
                                            set_spacing: SPACING_SMALL,

                                            gtk::Label {
                                                set_label: "Hex:",
                                                set_valign: gtk::Align::Center,
                                            },

                                            gtk::Entry {
                                                set_width_request: 70,
                                                set_valign: gtk::Align::Center,
                                                #[watch]
                                                set_text: &model.hex_value,
                                            },

                                            gtk::Button {
                                                set_label: "Find",
                                                set_valign: gtk::Align::Center,
                                            },
                                        },

                                        gtk::Box {
                                            set_orientation: gtk::Orientation::Horizontal,
                                            set_spacing: SPACING_SMALL,

                                            gtk::Label {
                                                set_label: "Dec:",
                                                set_valign: gtk::Align::Center,
                                            },

                                            gtk::Entry {
                                                set_width_request: 70,
                                                set_valign: gtk::Align::Center,
                                                #[watch]
                                                set_text: &model.dec_value,
                                            },

                                            gtk::Button {
                                                set_label: "Find",
                                                set_valign: gtk::Align::Center,
                                            },
                                        },
                                    },

                                    // spacer pushes the character information panel to the right
                                    gtk::Box {
                                        set_hexpand: true,
                                    },

                                    gtk::Frame {
                                        set_label: Some("Character Information"),

                                        #[wrap(Some)]
                                        set_child = &gtk::Box {
                                            set_orientation: gtk::Orientation::Vertical,
                                            set_margin_start: SPACING_SMALL,
                                            set_margin_end: SPACING_SMALL,
                                            set_margin_top: SPACING_SMALL,
                                            set_margin_bottom: SPACING_SMALL,

                                            #[name = "character_preview_label"]
                                            gtk::Label {
                                                set_width_request: 120,
                                                set_height_request: 120,
                                                add_css_class: "card",
                                                set_justify: gtk::Justification::Center,
                                            },
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
                 .unicode-cell.selected-cell { background-color: alpha(@accent_bg_color, 0.6); color: @accent_fg_color; }",
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
            font_list: None,
            unicode_set: UnicodeSet::new(),
            unicode_grid_view: None,
            unicode_set_list: None,
            section_positions: HashMap::new(),
            selected_character: None,
            character_preview: None,
            hex_value: String::new(),
            dec_value: String::new(),
            collected_text: String::new(),
            highlighted_char: Rc::new(RefCell::new(None)),
            grid_columns: 1,
        };

        let widgets = view_output!();

        model.unicode_grid_view = Some(widgets.unicode_grid_view.clone());
        model.unicode_set_list = Some(widgets.unicode_set_list.clone());
        model.character_preview = Some(widgets.character_preview_label.clone());

        let header_factory = build_unicode_header_factory();
        let grid_factory =
            build_unicode_grid_factory(sender.clone(), model.highlighted_char.clone());
        widgets.unicode_grid_view.set_factory(Some(&grid_factory));
        widgets
            .unicode_grid_view
            .set_header_factory(Some(&header_factory));

        widgets.unicode_scroller.hadjustment().connect_notify_local(
            Some("page-size"),
            {
                let sender = sender.clone();
                move |adjustment, _| {
                    sender.input(Messages::GridWidthChanged(adjustment.page_size()));
                }
            },
        );

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
                *self.highlighted_char.borrow_mut() = None;
                self.refresh_unicode_sections();
                self.update_character_preview();
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
            Messages::SetCollapsed(is_collapsed) => {
                self.is_collapsed = is_collapsed;
            }
            Messages::JumpToUnicodeSet(description) => {
                self.scroll_to_unicode_set(&description);
            }
            Messages::GridWidthChanged(available_width) => {
                let columns = compute_grid_columns(available_width);
                if columns != self.grid_columns {
                    self.grid_columns = columns;
                    self.refresh_unicode_sections();
                }
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

/// Returns whether the given font family has a glyph for at least one
/// codepoint in the inclusive `start..=end` range.
fn font_supports_range(context: &gtk4::pango::Context, font_name: &str, start: u32, end: u32) -> bool {
    let mut font_desc = gtk4::pango::FontDescription::new();
    font_desc.set_family(font_name);

    let Some(font) = context.load_font(&font_desc) else {
        return false;
    };

    (start..=end).any(|code| {
        char::from_u32(code)
            .filter(|ch| !ch.is_control())
            .is_some_and(|ch| font.has_char(ch))
    })
}

/// Computes how many fixed-size character cells fit in the given available
/// width (in pixels), clamped to a sane [1, MAX_GRID_COLUMNS] range.
fn compute_grid_columns(available_width: f64) -> usize {
    if available_width <= 0.0 {
        return 1;
    }

    let cell_span = (CELL_SIZE + SPACING_SMALL) as f64;
    let usable_width = available_width - (2 * SPACING_MEDIUM) as f64;
    let columns = (usable_width / cell_span).floor().max(1.0) as usize;

    columns.clamp(1, MAX_GRID_COLUMNS)
}

/// Builds the virtualized data model backing the character grid: one child
/// `gio::ListStore` per unicode block (each block becomes one "section" once
/// flattened), where each item is a `GridRow` chunk of up to `grid_columns`
/// characters. Returns the model wrapped for `ListView` use, plus a map of
/// block description -> flattened row position (used to scroll to a block).
fn build_unicode_model(
    sections: &[UnicodeEntry],
    font_name: &str,
    grid_columns: usize,
) -> (gtk::NoSelection, HashMap<String, u32>) {
    let outer_store = gio::ListStore::new::<gio::ListStore>();
    let mut positions = HashMap::new();
    let mut position: u32 = 0;
    let grid_columns = grid_columns.max(1);

    for section in sections {
        let chars: Vec<char> = (section.start_index..=section.end_index)
            .filter_map(|code| char::from_u32(code).filter(|ch| !ch.is_control()))
            .collect();

        if chars.is_empty() {
            continue;
        }

        positions.insert(section.description.clone(), position);

        let block_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        for chunk in chars.chunks(grid_columns) {
            let row = GridRow {
                description: section.description.clone(),
                font_name: font_name.to_string(),
                chars: chunk.to_vec(),
                grid_columns,
            };
            block_store.append(&glib::BoxedAnyObject::new(row));
            position += 1;
        }

        outer_store.append(&block_store);
    }

    let flatten_model = gtk::FlattenListModel::new(Some(outer_store));
    let selection_model = gtk::NoSelection::new(Some(flatten_model));

    (selection_model, positions)
}

/// Builds the (created-once, reused-forever) factory that renders each
/// virtualized row as a fixed-size strip of up to `MAX_GRID_COLUMNS` character
/// cells (only as many as the current row's chunk actually uses are filled in).
fn build_unicode_grid_factory(
    sender: ComponentSender<App>,
    highlighted_char: Rc<RefCell<Option<char>>>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let selected_label: Rc<RefCell<Option<gtk::Label>>> = Rc::new(RefCell::new(None));

    factory.connect_setup({
        let highlighted_char = highlighted_char.clone();
        move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let row_box = gtk::Box::new(gtk::Orientation::Horizontal, SPACING_SMALL);
        row_box.set_halign(gtk::Align::Start);
        row_box.set_margin_start(SPACING_MEDIUM);
        row_box.set_margin_end(SPACING_MEDIUM);
        row_box.set_margin_bottom(SPACING_SMALL);

        for _ in 0..MAX_GRID_COLUMNS {
            let label = gtk::Label::new(None);
            label.set_width_request(CELL_SIZE);
            label.set_height_request(CELL_SIZE);
            label.set_halign(gtk::Align::Center);
            label.set_valign(gtk::Align::Center);
            label.set_justify(gtk::Justification::Center);
            label.add_css_class("unicode-cell");

            let gesture = gtk::GestureClick::new();
            gesture.connect_released({
                let sender = sender.clone();
                let label = label.clone();
                let highlighted_char = highlighted_char.clone();
                let selected_label = selected_label.clone();
                move |_, n_press, _, _| {
                    if let Some(ch) = label.text().chars().next() {
                        *highlighted_char.borrow_mut() = Some(ch);

                        if let Some(prev) = selected_label.borrow_mut().take() {
                            prev.remove_css_class("selected-cell");
                        }
                        label.add_css_class("selected-cell");
                        *selected_label.borrow_mut() = Some(label.clone());

                        sender.input(Messages::CharacterSelected(ch as i32));

                        if n_press == 2 {
                            sender.input(Messages::CharacterDoubleClicked(ch as i32));
                        }
                    }
                }
            });
            label.add_controller(gesture);

            row_box.append(&label);
        }

        list_item.set_child(Some(&row_box));
        list_item.set_focusable(false);
        list_item.set_selectable(false);
        list_item.set_activatable(false);
        }
    });

    factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(item) = list_item.item() else {
            return;
        };
        let Ok(boxed) = item.downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let row: std::cell::Ref<GridRow> = boxed.borrow();

        let Some(row_box) = list_item.child().and_then(|w| w.downcast::<gtk::Box>().ok()) else {
            return;
        };

        let mut font_desc = gtk4::pango::FontDescription::new();
        font_desc.set_family(&row.font_name);
        font_desc.set_size(16 * gtk4::pango::SCALE);
        let attrs = gtk4::pango::AttrList::new();
        attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));

        let highlighted = *highlighted_char.borrow();

        let mut child = row_box.first_child();
        let mut index = 0usize;
        while let Some(widget) = child {
            let next = widget.next_sibling();

            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                if index >= row.grid_columns {
                    // Beyond the currently active column count for this
                    // width bucket: collapse entirely so this row's natural
                    // width doesn't balloon out to the full MAX_GRID_COLUMNS
                    // label pool size.
                    label.set_visible(false);
                    label.set_label("");
                    label.set_attributes(None);
                    label.remove_css_class("unicode-cell");
                    label.remove_css_class("selected-cell");
                } else {
                    label.set_visible(true);
                    match row.chars.get(index) {
                        Some(ch) => {
                            label.set_label(&ch.to_string());
                            label.set_attributes(Some(&attrs));
                            label.add_css_class("unicode-cell");
                            if Some(*ch) == highlighted {
                                label.add_css_class("selected-cell");
                            } else {
                                label.remove_css_class("selected-cell");
                            }
                        }
                        None => {
                            label.set_label("");
                            label.set_attributes(None);
                            label.remove_css_class("unicode-cell");
                            label.remove_css_class("selected-cell");
                        }
                    }
                }
            }

            index += 1;
            child = next;
        }
    });

    factory
}

/// Builds the (created-once, reused-forever) factory that renders each
/// section's header (the unicode block description) above its first row.
fn build_unicode_header_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(move |_, list_item| {
        let Some(list_header) = list_item.downcast_ref::<gtk::ListHeader>() else {
            return;
        };

        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.add_css_class("heading");
        label.set_margin_start(SPACING_MEDIUM);
        label.set_margin_end(SPACING_MEDIUM);
        label.set_margin_top(SPACING_MEDIUM);
        label.set_margin_bottom(SPACING_SMALL);

        list_header.set_child(Some(&label));
    });

    factory.connect_bind(move |_, list_item| {
        let Some(list_header) = list_item.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let Some(item) = list_header.item() else {
            return;
        };
        let Ok(boxed) = item.downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let row: std::cell::Ref<GridRow> = boxed.borrow();

        if let Some(label) = list_header
            .child()
            .and_then(|w| w.downcast::<gtk::Label>().ok())
        {
            label.set_label(&row.description);
        }
    });

    factory
}

fn bp_with_setters(
    bp: adw::Breakpoint,
    additions: &[(&impl IsA<glib::Object>, &str, impl ToValue)],
) -> adw::Breakpoint {
    bp.add_setters(additions);
    bp
}
