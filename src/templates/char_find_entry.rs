// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::WidgetTemplate;

use crate::helpers::static_data::{SPACING_SMALL};
use crate::tr;

// Template to show current selected character as hex or dec and find if user put number
#[relm4::widget_template(pub)]
impl WidgetTemplate for CharFindEntry {
    view! {
        gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: SPACING_SMALL,

                #[name = "label"]
                gtk::Label {
                    set_valign: gtk::Align::Center,
                },

                #[name = "entry"]
                gtk::Entry {
                    set_width_request: 70,
                    set_valign: gtk::Align::Center,
                    set_max_length: 7,
                },

                #[name = "button"]
                gtk::Button {
                    set_label: &tr!("Find"),
                    set_valign: gtk::Align::Center,
                },
            },
        }
    }
