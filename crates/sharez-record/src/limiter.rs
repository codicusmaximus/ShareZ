//! A peak limiter, so desktop audio and the microphone together never clip.
//!
//! The mix is kept in floating point, where it can go past full scale without distorting, and the
//! limiter brings it back under [`CEILING`]: the gain drops the instant a sample would go over and
//! recovers smoothly afterwards, so loud moments get quieter instead of crackling and quiet ones
//! are left alone.

/// The highest level a sample may reach: −1 dBFS, leaving the encoder some headroom.
pub(crate) const CEILING: f32 = 0.891;
/// How long the gain takes to recover (most of the way) after a peak.
const RELEASE_SECONDS: f32 = 0.2;

/// The sample format the limiter works on, as caps.
pub(crate) const RATE: u32 = 48_000;
pub(crate) const CHANNELS: usize = 2;

#[derive(Debug, Clone)]
pub(crate) struct Limiter {
    gain: f32,
    release: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Self {
            gain: 1.0,
            #[allow(clippy::cast_precision_loss)]
            release: 1.0 - (-1.0 / (RELEASE_SECONDS * RATE as f32)).exp(),
        }
    }
}

impl Limiter {
    /// Limits interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.as_chunks_mut::<CHANNELS>().0 {
            let peak = frame.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            self.gain += (1.0 - self.gain) * self.release;
            if peak * self.gain > CEILING {
                self.gain = CEILING / peak;
            }
            for s in frame {
                // The clamp only catches rounding.
                *s = (*s * self.gain).clamp(-CEILING, CEILING);
            }
        }
    }

    /// Limits little-endian `f32` samples in place, as they come in a buffer.
    pub fn process_bytes(&mut self, bytes: &mut [u8]) {
        let mut samples: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        self.process(&mut samples);
        for (b, s) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(samples) {
            *b = s.to_le_bytes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let s = amplitude * (i as f32 * 0.05).sin();
                [s, -s]
            })
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn quiet_audio_is_untouched() {
        let original = tone(0.5, 4800);
        let mut s = original.clone();
        Limiter::default().process(&mut s);
        assert_eq!(s, original);
    }

    #[test]
    fn a_loud_mix_never_goes_over_the_ceiling() {
        // Two full-scale sources added together.
        let mut s = tone(2.0, 48_000);
        Limiter::default().process(&mut s);
        assert!(peak(&s) <= CEILING, "peak {}", peak(&s));
        assert!(peak(&s) > CEILING * 0.95, "too quiet: {}", peak(&s));
    }

    #[test]
    fn gain_recovers_after_a_peak() {
        let mut limiter = Limiter::default();
        let mut bang = tone(4.0, 200);
        limiter.process(&mut bang);
        let mut after = tone(0.3, 48_000);
        limiter.process(&mut after);
        // A second after the bang, quiet audio plays at (nearly) its own level again.
        let tail = &after[after.len() - 4800..];
        assert!(peak(tail) > 0.29, "still ducked: {}", peak(tail));
    }

    #[test]
    fn bytes_round_trip() {
        let mut bytes: Vec<u8> = [0.25_f32, -3.0]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        Limiter::default().process_bytes(&mut bytes);
        let right = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert!((right.abs() - CEILING).abs() < 1e-6);
    }
}
