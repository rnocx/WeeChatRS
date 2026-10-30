fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    bundle_themes();

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        let host = std::env::var("HOST").unwrap_or_default();
        let cross = host.contains("linux") || host.contains("darwin");
        if cross {
            match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str() {
                "x86_64" => {
                    res.set_windres_path("x86_64-w64-mingw32-windres");
                    res.set_ar_path("x86_64-w64-mingw32-ar");
                }
                "aarch64" => {
                    res.set_windres_path("aarch64-w64-mingw32-windres");
                    res.set_ar_path("aarch64-w64-mingw32-ar");
                }
                _ => {}
            }
        }
        res.compile().unwrap();
    }
}

/// Embed every Ghostty theme in assets/themes as `(name, contents)`, sorted by name.
fn bundle_themes() {
    println!("cargo:rerun-if-changed=assets/themes");
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/themes");
    let mut themes: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| entries.filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name != "LICENSE" && !name.starts_with('.'))
            .collect())
        .unwrap_or_default();
    themes.sort_by_key(|name| name.to_lowercase());

    // Paths relative to the crate, resolved when compiling: an absolute path from
    // this build would point at a checkout that may be gone when another one reuses
    // the target directory.
    let mut out = String::from("pub(crate) static BUNDLED_THEMES: &[(&str, &str)] = &[\n");
    for name in &themes {
        out.push_str(&format!(
            "    ({:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/themes/\", {:?}))),\n",
            name, name,
        ));
    }
    out.push_str("];\n");
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("ghostty_themes.rs");
    std::fs::write(dest, out).unwrap();
}
