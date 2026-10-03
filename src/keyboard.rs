//! Small translucent on-screen keyboard. Keys light up when pressed on the
//! physical keyboard, and clicking a key types it into the terminal.

use std::collections::HashMap;

use eframe::egui::{self, pos2, vec2, Align2, FontId, Rect, Sense, Stroke};

use crate::theme::{lerp_color, Theme};

#[derive(Clone, Copy)]
enum Kind {
    /// Character key: (normal, shifted).
    Char(char, char),
    /// Sends a fixed byte sequence.
    Bytes(&'static str),
    Shift,
    Ctrl,
    Alt,
    Caps,
}

struct K {
    label: &'static str,
    /// Width in key units.
    w: f32,
    kind: Kind,
}

const fn c(label: &'static str, lo: char, up: char) -> K {
    K { label, w: 1.0, kind: Kind::Char(lo, up) }
}

const fn b(label: &'static str, w: f32, bytes: &'static str) -> K {
    K { label, w, kind: Kind::Bytes(bytes) }
}

const fn m(label: &'static str, w: f32, kind: Kind) -> K {
    K { label, w, kind }
}

/// Every row is 15 units wide.
const ROWS: [&[K]; 5] = [
    &[
        b("ESC", 1.0, "\x1b"), c("1", '1', '!'), c("2", '2', '@'), c("3", '3', '#'), c("4", '4', '$'),
        c("5", '5', '%'), c("6", '6', '^'), c("7", '7', '&'), c("8", '8', '*'), c("9", '9', '('),
        c("0", '0', ')'), c("-", '-', '_'), c("=", '=', '+'), b("BKSP", 2.0, "\x7f"),
    ],
    &[
        b("TAB", 1.5, "\t"), c("Q", 'q', 'Q'), c("W", 'w', 'W'), c("E", 'e', 'E'), c("R", 'r', 'R'),
        c("T", 't', 'T'), c("Y", 'y', 'Y'), c("U", 'u', 'U'), c("I", 'i', 'I'), c("O", 'o', 'O'),
        c("P", 'p', 'P'), c("[", '[', '{'), c("]", ']', '}'), K { label: "\\", w: 1.5, kind: Kind::Char('\\', '|') },
    ],
    &[
        m("CAPS", 1.75, Kind::Caps), c("A", 'a', 'A'), c("S", 's', 'S'), c("D", 'd', 'D'), c("F", 'f', 'F'),
        c("G", 'g', 'G'), c("H", 'h', 'H'), c("J", 'j', 'J'), c("K", 'k', 'K'), c("L", 'l', 'L'),
        c(";", ';', ':'), c("'", '\'', '"'), b("ENTER", 2.25, "\r"),
    ],
    &[
        m("SHIFT", 2.25, Kind::Shift), c("Z", 'z', 'Z'), c("X", 'x', 'X'), c("C", 'c', 'C'), c("V", 'v', 'V'),
        c("B", 'b', 'B'), c("N", 'n', 'N'), c("M", 'm', 'M'), c(",", ',', '<'), c(".", '.', '>'),
        c("/", '/', '?'), m("SHIFT", 2.75, Kind::Shift),
    ],
    &[
        m("CTRL", 1.5, Kind::Ctrl), m("ALT", 1.25, Kind::Alt), K { label: "SPACE", w: 7.25, kind: Kind::Char(' ', ' ') },
        b("←", 1.25, "\x1b[D"), b("↑", 1.25, "\x1b[A"), b("↓", 1.25, "\x1b[B"), b("→", 1.25, "\x1b[C"),
    ],
];

#[derive(Default)]
pub struct Keyboard {
    /// Per-label highlight, decays to 0.
    glow: HashMap<&'static str, f32>,
    shift: bool,
    ctrl: bool,
    alt: bool,
    caps: bool,
}

fn special_label(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::Enter => "ENTER",
        Key::Backspace => "BKSP",
        Key::Tab => "TAB",
        Key::Escape => "ESC",
        Key::Space => "SPACE",
        Key::ArrowLeft => "←",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowRight => "→",
        _ => return None,
    })
}

fn char_label(ch: char) -> Option<&'static str> {
    ROWS.iter().flat_map(|r| r.iter()).find_map(|k| match k.kind {
        Kind::Char(lo, up) if lo == ch || up == ch => Some(k.label),
        _ => None,
    })
}

impl Keyboard {
    fn flash(&mut self, label: &'static str) {
        self.glow.insert(label, 1.0);
    }

    /// Lights up the keys typed on the physical keyboard.
    pub fn observe(&mut self, events: &[egui::Event]) {
        for e in events {
            match e {
                egui::Event::Text(s) => {
                    for ch in s.chars() {
                        if let Some(l) = char_label(ch) {
                            self.flash(l);
                        }
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if let Some(l) = special_label(*key) {
                        self.flash(l);
                    } else if modifiers.ctrl {
                        // Ctrl+letter produces no text event; light the letter anyway.
                        if let Some(l) = key.name().chars().next().filter(|c| c.is_ascii_alphabetic()).and_then(char_label) {
                            self.flash(l);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn press(&mut self, kind: Kind) -> Option<Vec<u8>> {
        let out = match kind {
            Kind::Char(lo, up) => {
                let upper = if lo.is_ascii_alphabetic() { self.shift ^ self.caps } else { self.shift };
                let ch = if upper { up } else { lo };
                if self.ctrl && lo.is_ascii_alphabetic() {
                    vec![(lo.to_ascii_uppercase() as u8) & 0x1f]
                } else {
                    let mut bytes = ch.to_string().into_bytes();
                    if self.alt {
                        bytes.insert(0, 0x1b);
                    }
                    bytes
                }
            }
            Kind::Bytes(s) => s.as_bytes().to_vec(),
            Kind::Shift => {
                self.shift = !self.shift;
                return None;
            }
            Kind::Ctrl => {
                self.ctrl = !self.ctrl;
                return None;
            }
            Kind::Alt => {
                self.alt = !self.alt;
                return None;
            }
            Kind::Caps => {
                self.caps = !self.caps;
                return None;
            }
        };
        // Shift / Ctrl / Alt are one-shot when clicked.
        self.shift = false;
        self.ctrl = false;
        self.alt = false;
        Some(out)
    }

    /// Draws the keyboard centered in a strip of the given height and returns
    /// the bytes of a clicked key, if any.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &Theme, height: f32) -> Option<Vec<u8>> {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        for g in self.glow.values_mut() {
            *g = (*g - dt * 3.0).max(0.0);
        }
        let held = ui.input(|i| i.modifiers);

        let w = ui.available_width();
        let (strip, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
        let unit = ((height - 12.0) / 5.0).min(w * 0.72 / 15.0);
        let size = vec2(unit * 15.0, unit * 5.0);
        let plate = Rect::from_center_size(strip.center(), size + vec2(12.0, 12.0));
        let p = ui.painter();
        p.rect_filled(plate, 10.0, t.alpha(6));
        p.rect_stroke(plate, 10.0, Stroke::new(1.0_f32, t.alpha(28)));

        let gap = unit * 0.12;
        let mut out = None;
        let mut y = plate.top() + 6.0;
        for (r, row) in ROWS.iter().enumerate() {
            let mut x = plate.left() + 6.0;
            for (i, k) in row.iter().enumerate() {
                let rect = Rect::from_min_size(pos2(x + gap / 2.0, y + gap / 2.0), vec2(k.w * unit - gap, unit - gap));
                let resp = ui.interact(rect, ui.id().with(("osk", r, i)), Sense::click());
                let latched = match k.kind {
                    Kind::Shift => self.shift || held.shift,
                    Kind::Ctrl => self.ctrl || held.ctrl,
                    Kind::Alt => self.alt || held.alt,
                    Kind::Caps => self.caps,
                    _ => false,
                };
                let mut g = self.glow.get(k.label).copied().unwrap_or(0.0);
                if latched {
                    g = g.max(0.8);
                }
                if resp.hovered() {
                    g = g.max(0.3);
                }

                let p = ui.painter();
                p.rect_filled(rect, 4.0, lerp_color(t.bg, t.primary, 0.04 + g * 0.3).gamma_multiply(0.55 + g * 0.3));
                p.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, t.alpha((40.0 + g * 180.0) as u8)));
                if g > 0.4 {
                    p.rect_stroke(rect.expand(1.5), 5.0, Stroke::new(2.0_f32, t.accent_alpha((g * 160.0) as u8)));
                }
                let font = if k.label.chars().count() > 1 { unit * 0.27 } else { unit * 0.4 };
                let col = lerp_color(t.primary, t.accent, g).gamma_multiply(0.55 + g * 0.45);
                p.text(rect.center(), Align2::CENTER_CENTER, k.label, FontId::monospace(font), col);

                if resp.clicked() {
                    self.flash(k.label);
                    if let Some(bytes) = self.press(k.kind) {
                        out = Some(bytes);
                    }
                }
                x += k.w * unit;
            }
            y += unit;
        }
        out
    }
}
