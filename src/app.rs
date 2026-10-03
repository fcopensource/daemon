use std::f32::consts::{FRAC_PI_2, TAU};
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
    "QUANTUM CORE ............... SYNCHRONIZED",
    "ENTROPY POOL ............... 4096 QBITS",
    "NEURAL INTERFACE ........... CALIBRATED",
    "BIOMETRIC SIGNATURE ........ VERIFIED",
    "HOLO-RENDER PIPELINE ....... ONLINE",
    "MESH UPLINK ................ 7 RELAYS",
    "ENCRYPTION ................. POST-QUANTUM / KYBER-1024",
    "INTRUSION SHIELD ........... ARMED",
    "ORBITAL NODE MAP ........... LOCKED",
    "SYSTEM PROBES .............. ATTACHED",
    "SHELL DAEMON ............... SPAWNED",
    "ALL SUBSYSTEMS ............. NOMINAL",
];
/// Boot lines revealed per second.
const BOOT_SPEED: f32 = 6.0;
/// Extra ticks (at BOOT_SPEED) the welcome screen stays up.
const BOOT_HOLD: usize = 12;

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
    v.override_text_color = Some(t.text);
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

/// Transparent outer frame for a panel: leaves a gap around the glass panel inside it.
fn gap_frame() -> egui::Frame {
    egui::Frame::none().inner_margin(Margin::same(5.0))
}

/// Floating translucent "glass" panel that fills the space it is given.
fn glass<R>(ui: &mut egui::Ui, t: &Theme, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(t.panel)
        .rounding(10.0)
        .inner_margin(Margin::same(12.0))
        .stroke(Stroke::new(1.0_f32, t.alpha(40)))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            w::corners(ui.ctx(), ui.max_rect().expand(12.0), t.accent_alpha(170));
            add(ui)
        })
        .inner
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
        let elapsed = self.boot_start.elapsed().as_secs_f32();
        let shown = (elapsed * BOOT_SPEED) as usize;
        if skip || shown > BOOT_LOG.len() + BOOT_HOLD {
            self.booted = true;
            ctx.request_repaint();
            return;
        }

        // Sounds: a blip per subsystem, a power-up when the link is established.
        while self.boot_lines_played < shown.min(BOOT_LOG.len() + 1) {
            if self.boot_lines_played == BOOT_LOG.len() {
                self.sound.play(Sfx::Granted);
            } else {
                self.sound.play(Sfx::Blip);
            }
            self.boot_lines_played += 1;
        }

        let t = self.theme;
        let time = ctx.input(|i| i.time);
        let tf = time as f32;
        let progress = (elapsed * BOOT_SPEED / BOOT_LOG.len() as f32).min(1.0);
        let done = shown >= BOOT_LOG.len();
        let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
        w::backdrop(ctx, &t, time);

        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            let full = ui.max_rect().shrink(30.0);
            let p = ui.painter();
            let c = full.center() - vec2(0.0, 50.0);

            // Scanner: counter-rotating segmented rings around a progress arc.
            let rings: [(f32, f32, usize, f32); 4] = [(150.0, 0.6, 3, 2.0), (172.0, -0.35, 6, 1.0), (196.0, 0.22, 2, 3.0), (222.0, -0.12, 10, 1.0)];
            for (i, &(r, speed, segs, width)) in rings.iter().enumerate() {
                let color = if i % 2 == 0 { t.primary } else { t.accent };
                let n = segs as f32;
                for k in 0..segs {
                    let a0 = tf * speed + k as f32 * TAU / n;
                    w::arc(p, c, r, a0, a0 + TAU / n * 0.55, Stroke::new(width, color.gamma_multiply(0.85)));
                }
            }
            p.circle_filled(c, 128.0, t.alpha(10));
            p.circle_stroke(c, 128.0, Stroke::new(1.0_f32, t.alpha(50)));
            w::arc(p, c, 128.0, -FRAC_PI_2, -FRAC_PI_2 + TAU * progress, Stroke::new(4.0_f32, t.accent));

            if done {
                w::glitch_text(p, c, Align2::CENTER_CENTER, "DAEMON", FontId::monospace(44.0), &t, time * 2.0);
                p.text(c + vec2(0.0, 40.0), Align2::CENTER_CENTER, "NEURAL LINK ESTABLISHED", FontId::monospace(11.0), t.accent);
            } else {
                p.text(c, Align2::CENTER_CENTER, format!("{:>3.0}%", progress * 100.0), FontId::monospace(44.0), t.text);
                p.text(c + vec2(0.0, 40.0), Align2::CENTER_CENTER, "INITIALIZING", FontId::monospace(11.0), t.alpha(160));
            }

            // Status line and progress bar under the scanner.
            let bar = egui::Rect::from_center_size(c + vec2(0.0, 290.0), vec2(460.0, 4.0));
            w::progress(p, bar, &t, progress);
            let status = if done {
                format!("WELCOME BACK, {}", user.to_uppercase())
            } else {
                BOOT_LOG[shown.min(BOOT_LOG.len() - 1)].to_string()
            };
            p.text(bar.center_top() - vec2(0.0, 12.0), Align2::CENTER_BOTTOM, status, FontId::monospace(13.0), t.text);

            // Recent subsystem log, bottom-left.
            let visible = &BOOT_LOG[shown.min(BOOT_LOG.len()).saturating_sub(8)..shown.min(BOOT_LOG.len())];
            for (j, line) in visible.iter().rev().enumerate() {
                let y = full.bottom() - j as f32 * 18.0;
                let a = 200u8.saturating_sub(j as u8 * 22);
                p.text(egui::pos2(full.left(), y), Align2::LEFT_BOTTOM, format!("› {line}"), FontId::monospace(11.0), t.alpha(a));
            }

            p.text(full.left_top(), Align2::LEFT_TOP, "DAEMON OS  //  v2050.1", FontId::monospace(12.0), t.alpha(170));
            p.text(
                full.right_top(),
                Align2::RIGHT_TOP,
                chrono::Local::now().format("%Y.%m.%d  %H:%M:%S").to_string(),
                FontId::monospace(12.0),
                t.alpha(170),
            );
            p.text(full.right_bottom(), Align2::RIGHT_BOTTOM, "press any key to skip", FontId::monospace(11.0), t.alpha(110));
        });
        ctx.request_repaint();
    }

    fn top_bar(&self, ui: &mut egui::Ui, s: &Snapshot, time: f64) {
        let t = &self.theme;
        let pill = |ui: &mut egui::Ui, text: String, c: egui::Color32| {
            ui.label(RichText::new(format!("  {text}  ")).color(c).background_color(c.gamma_multiply(0.14)).size(11.0));
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(96.0, 22.0), Sense::hover());
            let p = ui.painter();
            let logo = rect.left_center() + vec2(8.0, 0.0);
            p.circle_stroke(logo, 7.0, Stroke::new(1.5_f32, t.accent));
            p.circle_filled(logo, 2.5, t.primary);
            w::glitch_text(p, rect.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, "DAEMON", FontId::monospace(15.0), t, time);
            ui.label(RichText::new("NEURAL OPERATING INTERFACE  ·  v2050.1").color(t.alpha(120)).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
                ui.label(RichText::new(format!("{user}@{}", s.hostname)).color(t.alpha(170)).size(11.0));
                let dot = if (time * 1.5) as i64 % 2 == 0 { "●" } else { "○" };
                pill(ui, format!("{dot} SECURE LINK"), t.accent);
                if !self.sound.available() {
                    pill(ui, "NO AUDIO".into(), t.alert);
                } else if self.sound.muted() {
                    pill(ui, "SOUND OFF  [F10]".into(), t.alert);
                } else {
                    pill(ui, "SOUND ON".into(), t.primary);
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
        w::kv(ui, t, "SOUND [F10]", if self.sound.muted() { "OFF" } else { "ON" });
        w::kv(ui, t, "CUSTOM SOUNDS", &self.sound.custom_count().to_string());
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
            self.sound.toggle_mute();
        }

        if !self.booted {
            self.boot_screen(ctx, &events);
            return;
        }

        self.handle_input(&events);
        let snap = self.stats.lock().unwrap().clone();
        let t = self.theme;
        let time = ctx.input(|i| i.time);

        w::backdrop(ctx, &t, time);

        // Ambient "data chatter" every few seconds.
        if time > self.next_chatter {
            self.sound.play(Sfx::Chatter);
            self.next_chatter = time + 6.0 + self.rng.next_f32() as f64 * 12.0;
        }

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().inner_margin(Margin::symmetric(16.0, 8.0)))
            .show(ctx, |ui| self.top_bar(ui, &snap, time));

        let hidden = egui::scroll_area::ScrollBarVisibility::AlwaysHidden;

        egui::SidePanel::left("left")
            .exact_width(SIDE_WIDTH)
            .resizable(false)
            .frame(gap_frame())
            .show(ctx, |ui| {
                glass(ui, &t, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("left_scroll")
                        .scroll_bar_visibility(hidden)
                        .show(ui, |ui| self.left_panel(ui, &snap));
                })
            });

        egui::SidePanel::right("right")
            .exact_width(SIDE_WIDTH)
            .resizable(false)
            .frame(gap_frame())
            .show(ctx, |ui| {
                glass(ui, &t, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("right_scroll")
                        .scroll_bar_visibility(hidden)
                        .show(ui, |ui| self.right_panel(ui, &snap, time));
                })
            });

        egui::TopBottomPanel::bottom("files")
            .exact_height(220.0)
            .resizable(false)
            .frame(gap_frame())
            .show(ctx, |ui| {
                glass(ui, &t, |ui| {
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
                })
            });

        egui::CentralPanel::default().frame(gap_frame()).show(ctx, |ui| {
            glass(ui, &t, |ui| {
                w::header(ui, &t, "TERMINAL", "NEURAL SHELL  //  TTY0");
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
            })
        });

        if t.retro {
            w::scanlines(ctx);
        }

        // Globe, scanlines and cursor are animated: ~30 fps.
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
