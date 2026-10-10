//! Sounds (off unless `sounds = true`): a soft brush scratch as you type, a
//! sword's shing when a deleted name is cut, a seal's thud, a tap of chalk.
//! Themes say which sound goes with what (`Theme::sound`); this file makes
//! them and plays them.
//!
//! The sounds are made here, from noise and decaying tones, so there are no
//! recordings to ship. Each is written once as a short WAV file in the temp
//! folder and played with `afplay`, the system's own player, at the volume
//! set; a quick run of the same sound (typing fast) plays one now and then.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::theme::Sound;

const RATE: u32 = 44_100;

/// Plays sounds, keeping each one from repeating faster than it can be
/// heard.
pub struct Player {
    volume: f32,
    last: HashMap<Sound, Instant>,
}

impl Player {
    pub fn new() -> Self {
        Self { volume: 0.0, last: HashMap::new() }
    }

    /// 0 is silent (sounds off).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    pub fn play(&mut self, sound: Sound) {
        if self.volume <= 0.0 {
            return;
        }
        let now = Instant::now();
        let gap = if sound == Sound::Brush || sound == Sound::Chalk { 70 } else { 120 };
        if self.last.get(&sound).is_some_and(|t| now.duration_since(*t) < Duration::from_millis(gap)) {
            return;
        }
        self.last.insert(sound, now);
        let Some(path) = file(sound) else { return };
        let volume = self.volume;
        // A thread waits on the player so it doesn't linger as a zombie.
        std::thread::spawn(move || {
            let _ = std::process::Command::new("afplay").arg("-v").arg(format!("{volume:.2}")).arg(path).status();
        });
    }
}

/// The sound's file, made if it is missing.
fn file(sound: Sound) -> Option<PathBuf> {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let dir = std::env::temp_dir().join(format!("oneprompt-{user}")).join("sounds-1");
    let path = dir.join(format!("{sound:?}.wav").to_lowercase());
    if !path.exists() {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&path, wav(&make(sound))).ok()?;
    }
    Some(path)
}

/// White noise, repeatable.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// A one-pole low-pass, `k` of the way to the input each sample.
fn smooth(samples: &mut [f32], k: f32) {
    let mut y = 0.0;
    for s in samples {
        y += (*s - y) * k;
        *s = y;
    }
}

/// The sound's samples, -1..1.
fn make(sound: Sound) -> Vec<f32> {
    let seconds = match sound {
        Sound::Brush => 0.09,
        Sound::Chalk => 0.05,
        Sound::Sword => 0.7,
        Sound::Thud => 0.22,
        Sound::Eraser => 0.45,
        Sound::Fire => 0.9,
    };
    let n = (seconds * RATE as f32) as usize;
    let t = |i: usize| i as f32 / RATE as f32;
    let mut noise = Noise(0x2545_f491_4f6c_dd1d + sound as u64);
    let mut out = vec![0.0f32; n];
    match sound {
        // Bristles dragged over paper: soft, low noise that swells and fades.
        Sound::Brush => {
            for (i, s) in out.iter_mut().enumerate() {
                let u = t(i) / seconds;
                *s = noise.next() * (u * 12.0).min(1.0) * (1.0 - u).powi(2);
            }
            smooth(&mut out, 0.18);
            gain(&mut out, 0.5);
        }
        // A tap of chalk: a short bright click with a little grit after it.
        Sound::Chalk => {
            for (i, s) in out.iter_mut().enumerate() {
                let x = t(i);
                *s = noise.next() * (-x * 90.0).exp() + (x * 2.0 * std::f32::consts::PI * 1800.0).sin() * (-x * 160.0).exp() * 0.4;
            }
            smooth(&mut out, 0.6);
            gain(&mut out, 0.45);
        }
        // The blade: a quick rising whoosh, then the steel ringing out with
        // a few inharmonic partials.
        Sound::Sword => {
            let partials = [(2210.0, 1.0, 5.0), (3470.0, 0.6, 7.0), (5190.0, 0.4, 9.0), (7330.0, 0.2, 12.0)];
            let mut swish = vec![0.0f32; n];
            for (i, s) in swish.iter_mut().enumerate() {
                let x = t(i);
                *s = noise.next() * (x / 0.05).min(1.0) * (-((x - 0.06).max(0.0)) * 30.0).exp();
            }
            smooth(&mut swish, 0.35);
            for (i, s) in out.iter_mut().enumerate() {
                let x = t(i);
                let ring: f32 = partials
                    .iter()
                    .map(|&(f, a, d)| (x * 2.0 * std::f32::consts::PI * f).sin() * a * (-(x - 0.05).max(0.0) * d).exp())
                    .sum();
                *s = swish[i] * 0.7 + ring * 0.25 * (x / 0.04).min(1.0);
            }
            gain(&mut out, 0.55);
        }
        // A seal pressed down: a low knock that drops in pitch, and the
        // paper's crunch.
        Sound::Thud => {
            let mut phase = 0.0f32;
            for (i, s) in out.iter_mut().enumerate() {
                let x = t(i);
                phase += 2.0 * std::f32::consts::PI * (70.0 + 90.0 * (-x * 30.0).exp()) / RATE as f32;
                *s = phase.sin() * (-x * 18.0).exp() + noise.next() * 0.25 * (-x * 60.0).exp();
            }
            smooth(&mut out, 0.5);
            gain(&mut out, 0.8);
        }
        // An eraser rubbed back and forth: soft noise rising and falling.
        Sound::Eraser => {
            for (i, s) in out.iter_mut().enumerate() {
                let u = t(i) / seconds;
                let strokes = (u * std::f32::consts::PI * 3.0).sin().abs();
                *s = noise.next() * strokes * (1.0 - u);
            }
            smooth(&mut out, 0.12);
            gain(&mut out, 0.55);
        }
        // Paper catching fire: a low roar with crackles in it.
        Sound::Fire => {
            for (i, s) in out.iter_mut().enumerate() {
                let u = t(i) / seconds;
                *s = noise.next() * 0.25 * (u * 6.0).min(1.0) * (1.0 - u);
            }
            smooth(&mut out, 0.05);
            let mut crackle = Noise(91);
            let mut i = 0;
            while i < n {
                i += (crackle.next().abs() * 0.06 * RATE as f32) as usize + 200;
                for k in 0..120.min(n.saturating_sub(i)) {
                    out[i + k] += crackle.next() * (-(k as f32) / 25.0).exp() * 0.8 * (1.0 - (i as f32 / n as f32));
                }
            }
            gain(&mut out, 0.6);
        }
    }
    // Ends faded, so nothing clicks.
    let edge = 64.min(n / 2);
    for k in 0..edge {
        let g = k as f32 / edge as f32;
        out[k] *= g;
        out[n - 1 - k] *= g;
    }
    out
}

/// Scales `samples` so their peak is `peak`.
fn gain(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 0.0 {
        for s in samples {
            *s *= peak / max;
        }
    }
}

/// 16-bit mono WAV.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_short_quiet_and_clean() {
        for sound in [Sound::Brush, Sound::Chalk, Sound::Sword, Sound::Thud, Sound::Eraser, Sound::Fire] {
            let samples = make(sound);
            assert!(!samples.is_empty() && samples.len() < RATE as usize, "{sound:?}");
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.1 && peak <= 0.81, "{sound:?} peak {peak}");
            assert!(samples[0].abs() < 1e-3 && samples[samples.len() - 1].abs() < 1e-3, "{sound:?} clicks");
            let file = wav(&samples);
            assert_eq!(&file[..4], b"RIFF");
            assert_eq!(file.len(), 44 + samples.len() * 2);
        }
    }
}
