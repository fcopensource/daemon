//! The eye of the daemon: a shaded, slit-pupiled eye built from egui meshes.
//!
//! Layers (back to front): sclera with radial shading, veins, iris with
//! gradient and striations, slit pupil, specular highlights, eyelid shadow,
//! eyelid masks, glowing spiked lid edges.

use std::f32::consts::TAU;

use eframe::egui::{self, pos2, vec2, Color32, Mesh, Pos2, Shape, Stroke, Vec2};

const IRIS_CORE: Color32 = Color32::from_rgb(255, 232, 140);
const IRIS_MID: Color32 = Color32::from_rgb(255, 112, 24);
const IRIS_EDGE: Color32 = Color32::from_rgb(140, 8, 0);
const SCLERA_CENTER: Color32 = Color32::from_rgb(78, 10, 8);
const SCLERA_EDGE: Color32 = Color32::from_rgb(10, 0, 0);
const LID: Color32 = Color32::from_rgb(255, 64, 40);

/// Points along the lid curves.
const N: usize = 48;

pub struct EyeState {
    /// 0 = closed, 1 = fully open.
    pub openness: f32,
    /// Gaze direction, each axis in -1..1.
    pub look: Vec2,
    /// Pupil width multiplier (1 = normal).
    pub dilation: f32,
}

fn rgba(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Lid profile: 1 in the middle, 0 at the corners.
fn profile(u: f32) -> f32 {
    (1.0 - u * u).max(0.0).powf(0.8)
}

/// Filled disc with a radial color gradient. `stops` are (radius fraction, color).
fn radial_disc(p: &egui::Painter, c: Pos2, rx: f32, ry: f32, stops: &[(f32, Color32)]) {
    const SEG: usize = 48;
    let mut m = Mesh::default();
    m.colored_vertex(c, stops[0].1);
    for &(f, col) in &stops[1..] {
        for i in 0..SEG {
            let a = i as f32 / SEG as f32 * TAU;
            m.colored_vertex(c + vec2(a.cos() * rx * f, a.sin() * ry * f), col);
        }
    }
    let ring = |r: usize, i: usize| (1 + r * SEG + i % SEG) as u32;
    for i in 0..SEG {
        m.add_triangle(0, ring(0, i), ring(0, i + 1));
    }
    for r in 0..stops.len() - 2 {
        for i in 0..SEG {
            m.add_triangle(ring(r, i), ring(r + 1, i), ring(r + 1, i + 1));
            m.add_triangle(ring(r, i), ring(r + 1, i + 1), ring(r, i + 1));
        }
    }
    p.add(Shape::mesh(m));
}

/// Draws the eye centered at `c`. `mask` must match the background behind it.
pub fn eye(p: &egui::Painter, c: Pos2, half_w: f32, half_h: f32, s: &EyeState, mask: Color32, time: f64) {
    let open = s.openness.clamp(0.0, 1.0);
    let tf = time as f32;
    // Stroke widths are tuned for half_w ≈ 110 px; scale them for smaller eyes.
    let k = (half_w / 110.0).clamp(0.15, 2.0);
    let xs: Vec<f32> = (0..=N).map(|i| -1.0 + 2.0 * i as f32 / N as f32).collect();
    let upper = |u: f32| pos2(c.x + u * half_w, c.y - half_h * open * profile(u) - 0.10 * half_h * u * open);
    let lower = |u: f32| pos2(c.x + u * half_w, c.y + half_h * 0.82 * open * profile(u) - 0.10 * half_h * u * open);

    // Backing disc, so the lid masks blend into the background.
    p.circle_filled(c, half_w * 1.18, mask);

    if open > 0.02 {
        // Sclera: fan from the center with dark edges for a rounded look.
        let mut m = Mesh::default();
        m.colored_vertex(c, SCLERA_CENTER);
        for &u in &xs {
            m.colored_vertex(upper(u), SCLERA_EDGE);
        }
        for &u in xs.iter().rev() {
            m.colored_vertex(lower(u), SCLERA_EDGE);
        }
        let ring = 2 * (N as u32 + 1);
        for i in 0..ring {
            m.add_triangle(0, 1 + i, 1 + (i + 1) % ring);
        }
        p.add(Shape::mesh(m));

        // Veins creeping from the corners toward the iris.
        for v in 0..8 {
            let u = if v % 2 == 0 { -0.85 + v as f32 * 0.04 } else { 0.85 - v as f32 * 0.04 };
            let start = if v < 4 { upper(u) } else { lower(u) };
            let start = start + (c - start) * 0.08;
            let target = c + (start - c) * 0.35;
            let pts: Vec<Pos2> = (0..=10)
                .map(|j| {
                    let f = j as f32 / 10.0;
                    let q = start + (target - start) * f;
                    q + vec2(0.0, (f * 9.0 + v as f32).sin() * half_h * 0.05)
                })
                .collect();
            p.add(Shape::line(pts, Stroke::new((1.2 * k).max(0.4), Color32::from_rgba_unmultiplied(190, 20, 10, 120))));
        }

        // Iris, following the gaze.
        let r = half_h * 0.9;
        let ic = c + vec2(s.look.x.clamp(-1.0, 1.0) * half_w * 0.33, s.look.y.clamp(-1.0, 1.0) * half_h * 0.15);
        p.circle_filled(ic, r * 1.25, Color32::from_rgba_unmultiplied(255, 60, 0, 25));
        radial_disc(p, ic, r, r, &[(0.0, IRIS_CORE), (0.3, IRIS_CORE), (0.62, IRIS_MID), (1.0, IRIS_EDGE)]);
        // Fibrous striations, slowly turning.
        for i in 0..40 {
            let a = i as f32 / 40.0 * TAU + tf * 0.05;
            let dir = vec2(a.cos(), a.sin());
            let col = if i % 2 == 0 {
                Color32::from_rgba_unmultiplied(60, 0, 0, 90)
            } else {
                Color32::from_rgba_unmultiplied(255, 220, 120, 45)
            };
            p.line_segment([ic + dir * r * 0.3, ic + dir * r * 0.95], Stroke::new(k.max(0.4), col));
        }
        p.circle_stroke(ic, r, Stroke::new(r * 0.09, Color32::from_rgba_unmultiplied(20, 0, 0, 200)));

        // Vertical slit pupil with a hot rim.
        let pw = r * 0.16 * s.dilation;
        let ph = r * 0.86;
        let slit: Vec<Pos2> = (0..32)
            .map(|i| {
                let a = i as f32 / 32.0 * TAU;
                ic + vec2(a.cos() * pw * (1.0 - 0.35 * a.sin().abs()), a.sin() * ph)
            })
            .collect();
        p.add(Shape::convex_polygon(slit, Color32::BLACK, Stroke::new((1.5 * k).max(0.4), Color32::from_rgba_unmultiplied(255, 90, 0, 140))));

        // Wet highlights: they stay put while the iris moves, which reads as a curved surface.
        let hl = c + vec2(-half_w * 0.12, -half_h * 0.32 * open);
        radial_disc(p, hl, r * 0.22, r * 0.13, &[(0.0, rgba(Color32::WHITE, 210)), (0.6, rgba(Color32::WHITE, 120)), (1.0, rgba(Color32::WHITE, 0))]);
        p.circle_filled(c + vec2(half_w * 0.18, half_h * 0.28 * open), r * 0.06, rgba(Color32::WHITE, 120));

        // Shadow cast by the upper lid.
        let mut m = Mesh::default();
        for &u in &xs {
            let top = upper(u);
            m.colored_vertex(top, Color32::from_black_alpha(210));
            m.colored_vertex(top + vec2(0.0, half_h * 0.45 * open * profile(u)), Color32::TRANSPARENT);
        }
        for i in 0..N as u32 {
            let (a, b) = (2 * i, 2 * i + 2);
            m.add_triangle(a, a + 1, b);
            m.add_triangle(a + 1, b + 1, b);
        }
        p.add(Shape::mesh(m));
    }

    // Lid masks: hide everything between the lid curves and the eye's bounding box.
    let reach = half_h * 1.3;
    let mut m = Mesh::default();
    for &u in &xs {
        let (up, lo) = (upper(u), lower(u));
        m.colored_vertex(pos2(up.x, c.y - reach), mask);
        m.colored_vertex(up, mask);
        m.colored_vertex(lo, mask);
        m.colored_vertex(pos2(lo.x, c.y + reach), mask);
    }
    for i in 0..N as u32 {
        let (a, b) = (4 * i, 4 * i + 4);
        for (x, y) in [(0, 1), (2, 3)] {
            m.add_triangle(a + x, a + y, b + x);
            m.add_triangle(a + y, b + y, b + x);
        }
    }
    p.add(Shape::mesh(m));

    // Glowing lid edges.
    let up_pts: Vec<Pos2> = xs.iter().map(|&u| upper(u)).collect();
    let lo_pts: Vec<Pos2> = xs.iter().map(|&u| lower(u)).collect();
    let pulse = 0.75 + 0.25 * (tf * 2.2).sin();
    for (w, a) in [(9.0, 25.0), (5.0, 60.0), (2.0, 255.0)] {
        let col = rgba(LID, (a * pulse) as u8);
        p.add(Shape::line(up_pts.clone(), Stroke::new((w * k).max(0.8), col)));
        p.add(Shape::line(lo_pts.clone(), Stroke::new((w * 0.7 * k).max(0.6), col)));
    }

    // Spikes on the upper lid, raised as the eye opens.
    for j in 0..7 {
        let u = -0.75 + j as f32 * 0.25;
        let base = upper(u);
        let len = half_h * (0.12 + 0.2 * open) * (1.0 - u.abs() * 0.5);
        let tip = base + vec2(u * len * 0.6, -len);
        p.line_segment([base, tip], Stroke::new((2.0 * k).max(0.8), rgba(LID, (200.0 * pulse) as u8)));
    }

    // Outer aura.
    for j in 0..4 {
        p.circle_stroke(c, half_w * (1.05 + j as f32 * 0.06), Stroke::new((2.0 * k).max(0.5), rgba(LID, (18.0 * pulse) as u8)));
    }
    if open <= 0.02 {
        // A burning seam while closed.
        p.line_segment([pos2(c.x - half_w, c.y), pos2(c.x + half_w, c.y)], Stroke::new((3.0 * k).max(1.0), rgba(LID, (255.0 * pulse) as u8)));
    }
}

/// Eye openness for a periodic blink: open except for a quick close/open near
/// the start of every `period` seconds.
pub fn blink(time: f64, period: f64) -> f32 {
    let phase = (time % period) as f32;
    let dur = 0.22;
    if phase < dur {
        let x = phase / dur;
        (2.0 * x - 1.0).abs().powf(0.7)
    } else {
        1.0
    }
}

/// Smoothstep from 0 to 1 as `x` goes from `a` to `b`.
pub fn ease(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

