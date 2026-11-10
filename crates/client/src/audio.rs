//! Synthesised sound effects (no audio files needed).

use std::f32::consts::TAU;

use macroquad::audio::{PlaySoundParams, Sound, load_sound_from_bytes, play_sound};

use crate::game::{ImpactKind, ImpactSound};

const RATE: u32 = 44_100;

pub struct Sounds {
    cue: Option<Sound>,
    ball: Option<Sound>,
    cushion: Option<Sound>,
    pocket: Option<Sound>,
}

impl Sounds {
    /// Builds every sound. Missing audio hardware just means silence.
    pub async fn load() -> Self {
        async fn make(samples: Vec<f32>) -> Option<Sound> {
            load_sound_from_bytes(&wav(&samples)).await.ok()
        }
        Sounds {
            cue: make(synth(0.06, 11, |t, n| {
                (0.6 * (TAU * 1650.0 * t).sin() + 0.4 * (TAU * 920.0 * t).sin() + 0.5 * n)
                    * (-t / 0.006).exp()
            }))
            .await,
            ball: make(synth(0.07, 23, |t, n| {
                (0.55 * (TAU * 2950.0 * t).sin() + 0.35 * (TAU * 4180.0 * t).sin() + 0.25 * n)
                    * (-t / 0.009).exp()
            }))
            .await,
            cushion: make(synth(0.14, 37, |t, n| {
                (0.7 * (TAU * 170.0 * t).sin() + 0.4 * (TAU * 95.0 * t).sin() + 0.2 * n)
                    * (-t / 0.035).exp()
            }))
            .await,
            pocket: make(synth(0.4, 41, |t, n| {
                [0.0f32, 0.07, 0.16]
                    .iter()
                    .enumerate()
                    .map(|(i, &start)| {
                        let local = t - start;
                        if local < 0.0 {
                            return 0.0;
                        }
                        (1.0 / (1.0 + i as f32))
                            * ((TAU * 125.0 * local).sin()
                                + 0.5 * (TAU * 240.0 * local).sin()
                                + 0.3 * n)
                            * (-local / 0.04).exp()
                    })
                    .sum()
            }))
            .await,
        }
    }

    /// Plays (and clears) the queued impacts at `master` volume (0..=1).
    pub fn play(&self, impacts: &mut Vec<ImpactSound>, master: f32) {
        for impact in impacts.drain(..).take(6) {
            let (sound, volume) = match impact.kind {
                ImpactKind::Cue => (&self.cue, impact.speed / 6.0),
                ImpactKind::Ball => (&self.ball, impact.speed / 3.0),
                ImpactKind::Cushion => (&self.cushion, impact.speed / 3.5),
                ImpactKind::Pocket => (&self.pocket, 0.8),
            };
            let volume = volume.clamp(0.0, 1.0) * master.clamp(0.0, 1.0);
            if let Some(sound) = sound
                && volume >= 0.02
            {
                play_sound(sound, PlaySoundParams { looped: false, volume });
            }
        }
    }
}

fn synth(duration: f32, seed: u32, f: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let n = (duration * RATE as f32) as usize;
    let mut state = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = state as f32 / u32::MAX as f32 * 2.0 - 1.0;
            let t = i as f32 / RATE as f32;
            let fade = ((n - i) as f32 / (0.004 * RATE as f32)).min(1.0);
            (f(t, noise) * fade).clamp(-1.0, 1.0)
        })
        .collect()
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
    fn wav_header_is_valid() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 6);
    }
}
