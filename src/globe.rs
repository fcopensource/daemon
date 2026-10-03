//! Rotating wireframe globe with dotted continents and animated network links.

use std::f32::consts::PI;

use eframe::egui::{pos2, vec2, Align2, FontId, Pos2, Sense, Stroke, Ui};

use crate::theme::Theme;

type V3 = [f32; 3];

/// Coarse land mask on a 10° grid: (row, first col, last col).
/// Row 0 is centered on 85°N, col 0 on 175°W.
const LAND: &[(u8, u8, u8)] = &[
    (0, 14, 15),
    (1, 6, 10), (1, 12, 15), (1, 19, 19), (1, 23, 23), (1, 27, 30),
    (2, 1, 11), (2, 13, 16), (2, 18, 35),
    (3, 2, 2), (3, 5, 11), (3, 17, 31), (3, 33, 33),
    (4, 5, 11), (4, 17, 31), (4, 32, 32),
    (5, 6, 10), (5, 17, 19), (5, 21, 29), (5, 31, 31),
    (6, 7, 8), (6, 10, 10), (6, 16, 23), (6, 25, 29),
    (7, 8, 9), (7, 16, 22), (7, 25, 25), (7, 28, 29),
    (8, 10, 12), (8, 17, 21), (8, 28, 29),
    (9, 10, 14), (9, 19, 21), (9, 28, 31),
    (10, 10, 14), (10, 19, 22), (10, 30, 32),
    (11, 11, 13), (11, 19, 20), (11, 29, 32),
    (12, 11, 12), (12, 20, 20), (12, 32, 32), (12, 35, 35),
    (13, 11, 11), (13, 34, 34),
    (14, 11, 11),
    (15, 12, 12),
    (16, 0, 35),
    (17, 0, 35),
];

/// (lat, lon) of network nodes.
const NODES: &[(f32, f32)] = &[
    (40.7, -74.0),   // New York
    (51.5, -0.1),    // London
    (55.8, 37.6),    // Moscow
    (35.7, 139.7),   // Tokyo
    (-33.9, 151.2),  // Sydney
    (-23.5, -46.6),  // São Paulo
    (19.1, 72.9),    // Mumbai
    (-33.9, 18.4),   // Cape Town
    (37.8, -122.4),  // San Francisco
    (1.3, 103.8),    // Singapore
];

const LINKS: &[(usize, usize)] = &[
    (0, 1), (1, 2), (2, 3), (3, 4), (0, 5), (1, 6), (6, 9),
    (9, 4), (7, 1), (8, 3), (0, 8), (5, 7), (6, 2),
];

fn from_latlon(lat: f32, lon: f32) -> V3 {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos() * lo.sin(), la.sin(), la.cos() * lo.cos()]
}

struct View {
    c: Pos2,
    r: f32,
    rot: f32,
    tilt: f32,
}

impl View {
    /// Screen position and depth (z > 0 faces the viewer).
    fn project(&self, v: V3) -> (Pos2, f32) {
        let (s, c) = self.rot.sin_cos();
        let x = v[0] * c + v[2] * s;
        let z = -v[0] * s + v[2] * c;
        let (ts, tc) = self.tilt.sin_cos();
        let y = v[1] * tc - z * ts;
        let z2 = v[1] * ts + z * tc;
        (pos2(self.c.x + x * self.r, self.c.y - y * self.r), z2)
    }
}

pub fn globe(ui: &mut Ui, t: &Theme, time: f64, height: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let p = ui.painter_at(rect);
    let r = rect.width().min(rect.height()) / 2.0 - 24.0;
    let c = rect.center();
    let rot = (time * 0.3 % std::f64::consts::TAU) as f32;
    let view = View { c, r, rot, tilt: 0.35 };

    // Atmosphere glow.
    p.circle_filled(c, r + 9.0, t.alpha(6));
    p.circle_filled(c, r + 4.0, t.alpha(10));
    p.circle_filled(c, r, t.bg);
    p.circle_filled(c, r, t.alpha(8));

    // Graticule.
    let front = Stroke::new(1.0_f32, t.alpha(55));
    let back = Stroke::new(1.0_f32, t.alpha(14));
    let polyline = |pts: &mut dyn Iterator<Item = V3>| {
        let mut prev: Option<(Pos2, f32)> = None;
        for v in pts {
            let (pt, z) = view.project(v);
            if let Some((pp, pz)) = prev {
                p.line_segment([pp, pt], if z + pz > 0.0 { front } else { back });
            }
            prev = Some((pt, z));
        }
    };
    for i in 0..12 {
        let lon = i as f32 * 30.0;
        polyline(&mut (0..=36).map(|j| from_latlon(-90.0 + j as f32 * 5.0, lon)));
    }
    for lat in [-60.0, -30.0, 0.0, 30.0, 60.0] {
        polyline(&mut (0..=72).map(|j| from_latlon(lat, j as f32 * 5.0)));
    }

    // Continents as dots.
    for &(row, c0, c1) in LAND {
        let lat = 85.0 - row as f32 * 10.0;
        for col in c0..=c1 {
            let lon = -175.0 + col as f32 * 10.0;
            for (dl, dn) in [(-2.5, -2.5), (-2.5, 2.5), (2.5, -2.5), (2.5, 2.5)] {
                let (pt, z) = view.project(from_latlon(lat + dl, lon + dn));
                if z > 0.0 {
                    p.circle_filled(pt, 1.4, t.alpha((60.0 + 180.0 * z) as u8));
                } else {
                    p.circle_filled(pt, 1.0, t.alpha(12));
                }
            }
        }
    }

    // Links: great-circle arcs lifted off the surface, with a moving packet.
    let visible = |pt: Pos2, z: f32| z > 0.0 || (pt - c).length() > r;
    for (k, &(a, b)) in LINKS.iter().enumerate() {
        let va = from_latlon(NODES[a].0, NODES[a].1);
        let vb = from_latlon(NODES[b].0, NODES[b].1);
        let dot = (va[0] * vb[0] + va[1] * vb[1] + va[2] * vb[2]).clamp(-1.0, 1.0);
        let theta = dot.acos();
        if theta < 1e-3 {
            continue;
        }
        let s = theta.sin();
        let point = |u: f32| -> V3 {
            let ka = ((1.0 - u) * theta).sin() / s;
            let kb = (u * theta).sin() / s;
            let h = 1.0 + 0.2 * (PI * u).sin() * (theta / PI + 0.3);
            [(va[0] * ka + vb[0] * kb) * h, (va[1] * ka + vb[1] * kb) * h, (va[2] * ka + vb[2] * kb) * h]
        };
        let mut prev: Option<(Pos2, bool)> = None;
        for j in 0..=32 {
            let (pt, z) = view.project(point(j as f32 / 32.0));
            let vis = visible(pt, z);
            if let Some((pp, pv)) = prev {
                let a = if vis && pv { 150 } else { 20 };
                p.line_segment([pp, pt], Stroke::new(1.0_f32, t.accent_alpha(a)));
            }
            prev = Some((pt, vis));
        }
        let u = ((time * 0.25 + k as f64 * 0.37) % 1.0) as f32;
        let (pt, z) = view.project(point(u));
        if visible(pt, z) {
            p.circle_filled(pt, 2.2, t.accent);
        }
    }

    // Nodes with pulsing rings.
    for (i, &(lat, lon)) in NODES.iter().enumerate() {
        let (pt, z) = view.project(from_latlon(lat, lon));
        if z > 0.0 {
            p.circle_filled(pt, 2.5, t.primary);
            let ph = ((time * 0.8 + i as f64 * 0.13) % 1.0) as f32;
            p.circle_stroke(pt, 2.0 + ph * 9.0, Stroke::new(1.0_f32, t.alpha(((1.0 - ph) * 200.0) as u8)));
        }
    }

    p.circle_stroke(c, r, Stroke::new(1.0_f32, t.alpha(140)));

    // HUD ring: slowly counter-rotating ticks plus a sweeping accent arc.
    let spin = -(time * 0.15) as f32;
    for i in 0..72 {
        let a = spin + i as f32 * 5f32.to_radians();
        let (inner, alpha) = if i % 6 == 0 { (r + 9.0, 160) } else { (r + 12.0, 60) };
        let dir = vec2(a.cos(), a.sin());
        p.line_segment([c + dir * inner, c + dir * (r + 16.0)], Stroke::new(1.0_f32, t.alpha(alpha)));
    }
    let sweep = (time * 1.2) as f32;
    crate::widgets::arc(&p, c, r + 20.0, sweep, sweep + 0.9, Stroke::new(2.0_f32, t.accent_alpha(200)));
    crate::widgets::arc(&p, c, r + 20.0, sweep + PI, sweep + PI + 0.4, Stroke::new(2.0_f32, t.alpha(140)));

    // HUD readouts.
    let font = FontId::monospace(9.0);
    let lon_deg = -(rot.to_degrees() % 360.0);
    p.text(rect.left_top() + vec2(2.0, 2.0), Align2::LEFT_TOP, format!("LON {lon_deg:+07.2}"), font.clone(), t.alpha(150));
    p.text(rect.right_top() + vec2(-2.0, 2.0), Align2::RIGHT_TOP, "TILT 20.0", font.clone(), t.alpha(150));
    p.text(
        rect.left_bottom() + vec2(2.0, -2.0),
        Align2::LEFT_BOTTOM,
        format!("NODES {}", NODES.len()),
        font.clone(),
        t.alpha(150),
    );
    p.text(rect.right_bottom() + vec2(-2.0, -2.0), Align2::RIGHT_BOTTOM, format!("LINKS {}", LINKS.len()), font, t.alpha(150));
}
