// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

use libadwaita as adw;
use gtk4::gio::{self, prelude::ApplicationExt};
mod app;
mod helpers;
mod unicode;
mod widgets;
use app::App;
use helpers::static_data::{APP_ID};
use relm4::RelmApp;
mod i18n;

fn main() {
    i18n::init_i18n();
    
    adw::init().expect("Failed to initialize Libadwaita");

    gtk4::init().expect("Failed to initialize GTK");


    let gtk_app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    gtk_app.connect_activate(|_| {
        // Load and register the GResource
        let resources_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/resources.gresource"));
        let resource_data = gtk4::glib::Bytes::from_static(resources_bytes);
        let resource =
            gio::Resource::from_data(&resource_data).expect("Failed to load GResource");
        gio::resources_register(&resource);

        // Add the GResource path to icon theme so AboutDialog can find the icon
        let display = gtk4::gdk::Display::default().expect("Could not get default display.");
        let icon_theme = gtk4::IconTheme::for_display(&display);
        icon_theme.add_resource_path("/app/rayadams/charactermap/assets");
    });

    let app = RelmApp::from_app(gtk_app);
    app.run::<App>(());

}
