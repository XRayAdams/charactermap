// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

use libadwaita as adw;
use libadwaita::prelude::AdwDialogExt;
use relm4::actions::{RelmAction};
use super::static_data::APP_ID;

relm4::new_action_group!(pub WindowActionGroup, "win");

relm4::new_stateless_action!(pub AboutAction, WindowActionGroup, "about");


pub fn create_about_action(parent: adw::ApplicationWindow, app_version: &str) -> RelmAction<AboutAction> {
    let app_version = app_version.to_string();
    RelmAction::<AboutAction>::new_stateless(move |_| {
        let dialog = adw::AboutDialog::builder()
            .application_icon(APP_ID)
            .license_type(gtk4::License::MitX11)
            .website("https://github.com/XRayAdams/charactermap")
            .issue_url("https://github.com/XRayAdams/charactermap/issues")
            .application_name("Character Map")
            .version(&app_version)
            .copyright("© 2026 Konstantin Adamov")
            .developers(vec!["Konstantin Adamov"])
            .can_close(true)
            .build();
        dialog.present(Some(&parent));
    })
}
