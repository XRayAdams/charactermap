// Copyright (c) 2025, 2026 Konstantin Adamov. Licensed under MIT.

use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};
use std::path::PathBuf;

const GETTEXT_PACKAGE: &str = "charactermap";

pub fn init_i18n() {
    setlocale(LocaleCategory::LcAll, "");

    let locale_dir = find_locale_dir()
        .unwrap_or_else(|| PathBuf::from("/usr/share/locale"));

    bindtextdomain(GETTEXT_PACKAGE, &locale_dir).expect("Failed to bind text domain");
    bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8").expect("Failed to set codeset");
    textdomain(GETTEXT_PACKAGE).expect("Failed to set text domain");
}

fn find_locale_dir() -> Option<PathBuf> {
    // 1. Respect TEXTDOMAINDIR env var (standard override, useful for debugging)
    if let Ok(dir) = std::env::var("TEXTDOMAINDIR") {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            return Some(path);
        }
    }

    // 2. Check SNAP environment
    if let Ok(snap_path) = std::env::var("SNAP") {
        let snap_locale = PathBuf::from(&snap_path).join("usr/share/locale");
        if snap_locale.is_dir() {
            return Some(snap_locale);
        }
    }

    // 3. Check for development build: locale/ directory next to the executable
    //    (target/debug/locale or target/release/locale)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let dev_locale = exe_dir.join("locale");
            if dev_locale.is_dir() {
                return Some(dev_locale);
            }
        }
    }

    // Fall through: caller uses /usr/share/locale (FHS system default)
    None
}

// Macro for marking translatable strings
#[macro_export]
macro_rules! tr {
    ($text:expr) => {
        gettextrs::gettext($text)
    };
}

// Marks a string literal as translatable without translating it immediately
// (e.g. inside a `const` table). xgettext extracts it as a msgid; the actual
// `tr!()` call happens later, at the point the value is displayed.
#[macro_export]
macro_rules! N_ {
    ($text:expr) => {
        $text
    };
}
