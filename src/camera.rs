//! Webcam feed: a background thread opens a camera while the camera panel is
//! open, decodes frames to RGB and keeps the latest one for the UI.
//!
//! The camera is only opened on request and released as soon as the panel is
//! closed. Frames never leave the machine; snapshots are saved locally.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use eframe::egui;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraFormat, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum CamStatus {
    #[default]
    Off,
    Starting,
    Live,
    Error(String),
}

/// One decoded frame, tightly packed RGB8.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct CamState {
    pub status: CamStatus,
    /// Names of the cameras found, in the order the OS lists them.
    pub devices: Vec<String>,
    /// Index into `devices` of the camera in use.
    pub active: usize,
    /// e.g. "1280×720 · 30 FPS · MJPEG"
    pub format: String,
    pub frame: Option<Arc<Frame>>,
    /// Incremented for every new frame, so the UI only re-uploads changed frames.
    pub seq: u64,
    /// Measured frames per second.
    pub fps: f32,
}

pub type SharedCam = Arc<Mutex<CamState>>;

/// Owns the capture thread. Dropping it (or calling `stop`) releases the camera.
pub struct Camera {
    pub shared: SharedCam,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Default for Camera {
    fn default() -> Self {
        Self { shared: SharedCam::default(), stop: Arc::new(AtomicBool::new(false)), thread: None }
    }
}

impl Camera {
    /// Whether the camera is (or is about to be) capturing.
    pub fn running(&self) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Opens camera number `index` (wrapping around the device list) on a background thread.
    pub fn start(&mut self, index: usize, ctx: egui::Context) {
        self.stop();
        // The previous thread must release the device before it can be reopened.
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.stop = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.shared.lock().unwrap();
            st.status = CamStatus::Starting;
            st.frame = None;
            st.fps = 0.0;
        }
        let (shared, stop) = (self.shared.clone(), self.stop.clone());
        self.thread = Some(std::thread::spawn(move || {
            let result = capture(index, &shared, &stop, &ctx);
            let mut st = shared.lock().unwrap();
            st.status = match result {
                Ok(()) => CamStatus::Off,
                Err(e) => CamStatus::Error(e),
            };
            st.frame = None;
            drop(st);
            ctx.request_repaint();
        }));
    }

    /// Asks the capture thread to stop. Does not wait: opening a device can take
    /// a while, and the UI must not freeze. The thread releases the camera after
    /// its current frame.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Switches to the next camera in the device list.
    pub fn next(&mut self, ctx: egui::Context) {
        let next = self.shared.lock().unwrap().active + 1;
        self.start(next, ctx);
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Formats to try, best first. `Camera::new` fails when a request matches no
/// format the device offers, so we fall back to whatever the device can do.
fn requests() -> [RequestedFormat<'static>; 3] {
    let hd = |f| RequestedFormatType::Closest(CameraFormat::new(Resolution::new(1280, 720), f, 30));
    [
        RequestedFormat::new::<RgbFormat>(hd(FrameFormat::MJPEG)),
        RequestedFormat::new::<RgbFormat>(hd(FrameFormat::YUYV)),
        RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
    ]
}

fn capture(index: usize, shared: &SharedCam, stop: &AtomicBool, ctx: &egui::Context) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !nokhwa::nokhwa_check() {
        // Asks macOS for camera permission; the answer arrives asynchronously.
        let (tx, rx) = std::sync::mpsc::channel();
        nokhwa::nokhwa_initialize(move |granted| {
            let _ = tx.send(granted);
        });
        if !rx.recv().unwrap_or(false) {
            return Err("Camera access denied. Allow it in System Settings → Privacy & Security → Camera.".into());
        }
    }

    let backend = nokhwa::native_api_backend().ok_or("No camera backend for this platform.")?;
    let devices = nokhwa::query(backend).map_err(|e| format!("Could not list cameras: {e}"))?;
    if devices.is_empty() {
        return Err("No camera found.".into());
    }
    let active = index % devices.len();
    {
        let mut st = shared.lock().unwrap();
        st.devices = devices.iter().map(|d| d.human_name()).collect();
        st.active = active;
    }
    let target = devices[active].index().clone();

    let mut last_err = String::new();
    let mut cam = None;
    for req in requests() {
        match nokhwa::Camera::new(target.clone(), req) {
            Ok(c) => {
                cam = Some(c);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let mut cam = cam.ok_or_else(|| format!("Could not open the camera: {last_err}"))?;
    cam.open_stream().map_err(|e| format!("Could not start the camera (is another app using it?): {e}"))?;
    let fmt = cam.camera_format();
    {
        let mut st = shared.lock().unwrap();
        st.format = format!("{}×{} · {} FPS · {}", fmt.width(), fmt.height(), fmt.frame_rate(), fmt.format());
        st.status = CamStatus::Live;
    }

    let mut window_start = Instant::now();
    let mut window_frames = 0u32;
    let mut failures = 0;
    while !stop.load(Ordering::Relaxed) {
        let decoded = cam.frame().and_then(|buf| buf.decode_image::<RgbFormat>());
        let img = match decoded {
            Ok(img) => {
                failures = 0;
                img
            }
            // A dropped or corrupt frame now and then is normal; a run of them means the device is gone.
            Err(e) => {
                failures += 1;
                if failures > 30 {
                    let _ = cam.stop_stream();
                    return Err(format!("Camera stopped responding: {e}"));
                }
                continue;
            }
        };
        let frame = Frame { width: img.width(), height: img.height(), rgb: img.into_raw() };
        window_frames += 1;
        let elapsed = window_start.elapsed().as_secs_f32();
        let mut st = shared.lock().unwrap();
        if elapsed >= 1.0 {
            st.fps = window_frames as f32 / elapsed;
            window_start = Instant::now();
            window_frames = 0;
        }
        st.frame = Some(Arc::new(frame));
        st.seq += 1;
        drop(st);
        ctx.request_repaint();
    }
    let _ = cam.stop_stream();
    Ok(())
}

/// Where snapshots go: `~/Pictures/DAEMON` (falling back to the home folder).
pub fn snapshot_dir(home: &std::path::Path) -> PathBuf {
    let pictures = home.join("Pictures");
    if pictures.is_dir() {
        pictures.join("DAEMON")
    } else {
        home.to_path_buf()
    }
}

/// Saves `frame` as a timestamped PNG in `dir` and returns its path.
pub fn save_snapshot(frame: &Frame, dir: &std::path::Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let name = chrono::Local::now().format("daemon-%Y%m%d-%H%M%S.png").to_string();
    let path = dir.join(name);
    image::save_buffer(&path, &frame.rgb, frame.width, frame.height, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("Could not save snapshot: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_as_png() {
        let dir = std::env::temp_dir().join(format!("daemon-cam-test-{}", std::process::id()));
        let frame = Frame { width: 2, height: 1, rgb: vec![255, 0, 0, 0, 255, 0] };
        let path = save_snapshot(&frame, &dir).unwrap();
        let back = image::open(&path).unwrap().into_rgb8();
        assert_eq!(back.dimensions(), (2, 1));
        assert_eq!(back.into_raw(), frame.rgb);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_dir_prefers_pictures() {
        let home = std::env::temp_dir().join(format!("daemon-home-test-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(snapshot_dir(&home), home);
        std::fs::create_dir_all(home.join("Pictures")).unwrap();
        assert_eq!(snapshot_dir(&home), home.join("Pictures").join("DAEMON"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn idle_camera_is_not_running() {
        let mut cam = Camera::default();
        assert!(!cam.running());
        cam.stop();
        assert!(!cam.running());
        assert_eq!(cam.shared.lock().unwrap().status, CamStatus::Off);
    }
}
