use eframe::egui::Color32;

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    /// Main UI color: headings, lines, graphs.
    pub primary: Color32,
    /// Secondary highlight: network links, TX graph, corner brackets.
    pub accent: Color32,
    /// Readable body text (terminal foreground, values).
    pub text: Color32,
    /// Window background.
    pub bg: Color32,
    /// Translucent panel fill.
    pub panel: Color32,
    pub alert: Color32,
    /// Old-school CRT look: scanlines over everything.
    pub retro: bool,
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

impl Theme {
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "neon" => Self::make("neon", (180, 120, 255), (0, 240, 255), (12, 6, 24)),
            "solar" => Self::make("solar", (255, 190, 90), (0, 220, 200), (16, 10, 6)),
            "ice" => Self::make("ice", (200, 235, 255), (110, 160, 255), (6, 10, 18)),
            "crimson" => Self::make("crimson", (255, 90, 110), (255, 200, 120), (18, 5, 8)),
            "tron" => Self::make("tron", (170, 207, 209), (255, 140, 60), (4, 6, 8)),
            "matrix" => Self { retro: true, ..Self::make("matrix", (57, 255, 20), (0, 220, 255), (0, 4, 0)) },
            _ => Self::make("nova", (90, 220, 255), (0, 255, 150), (4, 7, 16)),
        }
    }

    fn make(name: &'static str, p: (u8, u8, u8), a: (u8, u8, u8), bg: (u8, u8, u8)) -> Self {
        let primary = Color32::from_rgb(p.0, p.1, p.2);
        let bg = Color32::from_rgb(bg.0, bg.1, bg.2);
        let panel = lerp_color(bg, primary, 0.04);
        Self {
            name,
            primary,
            accent: Color32::from_rgb(a.0, a.1, a.2),
            text: lerp_color(primary, Color32::WHITE, 0.55),
            bg,
            panel: Color32::from_rgba_unmultiplied(panel.r(), panel.g(), panel.b(), 225),
            alert: Color32::from_rgb(255, 80, 90),
            retro: false,
        }
    }

    /// The primary color with the given opacity.
    pub fn alpha(&self, a: u8) -> Color32 {
        let p = self.primary;
        Color32::from_rgba_unmultiplied(p.r(), p.g(), p.b(), a)
    }

    pub fn accent_alpha(&self, a: u8) -> Color32 {
        let p = self.accent;
        Color32::from_rgba_unmultiplied(p.r(), p.g(), p.b(), a)
    }
}
