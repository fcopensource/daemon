//! A real terminal: spawns the user's shell in a PTY, feeds its output through
//! a VT100/xterm parser, and paints the resulting screen grid with egui.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::theme::Theme;

const FONT_SIZE: f32 = 14.0;
const SCROLLBACK: usize = 5000;

pub struct Terminal {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    rows: u16,
    cols: u16,
    scroll: usize,
}

fn default_shell() -> String {
    if let Ok(s) = std::env::var("DAEMON_SHELL") {
        return s;
    }
    if cfg!(windows) {
        "powershell.exe".into()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
    }
}

impl Terminal {
    pub fn new(ctx: egui::Context, cwd: &Path) -> Result<Self, String> {
        let (rows, cols) = (24, 80);
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let exited = Arc::new(AtomicBool::new(false));

        {
            let parser = parser.clone();
            let writer = writer.clone();
            let exited = exited.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let data = &buf[..n];
                            let dsr = {
                                let mut p = parser.lock().unwrap();
                                p.process(data);
                                // Answer "where is the cursor?" queries (ConPTY sends one on startup).
                                data.windows(4)
                                    .any(|w| w == b"\x1b[6n")
                                    .then(|| p.screen().cursor_position())
                            };
                            if let Some((r, c)) = dsr {
                                let resp = format!("\x1b[{};{}R", r + 1, c + 1);
                                let _ = writer.lock().unwrap().write_all(resp.as_bytes());
                            }
                            ctx.request_repaint();
                        }
                    }
                }
                exited.store(true, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }

        Ok(Self { parser, writer, master: pair.master, _child: child, exited, rows, cols, scroll: 0 })
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn application_cursor(&self) -> bool {
        self.parser.lock().unwrap().screen().application_cursor()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.scroll = 0;
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.lock().unwrap().set_size(rows, cols);
    }

    /// Paints the terminal into all remaining space of `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let font = FontId::monospace(FONT_SIZE);
        let (cw, ch) = ui.fonts(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click());

        let cols = ((rect.width() / cw).floor() as u16).max(10);
        let rows = ((rect.height() / ch).floor() as u16).max(4);
        self.resize(rows, cols);

        if response.hovered() {
            let dy = ui.input(|i| i.raw_scroll_delta.y);
            if dy != 0.0 {
                let mut lines = (dy / ch).round() as i64;
                if lines == 0 {
                    lines = dy.signum() as i64;
                }
                self.scroll = (self.scroll as i64 + lines).max(0) as usize;
            }
        }

        let painter = ui.painter_at(rect);
        let mut parser = self.parser.lock().unwrap();
        parser.set_scrollback(self.scroll);
        let screen = parser.screen();
        self.scroll = screen.scrollback(); // clamped by vt100

        for row in 0..rows {
            let y = rect.top() + row as f32 * ch;
            let mut col = 0;
            while col < cols {
                let Some(first) = screen.cell(row, col) else {
                    col += 1;
                    continue;
                };
                let style = cell_colors(first, t);
                let start = col;
                let mut text = String::new();
                while col < cols {
                    let Some(c) = screen.cell(row, col) else { break };
                    if cell_colors(c, t) != style {
                        break;
                    }
                    if !c.is_wide_continuation() {
                        let s = c.contents();
                        if s.is_empty() {
                            text.push(' ');
                        } else {
                            text.push_str(&s);
                        }
                    }
                    col += 1;
                }
                let x = rect.left() + start as f32 * cw;
                let (fg, bg) = style;
                if let Some(bg) = bg {
                    let w = (col - start) as f32 * cw;
                    painter.rect_filled(Rect::from_min_size(pos2(x, y), vec2(w, ch)), 0.0, bg);
                }
                if !text.trim().is_empty() {
                    painter.text(pos2(x, y), Align2::LEFT_TOP, text, font.clone(), fg);
                }
            }
        }

        // Blinking block cursor.
        let blink_on = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
        if self.scroll == 0 && !screen.hide_cursor() && blink_on && !self.exited() {
            let (cr, cc) = screen.cursor_position();
            let r = Rect::from_min_size(
                pos2(rect.left() + cc as f32 * cw, rect.top() + cr as f32 * ch),
                vec2(cw, ch),
            );
            painter.rect_filled(r, 1.0, t.accent_alpha(190));
        }

        if self.scroll > 0 {
            painter.text(
                rect.right_top() + vec2(-6.0, 4.0),
                Align2::RIGHT_TOP,
                format!("SCROLLBACK -{}", self.scroll),
                FontId::monospace(11.0),
                t.alert,
            );
        }

        if self.exited() {
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(180));
            painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(1.0_f32, t.alert));
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "SHELL EXITED  â€”  PRESS ENTER TO RESTART",
                FontId::monospace(18.0),
                t.alert,
            );
        }
    }
}

/// Translates an egui input event into the bytes a terminal would send.
pub fn event_to_bytes(event: &egui::Event, app_cursor: bool) -> Option<Vec<u8>> {
    use egui::{Event, Key};
    match event {
        Event::Text(t) => Some(t.as_bytes().to_vec()),
        Event::Paste(t) => Some(t.replace("\r\n", "\r").replace('\n', "\r").into_bytes()),
        // On Windows/Linux egui turns Ctrl+C / Ctrl+X into Copy / Cut; a terminal wants
        // the control codes. On macOS those events come from Cmd+C / Cmd+X instead, and
        // Ctrl+C arrives as a normal key press (handled below).
        Event::Copy if !cfg!(target_os = "macos") => Some(vec![0x03]),
        Event::Cut if !cfg!(target_os = "macos") => Some(vec![0x18]),
        Event::Key { key, pressed: true, modifiers, .. } => {
            let arrow = |c: char| {
                if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }
            };
            let s: String = match key {
                Key::Enter => "\r".into(),
                Key::Backspace => "\x7f".into(),
                Key::Tab if modifiers.shift => "\x1b[Z".into(),
                Key::Tab => "\t".into(),
                Key::Escape => "\x1b".into(),
                Key::ArrowUp => arrow('A'),
                Key::ArrowDown => arrow('B'),
                Key::ArrowRight => arrow('C'),
                Key::ArrowLeft => arrow('D'),
                Key::Home => "\x1b[H".into(),
                Key::End => "\x1b[F".into(),
                Key::Insert => "\x1b[2~".into(),
                Key::Delete => "\x1b[3~".into(),
                Key::PageUp => "\x1b[5~".into(),
                Key::PageDown => "\x1b[6~".into(),
                _ if modifiers.ctrl && !modifiers.alt => {
                    // Already delivered as Copy / Cut / Paste events (where Ctrl is the
                    // platform's command key, i.e. not on macOS).
                    if modifiers.command && matches!(key, Key::C | Key::X | Key::V) {
                        return None;
                    }
                    let name = key.name();
                    let ch = name.chars().next()?;
                    if name.len() == 1 && ch.is_ascii_alphabetic() {
                        return Some(vec![(ch.to_ascii_uppercase() as u8) & 0x1f]);
                    }
                    return None;
                }
                _ => return None,
            };
            Some(s.into_bytes())
        }
        _ => None,
    }
}

fn cell_colors(cell: &vt100::Cell, t: &Theme) -> (Color32, Option<Color32>) {
    let mut fg = match cell.fgcolor() {
        vt100::Color::Default => t.text,
        vt100::Color::Idx(i) => ansi(if cell.bold() && i < 8 { i + 8 } else { i }),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    };
    let mut bg = match cell.bgcolor() {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(ansi(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color32::from_rgb(r, g, b)),
    };
    if cell.inverse() {
        let new_fg = bg.unwrap_or(t.bg);
        bg = Some(fg);
        fg = new_fg;
    }
    (fg, bg)
}

/// xterm 256-color palette.
fn ansi(idx: u8) -> Color32 {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16),
        (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
        (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67),
        (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255),
    ];
    match idx {
        0..=15 => {
            let (r, g, b) = BASE[idx as usize];
            Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            let i = idx - 16;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(lv(i / 36), lv((i / 6) % 6), lv(i % 6))
        }
        _ => {
            let g = 8 + (idx - 232) * 10;
            Color32::from_rgb(g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers};

    fn key(k: Key, modifiers: Modifiers) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn text_and_special_keys() {
        assert_eq!(event_to_bytes(&Event::Text("hi".into()), false), Some(b"hi".to_vec()));
        assert_eq!(event_to_bytes(&key(Key::Enter, Modifiers::NONE), false), Some(b"\r".to_vec()));
        assert_eq!(event_to_bytes(&key(Key::Backspace, Modifiers::NONE), false), Some(vec![0x7f]));
        assert_eq!(event_to_bytes(&key(Key::Tab, Modifiers::SHIFT), false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(event_to_bytes(&key(Key::F9, Modifiers::NONE), false), None);
    }

    #[test]
    fn arrows_follow_cursor_mode() {
        assert_eq!(event_to_bytes(&key(Key::ArrowUp, Modifiers::NONE), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(event_to_bytes(&key(Key::ArrowUp, Modifiers::NONE), true), Some(b"\x1bOA".to_vec()));
    }

    #[test]
    fn ctrl_letters_become_control_codes() {
        assert_eq!(event_to_bytes(&key(Key::D, Modifiers::CTRL), false), Some(vec![0x04]));
        assert_eq!(event_to_bytes(&key(Key::L, Modifiers::CTRL), false), Some(vec![0x0c]));
    }

    #[test]
    fn paste_normalizes_newlines() {
        assert_eq!(event_to_bytes(&Event::Paste("a\r\nb\nc".into()), false), Some(b"a\rb\rc".to_vec()));
    }

    #[test]
    fn palette_endpoints() {
        assert_eq!(ansi(0), Color32::from_rgb(0, 0, 0));
        assert_eq!(ansi(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(ansi(232), Color32::from_rgb(8, 8, 8));
    }
}
