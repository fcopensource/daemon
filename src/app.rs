use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Align2, FontId, Margin, RichText, Sense, Stroke};

use crate::files::{FileAction, FileBrowser};
use crate::globe;
use crate::sound::{Rng, Sfx, Sound};
use crate::stats::{self, Shared, Snapshot};
use crate::terminal::{self, Terminal};
use crate::theme::Theme;
use crate::widgets::{self as w, fmt_bytes, fmt_duration, truncate};

const SIDE_WIDTH: f32 = 310.0;

/// Purely cosmetic boot sequence.
const BOOT_LOG: &[&str] = &[
    "DAEMON BIOS v1.0.0  //  build rust-2021  //  github.com/fcopensource/daemon",
    "",
    "[ OK ] Seeding entropy pool from /dev/urandom",
    "[ OK ] Mounting /dev/shadow",
    "[ OK ] Loading modules: netfilter cryptd stealth ghostfs",
    "[ OK ] Spoofing hardware address ......... de:ad:be:ef:13:37",
    "[ OK ] Routing through 7 proxy nodes",
    "[ OK ] Establishing encrypted tunnel ..... AES-256-GCM",
    "[ OK ] Uplink handshake .................. 0x1F3A9C",
    "[ OK ] Bypassing ICE layer 1/3",
    "[ OK ] Bypassing ICE layer 2/3",
    "[ OK ] Bypassing ICE layer 3/3",
    "[WARN] Trace detected — rerouting",
    "[ OK ] Trace evaded",
    "[ OK ] Attaching system probes",
    "[ OK ] Syncing global node map",
    "[ OK ] Spawning shell daemon ............. pid 1337",
    "",
    "root@daemon:~# ./connect --stealth",
];
/// Boot lines revealed per second.
const BOOT_SPEED: f32 = 11.0;
/// Extra ticks (at BOOT_SPEED) the ACCESS GRANTED banner stays up.
const BOOT_HOLD: usize = 20;

pub struct DaemonApp {
    theme: Theme,
    stats: Shared,
    term: Option<Terminal>,
    term_err: Option<String>,
    files: FileBrowser,
    sound: Sound,
    rng: Rng,
    last_key_sound: Instant,
    next_chatter: f64,
    boot_start: Instant,
    boot_lines_played: usize,
    booted: bool,
    ctx: egui::Context,
}

fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn apply_style(ctx: &egui::Context, t: &Theme) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(t.primary);
    v.panel_fill = t.bg;
    v.window_fill = t.bg;
    v.extreme_bg_color = t.bg;
    v.faint_bg_color = t.alpha(12);
    v.selection.bg_fill = t.alpha(70);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.alpha(60));
    style.override_font_id = Some(FontId::monospace(12.0));
    style.spacing.item_spacing = vec2(6.0, 3.0);
    ctx.set_style(style);
}

fn panel_frame(t: &Theme) -> egui::Frame {
    egui::Frame::none()
        .fill(t.bg)
        .inner_margin(Margin::same(10.0))
        .stroke(Stroke::new(1.0_f32, t.alpha(45)))
}

fn key_pressed(events: &[egui::Event], key: egui::Key) -> bool {
    events.iter().any(|e| matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key))
}

impl DaemonApp {
    pub fn new(cc: &eframe::CreationContext<'_>, theme: Theme, muted: bool) -> Self {
        let ctx = cc.egui_ctx.clone();
        apply_style(&ctx, &theme);
        // `with_maximized` on the builder is not always honored on Windows; ask again at runtime.
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        let start_dir = home_dir();
        let (term, term_err) = match Terminal::new(ctx.clone(), &start_dir) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            | 1;
        Self {
            theme,
            stats: stats::spawn(ctx.clone()),
            term,
            term_err,
            files: FileBrowser::new(start_dir),
            sound: Sound::new(muted),
            rng: Rng(seed),
            last_key_sound: Instant::now(),
            next_chatter: 8.0,
            boot_start: Instant::now(),
            boot_lines_played: 0,
            booted: false,
            ctx,
        }
    }

    fn restart_terminal(&mut self) {
        match Terminal::new(self.ctx.clone(), &self.files.cwd) {
            Ok(t) => {
                self.term = Some(t);
                self.term_err = None;
            }
            Err(e) => {
                self.term = None;
                self.term_err = Some(e);
            }
        }
    }

    fn handle_input(&mut self, events: &[egui::Event]) {
        let enter = key_pressed(events, egui::Key::Enter);
        let dead = self.term.as_ref().map_or(true, |t| t.exited());
        if dead {
            if enter {
                self.sound.play(Sfx::Granted);
                self.restart_terminal();
            }
            return;
        }
        let Some(term) = self.term.as_mut() else { return };
        let app_cursor = term.application_cursor();
        for e in events {
            let Some(bytes) = terminal::event_to_bytes(e, app_cursor) else { continue };
            term.write(&bytes);
            if bytes == b"\r" {
                self.sound.play(Sfx::Enter);
            } else if self.last_key_sound.elapsed() > Duration::from_millis(25) {
                self.sound.play(Sfx::Key);
                self.last_key_sound = Instant::now();
            }
        }
    }

    fn boot_screen(&mut self, ctx: &egui::Context, events: &[egui::Event]) {
        let skip = events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: true, .. }
            )
        });
        let shown = (self.boot_start.elapsed().as_secs_f32() * BOOT_SPEED) as usize;
        if skip || shown > BOOT_LOG.len() + BOOT_HOLD {
            self.booted = true;
            ctx.request_repaint();
            return;
        }

        // Sounds: a blip per new log line, a fanfare when access is granted.
        while self.boot_lines_played < shown.min(BOOT_LOG.len() + 1) {
            if self.boot_lines_played == BOOT_LOG.len() {
                self.sound.play(Sfx::Granted);
            } else if !BOOT_LOG[self.boot_lines_played].is_empty() {
                self.sound.play(Sfx::Blip);
            }
            self.boot_lines_played += 1;
        }

        let t = self.theme;
        let time = ctx.input(|i| i.time);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(t.bg).inner_margin(Margin::same(24.0)))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                {
                    let p = ui.painter();
                    w::matrix_rain(p, full.expand(24.0), &t, time);
                    // Dim the rain behind the log text.
                    p.rect_filled(
                        egui::Rect::from_min_size(full.min, vec2(full.width().min(760.0), full.height())),
                        0.0,
                        egui::Color32::from_black_alpha(200),
                    );
                }
                for line in BOOT_LOG.iter().take(shown) {
                    let color = if line.starts_with("[WARN]") { t.alert } else { t.alpha(230) };
                    ui.label(RichText::new(*line).font(FontId::monospace(14.0)).color(color));
                }
                if shown > BOOT_LOG.len() {
                    let p = ui.painter();
                    let banner = egui::Rect::from_center_size(full.center(), vec2(620.0, 190.0));
                    p.rect_filled(banner, 0.0, egui::Color32::from_black_alpha(235));
                    p.rect_stroke(banner, 0.0, Stroke::new(2.0_f32, t.primary));
                    w::glitch_text(p, banner.center() - vec2(0.0, 28.0), Align2::CENTER_CENTER, "DAEMON", FontId::monospace(80.0), &t, time * 3.0);
                    let blink = (time * 3.0) as i64 % 2 == 0;
                    if blink {
                        p.text(banner.center() + vec2(0.0, 52.0), Align2::CENTER_CENTER, "[ ACCESS GRANTED ]", FontId::monospace(22.0), t.primary);
                    }
                }
                ui.painter().text(full.right_bottom(), Align2::RIGHT_BOTTOM, "press any key to skip", FontId::monospace(11.0), t.alpha(110));
            });
        w::scanlines(ctx, &t, time);
        ctx.request_repaint();
    }

    fn top_bar(&self, ui: &mut egui::Ui, s: &Snapshot, time: f64) {
        let t = &self.theme;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(70.0, 18.0), Sense::hover());
            w::glitch_text(ui.painter(), rect.left_center(), Align2::LEFT_CENTER, "DAEMON", FontId::monospace(15.0), t, time);
            ui.label(RichText::new("// ROOT ACCESS TERMINAL").color(t.alpha(110)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
                ui.label(RichText::new(format!("root@{}  [{user}]", s.hostname)).color(t.alpha(170)));
                let dot = if (time * 1.5) as i64 % 2 == 0 { "●" } else { "○" };
                ui.label(RichText::new(format!("{dot} SECURE LINK")).color(t.accent));
                if self.sound.muted {
                    ui.label(RichText::new("MUTED").color(t.alert));
                }
            });
        });
    }

    fn left_panel(&self, ui: &mut egui::Ui, s: &Snapshot) {
        let t = &self.theme;
        let now = chrono::Local::now();
        ui.label(RichText::new(now.format("%H:%M:%S").to_string()).font(FontId::monospace(44.0)).color(t.primary));
        ui.label(RichText::new(now.format("%A %d %B %Y").to_string().to_uppercase()).color(t.alpha(160)));

        w::header(ui, t, "SYSTEM", &s.hostname);
        w::kv(ui, t, "OS", &s.os);
        w::kv(ui, t, "KERNEL", &s.kernel);
        w::kv(ui, t, "UPTIME", &fmt_duration(s.uptime));
        w::kv(ui, t, "CPU", &s.cpu_brand);

        w::header(ui, t, "CPU USAGE", &format!("{} CORES  {:>3.0}%", s.cores.len(), s.cpu_total));
        w::graph(ui, t, &[(&s.cpu_history, t.primary)], 100.0, 60.0);
        ui.add_space(4.0);
        w::core_bars(ui, t, &s.cores);

        let mem_frac = if s.mem_total > 0 { s.mem_used as f32 / s.mem_total as f32 } else { 0.0 };
        w::header(ui, t, "MEMORY", &format!("{} / {}", fmt_bytes(s.mem_used), fmt_bytes(s.mem_total)));
        w::mem_grid(ui, t, mem_frac);
        ui.add_space(2.0);
        w::kv(ui, t, "SWAP", &format!("{} / {}", fmt_bytes(s.swap_used), fmt_bytes(s.swap_total)));

        w::header(ui, t, "TOP PROCESSES", &format!("{} RUNNING", s.proc_count));
        egui::Grid::new("procs").num_columns(4).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
            for h in ["PID", "NAME", "CPU", "MEM"] {
                ui.label(RichText::new(h).color(t.alpha(120)).size(10.0));
            }
            ui.end_row();
            for p in &s.procs {
                ui.label(RichText::new(p.pid.to_string()).size(10.0).color(t.alpha(170)));
                ui.label(RichText::new(truncate(&p.name, 16)).size(10.0));
                let cpu_color = if p.cpu > 50.0 { t.alert } else { t.primary };
                ui.label(RichText::new(format!("{:.1}%", p.cpu)).size(10.0).color(cpu_color));
                ui.label(RichText::new(fmt_bytes(p.mem)).size(10.0));
                ui.end_row();
            }
        });
    }

    fn right_panel(&self, ui: &mut egui::Ui, s: &Snapshot, time: f64) {
        let t = &self.theme;
        let online = s.rx_total + s.tx_total > 0;

        w::header(ui, t, "GLOBAL NETWORK", if online { "TRACKING" } else { "NO SIGNAL" });
        globe::globe(ui, t, time, 250.0);

        w::header(ui, t, "NETWORK STATUS", &s.net_iface);
        w::kv(ui, t, "STATE", if online { "ONLINE" } else { "OFFLINE" });
        w::kv(ui, t, "DOWNLOAD", &format!("{}/s", fmt_bytes(s.rx_rate)));
        w::kv(ui, t, "UPLOAD", &format!("{}/s", fmt_bytes(s.tx_rate)));
        let max = s.rx_history.iter().chain(s.tx_history.iter()).cloned().fold(1024.0_f32, f32::max);
        ui.add_space(4.0);
        w::graph(ui, t, &[(&s.rx_history, t.primary), (&s.tx_history, t.accent)], max * 1.1, 70.0);
        ui.add_space(4.0);
        w::kv(ui, t, "TOTAL RX", &fmt_bytes(s.rx_total));
        w::kv(ui, t, "TOTAL TX", &fmt_bytes(s.tx_total));

        w::header(ui, t, "STORAGE", &format!("{} VOLUMES", s.disks.len()));
        for d in &s.disks {
            let frac = if d.total > 0 { d.used as f32 / d.total as f32 } else { 0.0 };
            let label = if d.name.is_empty() { d.mount.clone() } else { format!("{} {}", d.mount, d.name) };
            w::kv(ui, t, &truncate(&label, 18), &format!("{} / {}", fmt_bytes(d.used), fmt_bytes(d.total)));
            w::bar(ui, t, frac);
            ui.add_space(4.0);
        }

        w::header(ui, t, "CONTROLS", "");
        w::kv(ui, t, "THEME", &t.name.to_uppercase());
        w::kv(ui, t, "SOUND [F10]", if self.sound.muted { "OFF" } else { "ON" });
        w::kv(ui, t, "FULLSCREEN", "F11");
        w::kv(ui, t, "SCROLLBACK", "MOUSE WHEEL");
    }
}

impl eframe::App for DaemonApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep keyboard focus away from egui widgets: every key belongs to the terminal.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });

        let events = ctx.input(|i| i.events.clone());
        if key_pressed(&events, egui::Key::F11) {
            let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
        }
        if key_pressed(&events, egui::Key::F10) {
            self.sound.muted = !self.sound.muted;
        }

        if !self.booted {
            self.boot_screen(ctx, &events);
            return;
        }

        self.handle_input(&events);
        let snap = self.stats.lock().unwrap().clone();
        let t = self.theme;
        let time = ctx.input(|i| i.time);

        // Ambient "data chatter" every few seconds.
        if time > self.next_chatter {
            self.sound.play(Sfx::Chatter);
            self.next_chatter = time + 6.0 + self.rng.next_f32() as f64 * 12.0;
        }

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(t.bg).inner_margin(Margin::symmetric(10.0, 4.0)))
            .show(ctx, |ui| self.top_bar(ui, &snap, time));

        egui::SidePanel::left("left")
            .exact_width(SIDE_WIDTH)
            .resizable(false)
            .frame(panel_frame(&t))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().id_salt("left_scroll").show(ui, |ui| self.left_panel(ui, &snap));
            });

        egui::SidePanel::right("right")
            .exact_width(SIDE_WIDTH)
            .resizable(false)
            .frame(panel_frame(&t))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().id_salt("right_scroll").show(ui, |ui| self.right_panel(ui, &snap, time));
            });

        egui::TopBottomPanel::bottom("files")
            .exact_height(210.0)
            .resizable(false)
            .frame(panel_frame(&t))
            .show(ctx, |ui| {
                w::header(ui, &t, "FILESYSTEM", &self.files.cwd.display().to_string());
                if let Some(action) = self.files.show(ui, &t) {
                    self.sound.play(Sfx::Click);
                    let cmd = match action {
                        FileAction::Cd(p) => format!("cd \"{}\"\r", p.display()),
                        FileAction::Insert(p) => format!("\"{}\" ", p.display()),
                    };
                    if let Some(term) = self.term.as_mut() {
                        term.write(cmd.as_bytes());
                    }
                }
            });

        egui::CentralPanel::default().frame(panel_frame(&t)).show(ctx, |ui| {
            w::header(ui, &t, "TERMINAL", "ROOT SHELL // TTY0");
            ui.add_space(4.0);
            match (self.term.as_mut(), &self.term_err) {
                (Some(term), _) => term.show(ui, &t),
                (None, err) => {
                    ui.colored_label(t.alert, format!(
                        "FAILED TO START SHELL: {}\n\nPress ENTER to retry. Set DAEMON_SHELL to choose a different shell.",
                        err.as_deref().unwrap_or("unknown error")
                    ));
                }
            }
        });

        w::scanlines(ctx, &t, time);

        // Globe, scanlines and cursor are animated: ~30 fps.
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
