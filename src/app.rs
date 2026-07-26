use adw::prelude::*;
use gtk4::{glib, prelude::*};
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
    unicode_container: Option<gtk::Box>,
    unicode_scroller: Option<gtk::ScrolledWindow>,
    unicode_set_list: Option<gtk::ListBox>,
    section_headers: HashMap<String, gtk::Widget>,
    selected_character: Option<char>,
    character_preview: Option<gtk::Label>,
}

#[derive(Debug)]
pub enum Messages {
    FontSelected(String),
    CharacterSelected(i32),
    SetCollapsed(bool),
    SetFontPreview(bool),
    JumpToUnicodeSet(String),
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
    /// rebuilds the character grid and the "Jump to Unicode Set" list.
    fn refresh_unicode_sections(&mut self, sender: ComponentSender<Self>) {
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

        if let Some(container) = &self.unicode_container {
            self.section_headers = populate_unicode_grid(
                container,
                &self.unicode_set.filtered_unicode_sections,
                &font_name,
                &sender,
            );
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

    /// Scrolls the character grid so the header for the given unicode block is visible.
    fn scroll_to_unicode_set(&self, description: &str) {
        let (Some(container), Some(scroller), Some(header)) = (
            &self.unicode_container,
            &self.unicode_scroller,
            self.section_headers.get(description),
        ) else {
            return;
        };

        if let Some(bounds) = header.compute_bounds(container) {
            let adjustment = scroller.vadjustment();
            let max_value = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            let target = (bounds.y() as f64).clamp(adjustment.lower(), max_value);
            adjustment.set_value(target);
        }
    }

    /// Renders the currently selected character, big, in the Character Information box.
    fn update_character_preview(&self) {
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

                                // grid list of characters grouped by unicode blocks
                                #[name = "unicode_scroller"]
                                gtk::ScrolledWindow {
                                    set_vexpand: true,
                                    set_hscrollbar_policy: gtk::PolicyType::Never,

                                    #[name = "unicode_container"]
                                    gtk::Box {
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_margin_start: SPACING_MEDIUM,
                                        set_margin_end: SPACING_MEDIUM,
                                        set_margin_bottom: SPACING_MEDIUM,
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

                                    #[name = "jump_to_set_button"]
                                    gtk::MenuButton {
                                        set_valign: gtk::Align::Start,
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
            provider.load_from_data(
                ".unicode-cell { border-radius: 6px; background-color: alpha(currentColor, 0.08); }",
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
            unicode_container: None,
            unicode_scroller: None,
            unicode_set_list: None,
            section_headers: HashMap::new(),
            selected_character: None,
            character_preview: None,
        };

        let widgets = view_output!();

        model.unicode_container = Some(widgets.unicode_container.clone());
        model.unicode_scroller = Some(widgets.unicode_scroller.clone());
        model.unicode_set_list = Some(widgets.unicode_set_list.clone());
        model.character_preview = Some(widgets.character_preview_label.clone());

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

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            Messages::FontSelected(font_name) => {
                self.selected_font = font_name;
                self.selected_character = None;
                self.refresh_unicode_sections(sender);
                self.update_character_preview();
            }
            Messages::CharacterSelected(char_code) => {
                self.selected_character = char::from_u32(char_code as u32);
                self.update_character_preview();
            }
            Messages::SetCollapsed(is_collapsed) => {
                self.is_collapsed = is_collapsed;
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

/// Rebuilds the character grid inside `container`, one section per unicode
/// block, and returns a map of block description -> section header widget
/// (used later to scroll to a chosen block).
fn populate_unicode_grid(
    container: &gtk::Box,
    sections: &[UnicodeEntry],
    font_name: &str,
    sender: &ComponentSender<App>,
) -> HashMap<String, gtk::Widget> {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let mut headers = HashMap::new();

    let mut font_desc = gtk4::pango::FontDescription::new();
    font_desc.set_family(font_name);
    font_desc.set_size(16 * gtk4::pango::SCALE);
    let attrs = gtk4::pango::AttrList::new();
    attrs.insert(gtk4::pango::AttrFontDesc::new(&font_desc));

    for section in sections {
        let section_box = gtk::Box::new(gtk::Orientation::Vertical, SPACING_SMALL);

        let header = gtk::Label::new(Some(&section.description));
        header.set_xalign(0.0);
        header.add_css_class("heading");
        header.set_margin_top(SPACING_MEDIUM);
        section_box.append(&header);

        let flow_box = gtk::FlowBox::new();
        flow_box.set_selection_mode(gtk::SelectionMode::Single);
        flow_box.set_homogeneous(true);
        flow_box.set_row_spacing(SPACING_SMALL as u32);
        flow_box.set_column_spacing(SPACING_SMALL as u32);
        flow_box.set_min_children_per_line(4);
        flow_box.set_max_children_per_line(24);
        flow_box.set_activate_on_single_click(true);

        flow_box.connect_child_activated({
            let sender = sender.clone();
            move |_, child| {
                if let Some(ch) = child
                    .child()
                    .and_then(|w| w.downcast::<gtk::Label>().ok())
                    .and_then(|label| label.text().chars().next())
                {
                    sender.input(Messages::CharacterSelected(ch as i32));
                }
            }
        });

        for code in section.start_index..=section.end_index {
            if let Some(ch) = char::from_u32(code).filter(|ch| !ch.is_control()) {
                let label = gtk::Label::new(Some(&ch.to_string()));
                label.set_width_request(36);
                label.set_height_request(36);
                label.set_halign(gtk::Align::Center);
                label.set_valign(gtk::Align::Center);
                label.set_justify(gtk::Justification::Center);
                label.add_css_class("unicode-cell");
                label.set_attributes(Some(&attrs));
                flow_box.insert(&label, -1);
            }
        }

        section_box.append(&flow_box);
        container.append(&section_box);

        headers.insert(
            section.description.clone(),
            section_box.upcast::<gtk::Widget>(),
        );
    }

    headers
}

fn bp_with_setters(
    bp: adw::Breakpoint,
    additions: &[(&impl IsA<glib::Object>, &str, impl ToValue)],
) -> adw::Breakpoint {
    bp.add_setters(additions);
    bp
}
