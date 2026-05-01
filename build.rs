fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // Point at the mingw windres/ar when cross-compiling from Linux.
        let host = std::env::var("HOST").unwrap_or_default();
        if std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() == "x86_64"
            && (host.contains("linux") || host.contains("darwin"))
        {
            res.set_windres_path("x86_64-w64-mingw32-windres");
            res.set_ar_path("x86_64-w64-mingw32-ar");
        }
        res.compile().unwrap();
    }
}
