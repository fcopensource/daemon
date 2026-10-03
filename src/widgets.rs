//! Custom-painted HUD widgets.

use std::collections::VecDeque;

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::stats::HISTORY;
use crate::theme::{lerp_color, Theme};

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

/// Section title: accent tick, label, faint rule, and a right-aligned readout.
pub fn header(ui: &mut Ui, t: &Theme, left: &str, right: &str) {
    ui.add_space(10.0);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    let font = FontId::monospace(10.0);
    let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
    let max_right = ((w / cw) as usize).saturating_sub(left.chars().count() + 6);
    let p = ui.painter();
    p.rect_filled(Rect::from_min_size(rect.left_center() - vec2(0.0, 5.0), vec2(3.0, 10.0)), 1.5, t.accent);
    p.text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, left, FontId::monospace(11.0), t.primary);
    p.text(rect.right_center(), Align2::RIGHT_CENTER, truncate(right, max_right), font, t.alpha(150));
    // Rule that fades from primary to transparent.
    let y = rect.bottom() + 1.0;
    let segs = 24;
    for i in 0..segs {
        let x0 = rect.left() + rect.width() * i as f32 / segs as f32;
        let x1 = rect.left() + rect.width() * (i + 1) as f32 / segs as f32;
        let a = (70.0 * (1.0 - i as f32 / segs as f32)) as u8 + 12;
        p.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.0_f32, t.alpha(a)));
    }
    ui.add_space(6.0);
}

/// "KEY            value" row, value truncated to fit.
pub fn kv(ui: &mut Ui, t: &Theme, key: &str, value: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 17.0), Sense::hover());
    let font = FontId::monospace(11.0);
    let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
    let max = ((w / cw) as usize).saturating_sub(key.chars().count() + 2).max(4);
    let p = ui.painter();
    p.text(rect.left_center(), Align2::LEFT_CENTER, key, font.clone(), t.alpha(130));
    p.text(rect.right_center(), Align2::RIGHT_CENTER, truncate(value, max), font, t.text);
}

/// Area graph of one or more series over the last `HISTORY` samples.
pub fn graph(ui: &mut Ui, t: &Theme, series: &[(&VecDeque<f32>, Color32)], max: f32, height: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, t.alpha(8));
    let grid = Stroke::new(1.0_f32, t.alpha(14));
    for i in 1..4 {
        let y = rect.top() + rect.height() * i as f32 / 4.0;
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
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
                let y = rect.bottom() - 1.0 - (v / max).clamp(0.0, 1.0) * (rect.height() - 6.0);
                pos2(x, y)
            })
            .collect();
        // Translucent fill under the curve, brighter near the line.
        for pt in &pts {
            let h = rect.bottom() - pt.y;
            let mid = pos2(pt.x, pt.y + h * 0.35);
            p.line_segment([*pt, mid], Stroke::new(step + 0.5, color.gamma_multiply(0.18)));
            p.line_segment([mid, pos2(pt.x, rect.bottom())], Stroke::new(step + 0.5, color.gamma_multiply(0.07)));
        }
        p.add(Shape::line(pts.clone(), Stroke::new(4.0_f32, color.gamma_multiply(0.15))));
        p.add(Shape::line(pts.clone(), Stroke::new(1.5_f32, *color)));
        if let Some(last) = pts.last() {
            p.circle_filled(*last, 3.0, *color);
        }
    }
}

/// One rounded vertical bar per CPU core, colored by load.
pub fn core_bars(ui: &mut Ui, t: &Theme, cores: &[f32]) {
    if cores.is_empty() {
        return;
    }
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
    let p = ui.painter();
    let n = cores.len() as f32;
    let gap = 3.0;
    let bw = ((rect.width() - gap * (n - 1.0)) / n).max(1.0);
    for (i, c) in cores.iter().enumerate() {
        let x = rect.left() + i as f32 * (bw + gap);
        p.rect_filled(Rect::from_min_size(pos2(x, rect.top()), vec2(bw, rect.height())), 2.0, t.alpha(18));
        let frac = (c / 100.0).clamp(0.0, 1.0);
        let h = (rect.height() * frac).max(2.0);
        let color = lerp_color(t.primary, t.accent, frac * 1.4);
        p.rect_filled(Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + bw, rect.bottom())), 2.0, color);
    }
}

/// Thin rounded progress bar with a primary → accent gradient.
pub fn bar(ui: &mut Ui, t: &Theme, frac: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 5.0), Sense::hover());
    if frac > 0.9 {
        let p = ui.painter();
        p.rect_filled(rect, rect.height() / 2.0, t.alpha(25));
        p.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width() * frac.min(1.0), rect.height())), rect.height() / 2.0, t.alert);
    } else {
        progress(ui.painter(), rect, t, frac);
    }
}

pub fn progress(p: &egui::Painter, rect: Rect, t: &Theme, frac: f32) {
    p.rect_filled(rect, rect.height() / 2.0, t.alpha(25));
    let frac = frac.clamp(0.0, 1.0);
    let segs = 40;
    let filled_w = rect.width() * frac;
    for i in 0..segs {
        let x0 = rect.left() + filled_w * i as f32 / segs as f32;
        let x1 = rect.left() + filled_w * (i + 1) as f32 / segs as f32;
        let c = lerp_color(t.primary, t.accent, (x1 - rect.left()) / rect.width().max(1.0));
        p.rect_filled(Rect::from_min_max(pos2(x0, rect.top()), pos2(x1 + 0.5, rect.bottom())), 0.0, c);
    }
}

/// Memory as a grid of cells, filled with a primary → accent gradient.
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
        let color = if i < filled { lerp_color(t.primary, t.accent, col as f32 / COLS as f32) } else { t.alpha(22) };
        p.rect_filled(Rect::from_center_size(c, Vec2::splat(cell * 0.6)), 1.5, color);
    }
}

/// Glowing L-shaped brackets on the corners of `rect` (foreground layer).
pub fn corners(ctx: &egui::Context, rect: Rect, color: Color32) {
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("corners")));
    let l = 14.0;
    let s = Stroke::new(2.0_f32, color);
    for (c, dx, dy) in [
        (rect.left_top(), 1.0, 1.0),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        p.line_segment([c, c + vec2(l * dx, 0.0)], s);
        p.line_segment([c, c + vec2(0.0, l * dy)], s);
    }
}

/// Space-dark backdrop with a faint dot grid and two soft color glows.
pub fn backdrop(ctx: &egui::Context, t: &Theme, time: f64) {
    let p = ctx.layer_painter(egui::LayerId::background());
    let screen = ctx.screen_rect();
    p.rect_filled(screen, 0.0, t.bg);
    let drift = (time * 0.05).sin() as f32 * 60.0;
    for (center, color, radius) in [
        (screen.left_top() + vec2(screen.width() * 0.25 + drift, screen.height() * 0.3), t.primary, 520.0),
        (screen.right_bottom() - vec2(screen.width() * 0.2 - drift, screen.height() * 0.25), t.accent, 460.0),
    ] {
        for k in 0..6 {
            let r = radius * (1.0 - k as f32 * 0.15);
            p.circle_filled(center, r, color.gamma_multiply(0.012));
        }
    }
    let spacing = 26.0;
    let dot = t.alpha(16);
    let mut y = screen.top() + spacing / 2.0;
    while y < screen.bottom() {
        let mut x = screen.left() + spacing / 2.0;
        while x < screen.right() {
            p.circle_filled(pos2(x, y), 0.8, dot);
            x += spacing;
        }
        y += spacing;
    }
}

/// Polyline arc from angle `a0` to `a1` (radians).
pub fn arc(p: &egui::Painter, c: Pos2, r: f32, a0: f32, a1: f32, stroke: Stroke) {
    let steps = (((a1 - a0).abs() * r) / 6.0).ceil().max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / steps as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::line(pts, stroke));
}

/// CRT scanlines (retro themes only).
pub fn scanlines(ctx: &egui::Context) {
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("scanlines")));
    let screen = ctx.screen_rect();
    let mut y = screen.top();
    while y < screen.bottom() {
        p.line_segment([pos2(screen.left(), y), pos2(screen.right(), y)], Stroke::new(1.0_f32, Color32::from_black_alpha(45)));
        y += 3.0;
    }
}

/// Text with a periodic chromatic-aberration glitch.
pub fn glitch_text(p: &egui::Painter, pos: Pos2, align: Align2, text: &str, font: FontId, t: &Theme, time: f64) {
    let glitching = (time * 0.4).fract() < 0.04;
    if glitching {
        let dx = ((time * 113.0).sin() * 3.0) as f32;
        p.text(pos + vec2(dx, 0.0), align, text, font.clone(), t.accent.gamma_multiply(0.7));
        p.text(pos - vec2(dx, 0.0), align, text, font.clone(), t.primary.gamma_multiply(0.7));
    }
    p.text(pos, align, text, font, t.text);
}
