// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

//! The "Help" dialog, opened from the hamburger menu.

use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::AdwDialogExt;
use relm4::actions::RelmAction;

use crate::helpers::actions::WindowActionGroup;
use crate::helpers::static_data::{SPACING_MEDIUM, SPACING_SMALL};
use crate::{N_, tr};

relm4::new_stateless_action!(pub HelpAction, WindowActionGroup, "help");

/// A help section: a heading followed by one or more lines of body text.
/// Each line is kept as its own string so it can be translated individually.
struct HelpSection {
    title: &'static str,
    lines: &'static [&'static str],
}

const HELP_SECTIONS: &[HelpSection] = &[
    HelpSection {
        title: N_!("Font List"),
        lines: &[
            N_!("Use the arrow keys to move the selection."),
            N_!("Start typing to find a font by name."),
        ],
    },
    HelpSection {
        title: N_!("Grid"),
        lines: &[
            N_!("Use the arrow keys to move the selection."),
            N_!("Double-click, or press Enter, to add the selected character to the collection box."),
            N_!("Press Ctrl+C to copy the selected character to the clipboard."),
            N_!("If a character is not present in the selected font, it will be shown as a tofu box with a different background."),
        ],
    },
    HelpSection {
        title: N_!("Hex and Dec Entries"),
        lines: &[N_!("Enter a hex value, a decimal value, or a character, then press Enter to find it.")],
    },
    HelpSection {
        title: N_!("Search"),
        lines: &[
            N_!("Click the search icon to open the search bar."),
            N_!("Type a character's name to find every character whose name contains it."),
            N_!("Type, or paste, a single character to find it directly."),
        ],
    },
];

/// Builds the scrollable body: one heading + bullet-list Label per section.
fn build_content() -> gtk4::Widget {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, SPACING_SMALL);
    content.set_margin_top(SPACING_MEDIUM);
    content.set_margin_bottom(SPACING_MEDIUM);
    content.set_margin_start(SPACING_MEDIUM);
    content.set_margin_end(SPACING_MEDIUM);

    for section in HELP_SECTIONS {
        let heading = gtk4::Label::new(Some(&tr!(section.title)));
        heading.set_xalign(0.0);
        heading.add_css_class("heading");
        heading.set_margin_top(SPACING_SMALL);
        content.append(&heading);

        for line in section.lines {
            let translated_line = tr!(*line);
            let label = gtk4::Label::new(Some(&format!("•\u{a0}{translated_line}")));
            label.set_xalign(0.0);
            label.set_wrap(true);
            content.append(&label);
        }
    }

    gtk4::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build()
        .upcast()
}

pub fn create_help_action(parent: adw::ApplicationWindow) -> RelmAction<HelpAction> {
    RelmAction::<HelpAction>::new_stateless(move |_| {
        let title = tr!("Help");

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(&title, "")));

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&build_content()));

        let dialog = adw::Dialog::builder()
            .title(title.as_str())
            .content_width(420)
            .content_height(480)
            .child(&toolbar_view)
            .can_close(true)
            .build();

        dialog.present(Some(&parent));
    })
}
