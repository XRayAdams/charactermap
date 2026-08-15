// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::WidgetTemplate;

use crate::helpers::static_data::{SPACING_SMALL, SPACING_MEDIUM};
use crate::tr;

// Enables filtering selected font's supported Unicode blocks 
#[relm4::widget_template(pub)]
impl WidgetTemplate for FilterSwitchRow {
    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: SPACING_SMALL,
            set_margin_top: SPACING_MEDIUM,

            gtk::Label {
                set_xalign: 0.0,
                set_hexpand: true,
                set_label: &tr!("Filter Unicode pages"),
            },

            #[name = "switch"]
            gtk::Switch {
                set_valign: gtk::Align::Center,
                set_tooltip_text: Some(&tr!("Show only Unicode blocks the selected font supports")),
            },
        }
    }
}
