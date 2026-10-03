//! Clickable file-system browser shown under the terminal.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, FontId, Rect, Sense, Stroke};

use crate::theme::Theme;
use crate::widgets::{fmt_bytes, truncate};

const MAX_ENTRIES: usize = 500;

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

pub enum FileAction {
    /// User opened a directory: the shell should `cd` there.
    Cd(PathBuf),
    /// User clicked a file: type its path into the shell.
    Insert(PathBuf),
}

pub struct FileBrowser {
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    last_scan: Option<Instant>,
    error: Option<String>,
}

impl FileBrowser {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd, entries: Vec::new(), last_scan: None, error: None }
    }

    fn scan(&mut self) {
        self.last_scan = Some(Instant::now());
        match fs::read_dir(&self.cwd) {
            Ok(rd) => {
                let mut v: Vec<Entry> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| {
                        let md = e.metadata().ok();
                        Entry {
                            name: e.file_name().to_string_lossy().into_owned(),
                            path: e.path(),
                            is_dir: md.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                            size: md.map(|m| m.len()).unwrap_or(0),
                        }
                    })
                    .collect();
                v.sort_by(|a, b| {
                    b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                v.truncate(MAX_ENTRIES);
                self.entries = v;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, t: &Theme) -> Option<FileAction> {
        if self.last_scan.map_or(true, |s| s.elapsed() > Duration::from_secs(3)) {
            self.scan();
        }
        if let Some(err) = &self.error {
            ui.colored_label(t.alert, format!("ACCESS DENIED: {err}"));
        }

        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                if let Some(parent) = self.cwd.parent() {
                    if tile(ui, t, "..", true, "UP").clicked() {
                        action = Some(FileAction::Cd(parent.to_path_buf()));
                    }
                }
                for e in &self.entries {
                    let sub = if e.is_dir { "DIR".to_string() } else { fmt_bytes(e.size) };
                    let resp = tile(ui, t, &e.name, e.is_dir, &sub);
                    if resp.clicked() {
                        action = Some(if e.is_dir {
                            FileAction::Cd(e.path.clone())
                        } else {
                            FileAction::Insert(e.path.clone())
                        });
                    }
                    resp.on_hover_text(&e.name);
                }
            });
        });

        if let Some(FileAction::Cd(p)) = &action {
            self.cwd = p.clone();
            self.last_scan = None;
        }
        action
    }
}

fn tile(ui: &mut egui::Ui, t: &Theme, name: &str, is_dir: bool, sub: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 58.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    if hovered {
        p.rect_filled(rect, 2.0, t.alpha(30));
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, t.alpha(90)));
    }
    let icon = Rect::from_center_size(pos2(rect.center().x, rect.top() + 17.0), vec2(22.0, 16.0));
    if is_dir {
        let c = t.alpha(if hovered { 230 } else { 150 });
        p.rect_filled(icon, 1.0, c);
        p.rect_filled(Rect::from_min_size(icon.left_top() - vec2(0.0, 4.0), vec2(10.0, 4.0)), 0.0, c);
    } else {
        p.rect_stroke(icon.shrink2(vec2(4.0, 0.0)), 1.0, Stroke::new(1.0_f32, t.primary));
    }
    p.text(pos2(rect.center().x, rect.top() + 31.0), Align2::CENTER_TOP, truncate(name, 13), FontId::monospace(10.0), t.primary);
    p.text(pos2(rect.center().x, rect.top() + 44.0), Align2::CENTER_TOP, sub, FontId::monospace(9.0), t.alpha(120));
    resp
}
