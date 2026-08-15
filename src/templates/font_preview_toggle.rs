// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::WidgetTemplate;

use crate::tr;

// Font renderting switch
#[relm4::widget_template(pub)]
impl WidgetTemplate for FontPreviewToggle {
    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            add_css_class: "linked",

            #[name = "font_preview_off_button"]
            gtk::ToggleButton {
                set_icon_name: "format-text-plaintext-symbolic",
                set_tooltip_text: Some(&tr!("Show plain font names")),
            },
            
            #[name = "font_preview_on_button"]
            gtk::ToggleButton {
                set_icon_name: "format-text-italic-symbolic",
                set_tooltip_text: Some(&tr!("Preview names using each font")),
                set_group: Some(&font_preview_off_button),
            },
        }
    }
}

