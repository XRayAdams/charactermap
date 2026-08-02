// Copyright (c) 2025, 2026 Konstantin Adamov. Licensed under MIT.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    // Rerun this script if assets change
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=po");
    println!("cargo:rerun-if-changed=resources.gresource.xml");

    let out_dir = env::var("OUT_DIR").unwrap();
    
    
    // Compile the GResource file
    let status = Command::new("glib-compile-resources")
        .arg("resources.gresource.xml")
        .arg("--target")
        .arg(format!("{}/resources.gresource", out_dir))
        .arg("--sourcedir")
        .arg(".")
        .status()
        .expect("Failed to execute glib-compile-resources. Make sure it is installed.");

    if !status.success() {
        panic!("glib-compile-resources failed");
    }
    
    // Navigate up to the target directory (target/debug or target/release)
    // Structure is usually: target/<profile>/build/<package>-<hash>/out
    // So we go up 3 levels to get to target/<profile>
    let dest_path = Path::new(&out_dir)
        .parent().unwrap()
        .parent().unwrap()
        .parent().unwrap();

//    Compile translations
    compile_translations(dest_path);
}

fn compile_translations(dest_path: &Path) {
    let po_dir = Path::new("po");
    if !po_dir.exists() {
        return;
    }
    
    // Find all .po files
    if let Ok(entries) = fs::read_dir(po_dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("po") {
                    if let Some(lang) = path.file_stem().and_then(|s| s.to_str()) {
                        // Skip the template file
                        if lang.ends_with(".pot") || lang == "charactermap" {
                            continue;
                        }
                        
                        compile_po_file(&path, lang, dest_path);
                    }
                }
            }
        }
    }
}

fn compile_po_file(po_file: &Path, lang: &str, dest_path: &Path) {
    let locale_dir = dest_path.join("locale").join(lang).join("LC_MESSAGES");
    fs::create_dir_all(&locale_dir).expect("Failed to create locale directory");
    
    let mo_file = locale_dir.join("charactermap.mo");
    
    println!("Compiling {} translation: {:?} -> {:?}", lang, po_file, mo_file);
    
    // Try to compile using msgfmt
    let status = Command::new("msgfmt")
        .arg("-o")
        .arg(&mo_file)
        .arg(po_file)
        .status();
    
    match status {
        Ok(status) if status.success() => {
            println!("Successfully compiled {} translation", lang);
        }
        Ok(status) => {
            eprintln!("Warning: msgfmt failed with status: {}", status);
            eprintln!("Translation for {} will not be available", lang);
        }
        Err(e) => {
            eprintln!("Warning: Could not run msgfmt: {}", e);
            eprintln!("Please install gettext tools (msgfmt) to enable translations");
            eprintln!("On Ubuntu/Debian: sudo apt install gettext");
            eprintln!("On Fedora: sudo dnf install gettext");
        }
    }
}
