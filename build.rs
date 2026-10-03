fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // Embed the "D" icon into daemon.exe so Explorer and the taskbar show it.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the app icon: {e}");
        }
    }
}
