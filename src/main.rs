// Hide the extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod eye;
mod files;
mod globe;
mod keyboard;
mod sound;
mod stats;
mod terminal;
mod theme;
mod widgets;

use eframe::egui;

/// The "D" window/taskbar icon, embedded in the binary.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("embedded icon is a valid PNG")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData { rgba: img.into_raw(), width, height }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let fullscreen = flag("--fullscreen");
    let muted = flag("--mute") || std::env::var_os("DAEMON_MUTE").is_some();
    let theme_name = args
        .iter()
        .position(|a| a == "--theme")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var("DAEMON_THEME").ok())
        .unwrap_or_else(|| "nova".into());
    let theme = theme::Theme::from_name(&theme_name);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DAEMON")
            .with_icon(app_icon())
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([1000.0, 600.0])
            .with_maximized(true)
            .with_fullscreen(fullscreen),
        ..Default::default()
    };

    eframe::run_native(
        "DAEMON",
        options,
        Box::new(move |cc| Ok(Box::new(app::DaemonApp::new(cc, theme, muted)))),
    )
}
