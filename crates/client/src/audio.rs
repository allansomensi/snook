//! Synthesised sound effects (no audio files needed).

use std::f32::consts::TAU;

use macroquad::audio::{PlaySoundParams, Sound, load_sound_from_bytes, play_sound};

use crate::game::{ImpactKind, ImpactSound};

const RATE: u32 = 44_100;

/// Variants of each sound, so repeated hits never sound identical.
const VARIANTS: u32 = 4;

pub struct Sounds {
    cue: Vec<Sound>,
    ball: Vec<Sound>,
    cushion: Vec<Sound>,
    pocket: Vec<Sound>,
    next: std::cell::Cell<u32>,
}

/// A small, deterministic noise source.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// A damped resonance: frequency (Hz), amplitude, decay time (s).
type Mode = (f32, f32, f32);

/// Modal synthesis: a short filtered noise burst (the impact itself) plus
/// the resonances it excites, then a touch of room. `brightness` (0..1) is
/// how much of the burst's high end gets through.
fn strike(
    seed: u32,
    duration: f32,
    burst: f32,
    brightness: f32,
    burst_gain: f32,
    modes: &[Mode],
) -> Vec<f32> {
    let n = (duration * RATE as f32) as usize;
    let mut noise = Noise(seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453) | 1);
    // Random phases, so variants differ beyond their tuning.
    let phases: Vec<f32> =
        modes.iter().map(|_| (noise.next() + 1.0) * std::f32::consts::PI).collect();
    let mut lp = 0.0f32;
    let a = brightness.clamp(0.02, 1.0);
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            // The contact: a few hundred microseconds of noise, half-sine shaped.
            let env = if t < burst { (std::f32::consts::PI * t / burst).sin() } else { 0.0 };
            lp += a * (noise.next() * env - lp);
            let mut v = burst_gain * lp;
            for (k, &(f, amp, decay)) in modes.iter().enumerate() {
                // Each mode rings up within the contact time, then decays.
                let attack = (t / burst.max(1e-4)).min(1.0);
                v += amp * attack * (TAU * f * t + phases[k]).sin() * (-t / decay).exp();
            }
            v
        })
        .collect();
    reverb(&mut out, 0.16);
    // Normalise each sound to the same peak; loudness comes from the volume.
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    let fade_n = (0.003 * RATE as f32) as usize;
    let len = out.len();
    for (i, v) in out.iter_mut().enumerate() {
        let fade = ((len - i) as f32 / fade_n as f32).min(1.0);
        *v = (*v / peak * 0.95 * fade).clamp(-1.0, 1.0);
    }
    out
}

/// A small room: a few short feedback echoes (Schroeder combs), mixed low.
fn reverb(x: &mut [f32], mix: f32) {
    let dry = x.to_vec();
    let mut wet = vec![0.0f32; x.len()];
    for (delay_ms, feedback) in [(23.0f32, 0.42f32), (29.7, 0.38), (37.1, 0.33), (41.3, 0.3)] {
        let d = (delay_ms / 1000.0 * RATE as f32) as usize;
        let mut buf = vec![0.0f32; x.len()];
        for i in 0..x.len() {
            let fb = if i >= d { buf[i - d] * feedback } else { 0.0 };
            buf[i] = dry[i] + fb;
            wet[i] += if i >= d { buf[i - d] } else { 0.0 };
        }
    }
    for (v, w) in x.iter_mut().zip(wet) {
        *v += w * mix * 0.25;
    }
}

/// The variant `k` of a set of modes: every frequency a little off.
fn detune(modes: &[Mode], k: u32) -> Vec<Mode> {
    let mut n = Noise(0x9e37_79b9 ^ (k * 7919 + 1));
    modes
        .iter()
        .map(|&(f, a, d)| (f * (1.0 + 0.05 * n.next()), a, d * (1.0 + 0.15 * n.next())))
        .collect()
}

/// Two phenolic balls: a hard, bright click.
const BALL_MODES: &[Mode] = &[
    (2430.0, 1.0, 0.0045),
    (3870.0, 0.7, 0.0032),
    (5310.0, 0.5, 0.0024),
    (7240.0, 0.32, 0.0018),
    (9650.0, 0.2, 0.0012),
];
/// The leather tip on the cue ball: softer, woodier.
const CUE_MODES: &[Mode] =
    &[(1180.0, 0.6, 0.006), (1890.0, 0.45, 0.0045), (2460.0, 0.35, 0.004), (3900.0, 0.18, 0.0025)];
/// Rubber cushion backed by wood: a dull thump.
const CUSHION_MODES: &[Mode] =
    &[(118.0, 1.0, 0.045), (205.0, 0.6, 0.03), (330.0, 0.35, 0.02), (640.0, 0.15, 0.01)];

/// A ball dropping into a pocket: a thud, then knocks against the pocket
/// as it settles.
fn pocket(seed: u32) -> Vec<f32> {
    let parts = [(0.0f32, 1.0f32), (0.075, 0.45), (0.15, 0.25), (0.205, 0.12)];
    let len = (0.45 * RATE as f32) as usize;
    let mut out = vec![0.0f32; len];
    for (k, &(start, gain)) in parts.iter().enumerate() {
        let modes = detune(
            &[(96.0, 1.0, 0.05), (182.0, 0.7, 0.035), (300.0, 0.4, 0.02), (1250.0, 0.12, 0.006)],
            seed * 5 + k as u32,
        );
        let s = strike(seed * 13 + k as u32, 0.2, 0.0015, 0.25, 0.8, &modes);
        let at = (start * RATE as f32) as usize;
        for (i, v) in s.iter().enumerate() {
            if at + i < len {
                out[at + i] += v * gain;
            }
        }
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    out.iter().map(|v| v / peak * 0.95).collect()
}

impl Sounds {
    /// Builds every sound. Missing audio hardware just means silence.
    pub async fn load() -> Self {
        async fn make(samples: Vec<f32>) -> Option<Sound> {
            load_sound_from_bytes(&wav(&samples)).await.ok()
        }
        let mut sounds = Sounds {
            cue: Vec::new(),
            ball: Vec::new(),
            cushion: Vec::new(),
            pocket: Vec::new(),
            next: std::cell::Cell::new(0),
        };
        for k in 0..VARIANTS {
            if let Some(s) =
                make(strike(11 + k, 0.09, 0.0012, 0.35, 0.7, &detune(CUE_MODES, k))).await
            {
                sounds.cue.push(s);
            }
            if let Some(s) =
                make(strike(23 + k, 0.12, 0.0003, 0.9, 0.9, &detune(BALL_MODES, k))).await
            {
                sounds.ball.push(s);
            }
            if let Some(s) =
                make(strike(37 + k, 0.2, 0.004, 0.12, 0.6, &detune(CUSHION_MODES, k))).await
            {
                sounds.cushion.push(s);
            }
            if let Some(s) = make(pocket(41 + k)).await {
                sounds.pocket.push(s);
            }
        }
        sounds
    }

    /// Plays (and clears) the queued impacts at `master` volume (0..=1).
    pub fn play(&self, impacts: &mut Vec<ImpactSound>, master: f32) {
        for impact in impacts.drain(..).take(6) {
            // Loudness follows the impact speed, compressed like the ear.
            let (set, volume) = match impact.kind {
                ImpactKind::Cue => (&self.cue, (impact.speed / 6.0).sqrt()),
                ImpactKind::Ball => (&self.ball, (impact.speed / 3.5).sqrt()),
                ImpactKind::Cushion => (&self.cushion, (impact.speed / 3.0).sqrt() * 0.8),
                ImpactKind::Pocket => (&self.pocket, 0.75),
            };
            let volume = volume.clamp(0.0, 1.0) * master.clamp(0.0, 1.0);
            if set.is_empty() || volume < 0.03 {
                continue;
            }
            let k = self.next.get();
            self.next.set(k.wrapping_add(1));
            play_sound(&set[k as usize % set.len()], PlaySoundParams { looped: false, volume });
        }
    }
}

/// Encodes mono samples as a 16-bit PCM WAV file.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s * 0.9 * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_finite_and_normalised() {
        for s in [
            strike(1, 0.1, 0.0003, 0.9, 0.9, BALL_MODES),
            strike(2, 0.2, 0.004, 0.12, 0.6, CUSHION_MODES),
            pocket(3),
        ] {
            assert!(s.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.5, "too quiet: {peak}");
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 6);
    }
}
