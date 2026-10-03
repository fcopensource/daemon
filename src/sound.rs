//! Synthesized UI sounds (no audio files). A dedicated thread owns the audio
//! output stream; the UI sends it sound events over a channel.

use std::f32::consts::TAU;
use std::sync::mpsc::{self, Sender};
use std::thread;

use rodio::buffer::SamplesBuffer;
use rodio::OutputStream;

const RATE: u32 = 44_100;

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
}

impl Sound {
    pub fn new(muted: bool) -> Self {
        let (tx, rx) = mpsc::channel::<Sfx>();
        thread::spawn(move || {
            // No audio device: just drop every event.
            let Ok((_stream, handle)) = OutputStream::try_default() else { return };
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
            while let Ok(sfx) = rx.recv() {
                let samples = synth(sfx, &mut rng);
                let _ = handle.play_raw(SamplesBuffer::new(1, RATE, samples));
            }
        });
        Self { tx, muted }
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

fn silence(dur: f32) -> Vec<f32> {
    vec![0.0; (dur * RATE as f32) as usize]
}

fn synth(sfx: Sfx, rng: &mut Rng) -> Vec<f32> {
    let mut out = match sfx {
        Sfx::Key => {
            let f = rng.range(1500.0, 2600.0);
            mix(noise(rng, 0.03, 0.35, 170.0), tone(f, 0.03, 0.04, 150.0, true))
        }
        Sfx::Enter => mix(noise(rng, 0.07, 0.45, 60.0), tone(140.0, 0.09, 0.3, 35.0, false)),
        Sfx::Blip => tone(rng.range(900.0, 2000.0), 0.035, 0.07, 40.0, true),
        Sfx::Click => mix(tone(2400.0, 0.02, 0.1, 200.0, false), tone(1200.0, 0.03, 0.08, 120.0, true)),
        Sfx::Granted => {
            let mut v = Vec::new();
            for f in [523.25, 659.25, 783.99] {
                v.extend(tone(f, 0.09, 0.12, 12.0, true));
            }
            v.extend(tone(1046.5, 0.35, 0.12, 6.0, true));
            v
        }
        Sfx::Chatter => {
            let mut v = Vec::new();
            for _ in 0..(6 + (rng.next_f32() * 8.0) as usize) {
                let f = rng.range(600.0, 3200.0);
                let d = rng.range(0.015, 0.04);
                v.extend(tone(f, d, 0.03, 30.0, true));
                v.extend(silence(rng.range(0.005, 0.03)));
            }
            v
        }
    };
    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}
