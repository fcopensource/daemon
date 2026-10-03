//! Small custom-painted widgets in the eDEX style.

use std::collections::VecDeque;

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::stats::HISTORY;
use crate::theme::Theme;

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{b} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

pub fn fmt_duration(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{d}d {h:02}:{m:02}:{s:02}")
}

/// Section title with an underline, e.g. "CPU USAGE ─────────── 12%".
pub fn header(ui: &mut Ui, t: &Theme, left: &str, right: &str) {
    ui.add_space(8.0);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    let font = FontId::monospace(10.0);
    let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
    let max_right = ((w / cw) as usize).saturating_sub(left.chars().count() + 4);
    let p = ui.painter();
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, t.alpha(110)));
    p.line_segment([rect.left_bottom(), rect.left_bottom() - vec2(0.0, 5.0)], Stroke::new(1.0_f32, t.primary));
    p.line_segment([rect.right_bottom(), rect.right_bottom() - vec2(0.0, 5.0)], Stroke::new(1.0_f32, t.primary));
    p.text(rect.left_center() + vec2(4.0, -1.0), Align2::LEFT_CENTER, left, FontId::monospace(11.0), t.primary);
    p.text(rect.right_center() + vec2(-4.0, -1.0), Align2::RIGHT_CENTER, truncate(right, max_right), font, t.alpha(150));
    ui.add_space(4.0);
}

/// "KEY ............ value" row, value truncated to fit.
pub fn kv(ui: &mut Ui, t: &Theme, key: &str, value: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
    let font = FontId::monospace(11.0);
    let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
    let max = ((w / cw) as usize).saturating_sub(key.chars().count() + 2).max(4);
    let p = ui.painter();
    p.text(rect.left_center(), Align2::LEFT_CENTER, key, font.clone(), t.alpha(140));
    p.text(rect.right_center(), Align2::RIGHT_CENTER, truncate(value, max), font, t.primary);
}

/// Line graph of one or more series over the last `HISTORY` samples.
pub fn graph(ui: &mut Ui, t: &Theme, series: &[(&VecDeque<f32>, Color32)], max: f32, height: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let p = ui.painter();
    p.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, t.alpha(50)));
    let grid = Stroke::new(1.0_f32, t.alpha(18));
    for i in 1..4 {
        let y = rect.top() + rect.height() * i as f32 / 4.0;
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
    }
    for i in 1..12 {
        let x = rect.left() + rect.width() * i as f32 / 12.0;
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], grid);
    }

    let max = max.max(1e-3);
    let step = rect.width() / (HISTORY as f32 - 1.0);
    for (data, color) in series {
        let n = data.len();
        if n < 2 {
            continue;
        }
        let pts: Vec<Pos2> = data
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = rect.right() - (n - 1 - i) as f32 * step;
                let y = rect.bottom() - 1.0 - (v / max).clamp(0.0, 1.0) * (rect.height() - 2.0);
                pos2(x, y)
            })
            .collect();
        p.add(Shape::line(pts, Stroke::new(1.5_f32, *color)));
    }
}

/// One vertical bar per CPU core.
pub fn core_bars(ui: &mut Ui, t: &Theme, cores: &[f32]) {
    if cores.is_empty() {
        return;
    }
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
    let p = ui.painter();
    let n = cores.len() as f32;
    let gap = 2.0;
    let bw = ((rect.width() - gap * (n - 1.0)) / n).max(1.0);
    for (i, c) in cores.iter().enumerate() {
        let x = rect.left() + i as f32 * (bw + gap);
        p.rect_filled(Rect::from_min_size(pos2(x, rect.top()), vec2(bw, rect.height())), 0.0, t.alpha(25));
        let h = rect.height() * (c / 100.0).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + bw, rect.bottom())), 0.0, t.primary);
    }
}

/// Horizontal progress bar.
pub fn bar(ui: &mut Ui, t: &Theme, frac: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 5.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, t.alpha(30));
    let color = if frac > 0.9 { t.alert } else { t.primary };
    let filled = Rect::from_min_size(rect.min, vec2(rect.width() * frac.clamp(0.0, 1.0), rect.height()));
    p.rect_filled(filled, 0.0, color);
}

/// eDEX-style grid of dots, filled proportionally to memory usage.
pub fn mem_grid(ui: &mut Ui, t: &Theme, frac: f32) {
    const COLS: usize = 40;
    const ROWS: usize = 5;
    let w = ui.available_width();
    let cell = w / COLS as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(w, cell * ROWS as f32), Sense::hover());
    let p = ui.painter();
    let filled = (frac.clamp(0.0, 1.0) * (COLS * ROWS) as f32).round() as usize;
    for i in 0..COLS * ROWS {
        let (col, row) = (i / ROWS, i % ROWS);
        let c = pos2(rect.left() + (col as f32 + 0.5) * cell, rect.top() + (row as f32 + 0.5) * cell);
        let color = if i < filled { t.primary } else { t.alpha(35) };
        p.rect_filled(Rect::from_center_size(c, Vec2::splat(cell * 0.55)), 0.0, color);
    }
}

fn hash(mut x: u64) -> u64 {
    // splitmix64
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// CRT scanlines plus a slow-moving refresh band, painted over the whole UI.
pub fn scanlines(ctx: &egui::Context, t: &Theme, time: f64) {
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("scanlines")));
    let screen = ctx.screen_rect();
    let mut y = screen.top();
    while y < screen.bottom() {
        p.line_segment([pos2(screen.left(), y), pos2(screen.right(), y)], Stroke::new(1.0_f32, Color32::from_black_alpha(45)));
        y += 3.0;
    }
    let band_y = screen.top() + ((time * 70.0) as f32 % (screen.height() + 120.0)) - 120.0;
    p.rect_filled(Rect::from_min_size(pos2(screen.left(), band_y), vec2(screen.width(), 120.0)), 0.0, t.alpha(2));
}

/// Text that periodically glitches with red/cyan offset copies.
pub fn glitch_text(p: &egui::Painter, pos: Pos2, align: Align2, text: &str, font: FontId, t: &Theme, time: f64) {
    let glitching = (time * 0.45).fract() < 0.05;
    if glitching {
        let dx = ((time * 113.0).sin() * 4.0) as f32;
        let dy = ((time * 71.0).cos() * 1.5) as f32;
        p.text(pos + vec2(dx, dy), align, text, font.clone(), t.alert.gamma_multiply(0.8));
        p.text(pos - vec2(dx, -dy), align, text, font.clone(), t.accent.gamma_multiply(0.8));
    }
    p.text(pos, align, text, font, t.primary);
}

/// "Digital rain" of falling characters.
pub fn matrix_rain(p: &egui::Painter, rect: Rect, t: &Theme, time: f64) {
    const CHARS: &[u8] = b"0123456789ABCDEF<>/\\|#$%&*+=:;{}[]";
    const TRAIL: u64 = 18;
    let size = 14.0;
    let font = FontId::monospace(size - 1.0);
    let cols = (rect.width() / size) as u64;
    let rows = (rect.height() / size) as u64 + 1;
    let flicker = (time * 6.0) as u64;
    for c in 0..cols {
        let h = hash(c);
        let speed = 8.0 + (h % 100) as f64 / 6.0;
        let head = ((time * speed) as u64 + (h >> 16) % 200) % (rows + TRAIL + 20);
        for k in 0..TRAIL.min(head + 1) {
            let row = head - k;
            if row >= rows {
                continue;
            }
            let ch = CHARS[(hash(c * 7919 + row * 104729 + flicker) % CHARS.len() as u64) as usize] as char;
            let color = if k == 0 {
                Color32::from_rgb(200, 255, 200)
            } else {
                t.alpha((140.0 * (1.0 - k as f32 / TRAIL as f32)) as u8)
            };
            let pos = pos2(rect.left() + c as f32 * size, rect.top() + row as f32 * size);
            p.text(pos, Align2::LEFT_TOP, ch, font.clone(), color);
        }
    }
}
