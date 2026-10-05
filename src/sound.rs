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
const VOLUME: f32 = 1.0;
/// Gain of the looping ambient track.
const AMBIENT_VOLUME: f32 = 0.35;
/// Audio formats accepted for custom sounds.
const EXTENSIONS: [&str; 4] = ["wav", "ogg", "mp3", "flac"];

#[derive(Clone, Copy)]
pub enum Sfx {
    /// Keystroke: crushed digital tick.
    Key,
    /// Enter: distorted sub drop.
    Enter,
    /// Boot log line: glitch stutter.
    Blip,
    /// Boot complete: demonic power chord.
    Granted,
    /// File-browser click.
    Click,
    /// Ambient data chatter, sometimes whispering.
    Chatter,
    /// The eye opening: growl.
    Awaken,
    /// The eye blinking.
    Blink,
    /// Deep scan opened.
    Scan,
}

impl Sfx {
    const ALL: [Sfx; 9] = [
        Sfx::Key, Sfx::Enter, Sfx::Blip, Sfx::Granted, Sfx::Click,
        Sfx::Chatter, Sfx::Awaken, Sfx::Blink, Sfx::Scan,
    ];

    /// File stem used to override this sound, e.g. `sounds/key.wav`.
    fn file_stem(self) -> &'static str {
        match self {
            Sfx::Key => "key",
            Sfx::Enter => "enter",
            Sfx::Blip => "boot",
            Sfx::Granted => "granted",
            Sfx::Click => "click",
            Sfx::Chatter => "chatter",
            Sfx::Awaken => "awaken",
            Sfx::Blink => "blink",
            Sfx::Scan => "scan",
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

// ---------------------------------------------------------------------------
// DSP building blocks
// ---------------------------------------------------------------------------

fn len(dur: f32) -> usize {
    (dur * RATE as f32) as usize
}

fn silence(dur: f32) -> Vec<f32> {
    vec![0.0; len(dur)]
}

/// Sine or square tone with exponential decay (`decay` = 0 holds the level).
fn tone(freq: f32, dur: f32, vol: f32, decay: f32, square: bool) -> Vec<f32> {
    (0..len(dur))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let s = (TAU * freq * t).sin();
            let w = if square { s.signum() * 0.5 } else { s };
            let attack = (i as f32 / 60.0).min(1.0);
            w * vol * attack * (-t * decay).exp()
        })
        .collect()
}

/// Detuned stack of sawtooth oscillators, normalized to `vol`.
fn saws(freqs: &[f32], dur: f32, vol: f32) -> Vec<f32> {
    let n = freqs.len() as f32;
    (0..len(dur))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            freqs.iter().map(|f| 2.0 * (t * f).fract() - 1.0).sum::<f32>() / n * vol
        })
        .collect()
}

/// Exponential pitch sweep with exponential decay.
fn sweep(f0: f32, f1: f32, dur: f32, vol: f32, decay: f32, square: bool) -> Vec<f32> {
    let n = len(dur);
    let mut phase = 0.0_f32;
    (0..n)
        .map(|i| {
            let u = i as f32 / n as f32;
            let t = i as f32 / RATE as f32;
            phase += TAU * f0 * (f1 / f0).powf(u) / RATE as f32;
            let s = phase.sin();
            let w = if square { s.signum() * 0.5 } else { s };
            w * vol * (i as f32 / 60.0).min(1.0) * (-t * decay).exp()
        })
        .collect()
}

fn white(rng: &mut Rng, dur: f32, vol: f32) -> Vec<f32> {
    (0..len(dur)).map(|_| (rng.next_f32() * 2.0 - 1.0) * vol).collect()
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

/// Soft clipping; `drive` > 1 adds grit.
fn distort(v: &mut [f32], drive: f32) {
    let norm = drive.tanh();
    for s in v {
        *s = (*s * drive).tanh() / norm;
    }
}

/// One-pole low-pass filter.
fn lowpass(v: &mut [f32], cutoff: f32) {
    let a = 1.0 - (-TAU * cutoff / RATE as f32).exp();
    let mut y = 0.0;
    for s in v {
        y += a * (*s - y);
        *s = y;
    }
}

/// Crude band-pass: low-pass at `hi` minus low-pass at `lo`.
fn bandpass(v: &mut [f32], lo: f32, hi: f32) {
    let mut low = v.to_vec();
    lowpass(&mut low, lo);
    lowpass(v, hi);
    for (s, l) in v.iter_mut().zip(low) {
        *s -= l;
    }
}

/// Sample-rate and bit-depth reduction: the "digital glitch" sound.
/// Quantizes relative to the signal's peak, so quiet sounds keep their shape.
fn bitcrush(v: &mut [f32], hold: usize, levels: f32) {
    let peak = v.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    if peak == 0.0 {
        return;
    }
    let mut held = 0.0;
    for (i, s) in v.iter_mut().enumerate() {
        if i % hold.max(1) == 0 {
            held = (*s / peak * levels).round() / levels * peak;
        }
        *s = held;
    }
}

/// Linear fade-in over `attack` seconds and fade-out over the last `release` seconds.
fn envelope(v: &mut [f32], attack: f32, release: f32) {
    let n = v.len() as f32;
    for (i, s) in v.iter_mut().enumerate() {
        let t = i as f32 / RATE as f32;
        let rest = (n - i as f32) / RATE as f32;
        *s *= (t / attack.max(1e-4)).min(1.0) * (rest / release.max(1e-4)).min(1.0);
    }
}

/// Amplitude wobble at `rate` Hz: turns a drone into a growl.
fn tremolo(v: &mut [f32], rate: f32, depth: f32) {
    for (i, s) in v.iter_mut().enumerate() {
        let t = i as f32 / RATE as f32;
        *s *= 1.0 - depth * 0.5 * (1.0 + (TAU * rate * t).sin());
    }
}

/// Feedback echo that extends the sound by a few repeats.
fn echo(mut v: Vec<f32>, delay: f32, feedback: f32) -> Vec<f32> {
    let d = len(delay).max(1);
    v.resize(v.len() + d * 5, 0.0);
    for i in d..v.len() {
        v[i] += v[i - d] * feedback;
    }
    v
}

fn concat(parts: Vec<Vec<f32>>) -> Vec<f32> {
    parts.into_iter().flatten().collect()
}

/// Breathy, band-limited noise with syllable-like pulses: an inhuman whisper.
fn whisper(rng: &mut Rng, dur: f32, vol: f32) -> Vec<f32> {
    let mut v = white(rng, dur, 1.0);
    bandpass(&mut v, 500.0, 2600.0);
    let syllables = rng.range(5.0, 9.0);
    for (i, s) in v.iter_mut().enumerate() {
        let t = i as f32 / RATE as f32;
        *s *= vol * (0.5 + 0.5 * (TAU * syllables * t).sin()).powf(2.0);
    }
    envelope(&mut v, 0.15, 0.3);
    v
}

// ---------------------------------------------------------------------------
// The sounds: hacker glitch meets demonic low end
// ---------------------------------------------------------------------------

fn synth(sfx: Sfx, rng: &mut Rng) -> Vec<f32> {
    let mut out = match sfx {
        // Bit-crushed digital tick over a tiny sub thump.
        Sfx::Key => {
            let mut tick = tone(rng.range(1600.0, 3400.0), 0.014, 0.22, 250.0, true);
            bitcrush(&mut tick, 3, 5.0);
            mix(tick, sweep(150.0, 55.0, 0.05, 0.3, 45.0, false))
        }
        // Distorted sub drop with a metallic ring and a crunchy tail.
        Sfx::Enter => {
            let mut sub = sweep(140.0, 36.0, 0.38, 0.9, 8.0, false);
            distort(&mut sub, 3.0);
            let ring = mix(tone(337.0, 0.3, 0.07, 14.0, false), tone(913.0, 0.25, 0.04, 20.0, false));
            let mut crunch = white(rng, 0.07, 0.2);
            bitcrush(&mut crunch, 9, 4.0);
            envelope(&mut crunch, 0.001, 0.06);
            echo(mix(mix(sub, ring), crunch), 0.09, 0.28)
        }
        // Stuttering glitch burst.
        Sfx::Blip => {
            let f = rng.range(250.0, 1400.0);
            let mut parts = Vec::new();
            for k in 0..3 {
                let mut seg = tone(f * (1.0 + k as f32 * 0.5), 0.012, 0.18, 0.0, true);
                bitcrush(&mut seg, 4, 4.0);
                parts.push(seg);
                parts.push(silence(0.008));
            }
            mix(concat(parts), sweep(110.0, 50.0, 0.06, 0.22, 30.0, false))
        }
        // Clicking in the file browser: short crushed down-chirp.
        Sfx::Click => {
            let mut c = sweep(2400.0, 600.0, 0.05, 0.25, 40.0, true);
            bitcrush(&mut c, 5, 5.0);
            mix(c, sweep(120.0, 60.0, 0.05, 0.25, 40.0, false))
        }
        // Boot complete: a detuned, distorted power chord built on the tritone,
        // a whisper, and a glitch arpeggio on top.
        Sfx::Granted => {
            let mut chord = saws(&[55.0, 55.4, 77.8, 82.4, 110.3, 116.5], 2.4, 0.9);
            lowpass(&mut chord, 900.0);
            distort(&mut chord, 2.5);
            envelope(&mut chord, 0.3, 1.5);
            let mut arp = Vec::new();
            for f in [880.0, 1244.5, 1760.0, 2489.0, 1760.0] {
                let mut n = tone(f, 0.06, 0.12, 25.0, true);
                bitcrush(&mut n, 6, 5.0);
                arp.extend(n);
            }
            let w = whisper(rng, 1.4, 0.25);
            echo(mix(mix(chord, arp), w), 0.19, 0.32)
        }
        // The eye opens: a growling, swelling drone with breath.
        Sfx::Awaken => {
            let mut growl = saws(&[41.2, 43.7, 61.7, 82.4], 2.4, 1.0);
            tremolo(&mut growl, 7.5, 0.55);
            lowpass(&mut growl, 520.0);
            distort(&mut growl, 4.0);
            envelope(&mut growl, 1.0, 0.9);
            let mut breath = white(rng, 2.0, 0.6);
            lowpass(&mut breath, 650.0);
            envelope(&mut breath, 0.8, 0.8);
            let mut v = mix(growl, breath);
            for s in &mut v {
                *s *= 0.55;
            }
            echo(v, 0.27, 0.3)
        }
        // Eyelid: a soft low flap.
        Sfx::Blink => {
            let mut v = sweep(95.0, 45.0, 0.1, 0.45, 25.0, false);
            distort(&mut v, 2.0);
            mix(v, tone(2600.0, 0.006, 0.05, 400.0, false))
        }
        // Deep scan: rising crushed sweep over a demonic drone.
        Sfx::Scan => {
            let mut rise = sweep(180.0, 2600.0, 0.6, 0.18, 1.5, true);
            bitcrush(&mut rise, 6, 6.0);
            let mut drone = saws(&[55.0, 77.8], 0.8, 0.6);
            lowpass(&mut drone, 600.0);
            distort(&mut drone, 3.0);
            envelope(&mut drone, 0.05, 0.5);
            echo(mix(rise, drone), 0.12, 0.3)
        }
        // Ambient: modem-like data chatter, sometimes with a whisper underneath.
        Sfx::Chatter => {
            let mut v = Vec::new();
            for _ in 0..(6 + (rng.next_f32() * 8.0) as usize) {
                let mut n = tone(rng.range(400.0, 3200.0), rng.range(0.01, 0.035), 0.06, 20.0, true);
                bitcrush(&mut n, 6, 4.0);
                v.extend(n);
                v.extend(silence(rng.range(0.005, 0.04)));
            }
            if rng.next_f32() < 0.35 {
                v = mix(v, whisper(rng, 1.2, 0.12));
            }
            echo(v, 0.15, 0.25)
        }
    };
    for s in &mut out {
        *s = s.clamp(-1.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_audible_and_in_range() {
        let mut rng = Rng(42);
        for sfx in Sfx::ALL {
            let s = synth(sfx, &mut rng);
            assert!(!s.is_empty(), "{} is empty", sfx.file_stem());
            assert!(s.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{} out of range", sfx.file_stem());
            assert!(s.iter().any(|x| x.abs() > 0.01), "{} is silent", sfx.file_stem());
        }
    }

    #[test]
    fn file_stems_are_unique() {
        let mut stems: Vec<_> = Sfx::ALL.iter().map(|s| s.file_stem()).collect();
        stems.sort();
        stems.dedup();
        assert_eq!(stems.len(), Sfx::ALL.len());
    }

    #[test]
    fn rng_stays_in_unit_interval() {
        let mut rng = Rng(7);
        for _ in 0..10_000 {
            let x = rng.next_f32();
            assert!((0.0..1.0).contains(&x));
        }
    }
}
