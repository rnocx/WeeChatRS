fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
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
