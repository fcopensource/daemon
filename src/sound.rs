//! Synthesized UI sounds (no audio files). A dedicated thread owns the audio
//! output stream; the UI sends it sound events over a channel.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Source};

const RATE: u32 = 44_100;
/// Master gain applied to every sound.
const VOLUME: f32 = 3.0;

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

pub struct Sound {
    tx: Sender<Sfx>,
    pub muted: bool,
    /// Cleared if the audio device could not be opened or playback failed.
    ok: Arc<AtomicBool>,
}

impl Sound {
    pub fn new(muted: bool) -> Self {
        let (tx, rx) = mpsc::channel::<Sfx>();
        let ok = Arc::new(AtomicBool::new(true));
        let status = ok.clone();
        thread::spawn(move || {
            let Ok((_stream, handle)) = OutputStream::try_default() else {
                status.store(false, Ordering::Relaxed);
                return;
            };
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
            while let Ok(sfx) = rx.recv() {
                let samples = synth(sfx, &mut rng);
                if handle.play_raw(SamplesBuffer::new(1, RATE, samples).amplify(VOLUME)).is_err() {
                    status.store(false, Ordering::Relaxed);
                }
            }
        });
        Self { tx, muted, ok }
    }

    pub fn available(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }

    pub fn play(&self, sfx: Sfx) {
        if !self.muted {
            let _ = self.tx.send(sfx);
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
