//! UI sounds. Every effect is synthesized at runtime, but any of them can be
//! replaced by an audio file in a `sounds/` folder (see `custom_file`), and an
//! optional `sounds/ambient.*` track loops quietly in the background.
//! A dedicated thread owns the audio output; the UI talks to it over a channel.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, Sink, Source};

const RATE: u32 = 44_100;
/// Master gain applied to every synthesized sound.
const VOLUME: f32 = 3.0;
/// Gain of the looping ambient track.
const AMBIENT_VOLUME: f32 = 0.35;
/// Audio formats accepted for custom sounds.
const EXTENSIONS: [&str; 4] = ["wav", "ogg", "mp3", "flac"];

#[derive(Clone, Copy)]
pub enum Sfx {
    /// Keystroke click.
    Key,
    /// Heavier Enter key thunk.
    Enter,
    /// Short boot-log beep.
    Blip,
    /// Rising "access granted" fanfare.
    Granted,
    /// File-browser click.
    Click,
    /// Burst of quiet random data chirps.
    Chatter,
}

impl Sfx {
    const ALL: [Sfx; 6] = [Sfx::Key, Sfx::Enter, Sfx::Blip, Sfx::Granted, Sfx::Click, Sfx::Chatter];

    /// File stem used to override this sound, e.g. `sounds/key.wav`.
    fn file_stem(self) -> &'static str {
        match self {
            Sfx::Key => "key",
            Sfx::Enter => "enter",
            Sfx::Blip => "boot",
            Sfx::Granted => "granted",
            Sfx::Click => "click",
            Sfx::Chatter => "chatter",
        }
    }
}

enum Msg {
    Play(Sfx),
    Mute(bool),
}

pub struct Sound {
    tx: Sender<Msg>,
    muted: bool,
    /// Cleared if the audio device could not be opened or playback failed.
    ok: Arc<AtomicBool>,
    /// Number of custom sound files found (including the ambient track).
    custom: Arc<AtomicUsize>,
}

/// Folders searched for custom sounds: `./sounds`, `./assets/sounds`, and
/// `sounds` next to the executable (or in the macOS bundle's Resources).
fn sound_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("sounds"), PathBuf::from("assets/sounds")];
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
        dirs.push(exe_dir.join("sounds"));
        dirs.push(exe_dir.join("../Resources/sounds"));
    }
    dirs
}

/// Reads the first `<stem>.<ext>` found in the sound folders.
fn custom_file(stem: &str) -> Option<Arc<[u8]>> {
    for dir in sound_dirs() {
        for ext in EXTENSIONS {
            if let Ok(bytes) = std::fs::read(dir.join(format!("{stem}.{ext}"))) {
                return Some(bytes.into());
            }
        }
    }
    None
}

impl Sound {
    pub fn new(muted: bool) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>();
        let ok = Arc::new(AtomicBool::new(true));
        let custom_count = Arc::new(AtomicUsize::new(0));
        let (status, count) = (ok.clone(), custom_count.clone());
        thread::spawn(move || {
            let Ok((_stream, handle)) = OutputStream::try_default() else {
                status.store(false, Ordering::Relaxed);
                return;
            };

            let custom: HashMap<&'static str, Arc<[u8]>> = Sfx::ALL
                .iter()
                .filter_map(|s| custom_file(s.file_stem()).map(|b| (s.file_stem(), b)))
                .collect();
            let mut found = custom.len();

            // Optional looping background track.
            let ambient = custom_file("ambient").and_then(|bytes| {
                let sink = Sink::try_new(&handle).ok()?;
                let source = Decoder::new_looped(Cursor::new(bytes)).ok()?;
                sink.append(source.amplify(AMBIENT_VOLUME));
                if muted {
                    sink.pause();
                }
                found += 1;
                Some(sink)
            });
            count.store(found, Ordering::Relaxed);

            let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
            while let Ok(msg) = rx.recv() {
                let sfx = match msg {
                    Msg::Mute(m) => {
                        if let Some(sink) = &ambient {
                            if m { sink.pause() } else { sink.play() }
                        }
                        continue;
                    }
                    Msg::Play(sfx) => sfx,
                };
                let result = match custom.get(sfx.file_stem()).map(|b| Decoder::new(Cursor::new(b.clone()))) {
                    Some(Ok(decoder)) => handle.play_raw(decoder.convert_samples::<f32>()),
                    _ => handle.play_raw(SamplesBuffer::new(1, RATE, synth(sfx, &mut rng)).amplify(VOLUME)),
                };
                if result.is_err() {
                    status.store(false, Ordering::Relaxed);
                }
            }
        });
        Self { tx, muted, ok, custom: custom_count }
    }

    pub fn available(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }

    pub fn custom_count(&self) -> usize {
        self.custom.load(Ordering::Relaxed)
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        let _ = self.tx.send(Msg::Mute(self.muted));
    }

    pub fn play(&self, sfx: Sfx) {
        if !self.muted {
            let _ = self.tx.send(Msg::Play(sfx));
        }
    }
}

/// Tiny xorshift PRNG.
pub struct Rng(pub u64);

impl Rng {
    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}

fn tone(freq: f32, dur: f32, vol: f32, decay: f32, square: bool) -> Vec<f32> {
    let len = (dur * RATE as f32) as usize;
    (0..len)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let s = (TAU * freq * t).sin();
            let w = if square { s.signum() * 0.5 } else { s };
            let attack = (i as f32 / 80.0).min(1.0);
            w * vol * attack * (-t * decay).exp()
        })
        .collect()
}

/// High-passed white noise burst: sounds like a mechanical click.
fn noise(rng: &mut Rng, dur: f32, vol: f32, decay: f32) -> Vec<f32> {
    let len = (dur * RATE as f32) as usize;
    let mut prev = 0.0;
    (0..len)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let n = rng.next_f32() * 2.0 - 1.0;
            let hp = n - prev;
            prev = n;
            hp * 0.5 * vol * (-t * decay).exp()
        })
        .collect()
}

fn mix(mut a: Vec<f32>, b: Vec<f32>) -> Vec<f32> {
    if b.len() > a.len() {
        a.resize(b.len(), 0.0);
    }
    for (x, y) in a.iter_mut().zip(b) {
        *x += y;
    }
    a
}

/// Exponential frequency sweep with a smooth fade in and out.
fn sweep(f0: f32, f1: f32, dur: f32, vol: f32) -> Vec<f32> {
    let len = (dur * RATE as f32) as usize;
    let mut phase = 0.0_f32;
    (0..len)
        .map(|i| {
            let u = i as f32 / len as f32;
            let f = f0 * (f1 / f0).powf(u);
            phase += TAU * f / RATE as f32;
            phase.sin() * vol * (PI_F * u).sin()
        })
        .collect()
}

const PI_F: f32 = std::f32::consts::PI;

fn silence(dur: f32) -> Vec<f32> {
    vec![0.0; (dur * RATE as f32) as usize]
}

fn synth(sfx: Sfx, rng: &mut Rng) -> Vec<f32> {
    let mut out = match sfx {
        // Soft, glassy tick.
        Sfx::Key => {
            let f = rng.range(2800.0, 3600.0);
            mix(noise(rng, 0.02, 0.18, 260.0), tone(f, 0.04, 0.035, 120.0, false))
        }
        // Low "confirm" pulse.
        Sfx::Enter => mix(tone(180.0, 0.12, 0.25, 28.0, false), tone(720.0, 0.08, 0.06, 45.0, false)),
        Sfx::Blip => tone(rng.range(1400.0, 2600.0), 0.05, 0.05, 55.0, false),
        Sfx::Click => mix(tone(1900.0, 0.05, 0.08, 70.0, false), tone(2850.0, 0.05, 0.04, 90.0, false)),
        // Power-up: rising sweep resolving into a shimmering chord.
        Sfx::Granted => {
            let mut v = sweep(180.0, 1400.0, 0.45, 0.12);
            let chord = [880.0, 1108.7, 1318.5, 1760.0]
                .iter()
                .map(|&f| tone(f, 0.9, 0.05, 4.0, false))
                .fold(Vec::new(), mix);
            v.extend(chord);
            v
        }
        // Quiet stream of "data" pips.
        Sfx::Chatter => {
            let mut v = Vec::new();
            for _ in 0..(5 + (rng.next_f32() * 6.0) as usize) {
                let f = rng.range(1800.0, 4200.0);
                v.extend(tone(f, rng.range(0.02, 0.05), 0.018, 60.0, false));
                v.extend(silence(rng.range(0.01, 0.05)));
            }
            v
        }
    };
    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}
