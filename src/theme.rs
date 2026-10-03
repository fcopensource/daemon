use eframe::egui::Color32;

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub primary: Color32,
    /// Secondary highlight (network links, TX graph, glitch fringe).
    pub accent: Color32,
    pub bg: Color32,
    pub alert: Color32,
}

impl Theme {
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "tron" => Self::make("tron", (170, 207, 209), (255, 140, 60)),
            "blade" => Self::make("blade", (204, 133, 61), (90, 200, 255)),
            "red" => Self::make("red", (255, 70, 70), (255, 200, 80)),
            "purple" => Self::make("purple", (190, 120, 255), (80, 255, 200)),
            "amber" => Self::make("amber", (255, 176, 0), (255, 80, 40)),
            "ice" => Self::make("ice", (120, 220, 255), (255, 255, 255)),
            _ => Self::make("daemon", (57, 255, 20), (0, 220, 255)),
        }
    }

    fn make(name: &'static str, p: (u8, u8, u8), a: (u8, u8, u8)) -> Self {
        Self {
            name,
            primary: Color32::from_rgb(p.0, p.1, p.2),
            accent: Color32::from_rgb(a.0, a.1, a.2),
            bg: Color32::from_rgb(0, 0, 0),
            alert: Color32::from_rgb(255, 50, 70),
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
