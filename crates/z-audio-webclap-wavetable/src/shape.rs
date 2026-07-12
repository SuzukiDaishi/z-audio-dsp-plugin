//! Waveshaping transfer functions and their first-order ADAA wrappers.
//!
//! Every memoryless shaper in the synth (the filter-drive soft clip and
//! the tanh/hard/fold/sine distortion bus) generates harmonics above
//! Nyquist that fold back as inharmonic hash — the classic "dirty bass"
//! artifact. Instead of oversampling, the audio path runs each shaper
//! through first-order antiderivative anti-aliasing (ADAA): evaluating
//! `(F(x[n]) − F(x[n−1])) / (x[n] − x[n−1])` with the closed-form
//! antiderivative `F` is equivalent to convolving the continuous-time
//! shaped signal with a 1-sample boxcar before resampling, which
//! attenuates the folded band at a fraction of the cost of 2×
//! oversampling. The naive functions stay available for control-rate
//! and reference use; each has an `*_ad` antiderivative normalized to
//! `F(0) = 0`.

use crate::params::{DIST_FOLD, DIST_HARD, DIST_SINE};

/// Cheap tanh-shaped saturator for the filter drive and the tanh
/// distortion mode (Padé-style rational approximation, exact ±1 clamp
/// beyond |x| = 3).
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// One distortion transfer function on a pre-gained sample. Crush is
/// handled separately (it needs the sample-hold state).
#[inline]
pub fn distort(mode: u8, x: f32) -> f32 {
    match mode {
        DIST_HARD => hard_clip(x),
        DIST_FOLD => fold(x),
        DIST_SINE => sine_shape(x),
        _ => soft_clip(x),
    }
}

#[inline]
fn hard_clip(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

/// Triangle foldback: identity in [-1,1], reflects beyond.
#[inline]
fn fold(x: f32) -> f32 {
    let g = x - 4.0 * ((x + 2.0) * 0.25).floor();
    if g.abs() > 1.0 {
        g.signum() * (2.0 - g.abs())
    } else {
        g
    }
}

/// Master bus only, so a real sin() per sample is affordable.
#[inline]
fn sine_shape(x: f32) -> f32 {
    (core::f32::consts::FRAC_PI_2 * x.clamp(-3.0, 3.0)).sin()
}

/// ∫ soft_clip. On |x| ≤ 3, d/dx [x²/18 + (4/3)·ln((x²+3)/3)] =
/// x·(x² + 27)/(9x² + 27); beyond the clamp soft_clip ≡ ±1, so the
/// antiderivative continues linearly. F is even.
#[inline]
fn soft_clip_ad(x: f32) -> f64 {
    let ax = (x.abs() as f64).min(3.0);
    let core = ax * ax / 18.0 + (4.0 / 3.0) * ((ax * ax + 3.0) / 3.0).ln();
    core + (x.abs() as f64 - ax)
}

/// ∫ hard clip: x²/2 inside [-1,1], linear continuation beyond.
#[inline]
fn hard_ad(x: f32) -> f64 {
    let ax = x.abs() as f64;
    if ax <= 1.0 {
        0.5 * ax * ax
    } else {
        ax - 0.5
    }
}

/// ∫ sine_shape: (2/π)·(1 − cos(πx/2)) on |x| ≤ 3; sine_shape(±3) = ∓1,
/// so the tail continues linearly downward. F is even.
#[inline]
fn sine_ad(x: f32) -> f64 {
    let ax = x.abs() as f64;
    if ax <= 3.0 {
        core::f64::consts::FRAC_2_PI * (1.0 - (core::f64::consts::FRAC_PI_2 * ax).cos())
    } else {
        core::f64::consts::FRAC_2_PI - (ax - 3.0)
    }
}

/// ∫ fold. fold is odd with period 4 and zero mean per period, so its
/// antiderivative is the periodic G(g) with g = wrap(x) ∈ [-2, 2):
/// |g| ≤ 1 → g²/2, else 2|g| − g²/2 − 1 (continuous at |g| = 1 and at
/// the wrap point, G(±2) = 1).
#[inline]
fn fold_ad(x: f32) -> f64 {
    let g = (x - 4.0 * ((x + 2.0) * 0.25).floor()) as f64;
    let ag = g.abs();
    if ag <= 1.0 {
        0.5 * g * g
    } else {
        2.0 * ag - 0.5 * g * g - 1.0
    }
}

/// First-order ADAA state — one per audio channel per shaper site.
/// `f1` caches F(x1) in f64: the difference F(x) − F(x1) cancels
/// catastrophically in f32 for small increments.
#[derive(Clone, Copy, Default)]
pub struct Adaa1 {
    x1: f32,
    f1: f64,
}

impl Adaa1 {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    fn tick(&mut self, x: f32, f: fn(f32) -> f32, big_f: fn(f32) -> f64) -> f32 {
        let fx = big_f(x);
        let dx = x - self.x1;
        let y = if dx.abs() > 1.0e-3 {
            ((fx - self.f1) / dx as f64) as f32
        } else {
            // Ill-conditioned difference: fall back to the midpoint,
            // which the ADAA formula converges to as dx → 0.
            f(0.5 * (x + self.x1))
        };
        self.x1 = x;
        self.f1 = fx;
        y
    }

    /// Anti-aliased soft clip (the filter-drive saturator).
    #[inline]
    pub fn soft_clip(&mut self, x: f32) -> f32 {
        self.tick(x, soft_clip, soft_clip_ad)
    }

    /// Anti-aliased `distort` for the shaper modes. Crush never reaches
    /// here — its aliasing is the effect.
    #[inline]
    pub fn distort(&mut self, mode: u8, x: f32) -> f32 {
        match mode {
            DIST_HARD => self.tick(x, hard_clip, hard_ad),
            DIST_FOLD => self.tick(x, fold, fold_ad),
            DIST_SINE => self.tick(x, sine_shape, sine_ad),
            _ => self.tick(x, soft_clip, soft_clip_ad),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::DIST_TANH;

    const MODES: [u8; 4] = [DIST_TANH, DIST_HARD, DIST_FOLD, DIST_SINE];

    #[test]
    fn antiderivatives_match_the_shapers() {
        // Central difference of each F must reproduce f everywhere,
        // including across the clamp/fold boundaries.
        type Pair = (fn(f32) -> f32, fn(f32) -> f64);
        let pairs: [Pair; 4] = [
            (soft_clip, soft_clip_ad),
            (hard_clip, hard_ad),
            (fold, fold_ad),
            (sine_shape, sine_ad),
        ];
        let h = 1.0e-3f32;
        for (k, (f, big_f)) in pairs.iter().enumerate() {
            for i in 0..2000 {
                let x = -6.0 + 12.0 * i as f32 / 1999.0;
                let deriv = ((big_f(x + h) - big_f(x - h)) / (2.0 * h as f64)) as f32;
                assert!(
                    (deriv - f(x)).abs() < 5.0e-3,
                    "pair {k} at x={x}: F' = {deriv}, f = {}",
                    f(x)
                );
            }
        }
    }

    #[test]
    fn adaa_matches_naive_for_slow_signals() {
        // A 20 Hz sine at 48 kHz moves so little per sample that ADAA and
        // the naive shaper must agree closely in every mode.
        for mode in MODES {
            let mut state = Adaa1::default();
            let mut max_diff = 0.0f32;
            for n in 0..48_000 {
                let x = 2.5 * (core::f32::consts::TAU * 20.0 * n as f32 / 48_000.0).sin();
                let y = state.distort(mode, x);
                if n > 0 {
                    max_diff = max_diff.max((y - distort(mode, x)).abs());
                }
            }
            assert!(max_diff < 1.0e-2, "mode {mode}: max diff {max_diff}");
        }
    }

    #[test]
    fn adaa_reduces_fold_aliasing() {
        // A hot 2217 Hz sine through the wavefolder sprays harmonics past
        // Nyquist; measure the folded image of the 13th harmonic
        // (48000 − 13·2217 = 19179 Hz) and demand ADAA cuts its power
        // versus the naive shaper. Relative assertion — never flaky.
        let sr = 48_000.0f64;
        let f0 = 2217.0f64;
        let n = 16_384;
        let goertzel = |buf: &[f32], freq: f64| -> f64 {
            let w = core::f64::consts::TAU * freq / sr;
            let coeff = 2.0 * w.cos();
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for &x in buf {
                let s0 = x as f64 + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            let real = s1 - s2 * w.cos();
            let imag = s2 * w.sin();
            (real * real + imag * imag) / (buf.len() as f64 / 2.0).powi(2)
        };
        let render = |adaa: bool| -> Vec<f32> {
            let mut state = Adaa1::default();
            (0..n + 64)
                .map(|i| {
                    let x = 12.0 * (core::f64::consts::TAU * f0 * i as f64 / sr).sin() as f32;
                    if adaa {
                        state.distort(DIST_FOLD, x)
                    } else {
                        distort(DIST_FOLD, x)
                    }
                })
                .skip(64) // settle the ADAA state
                .collect()
        };
        let alias_freq = sr - 13.0 * f0;
        let naive = goertzel(&render(false), alias_freq);
        let adaa = goertzel(&render(true), alias_freq);
        assert!(
            adaa < naive * 0.5,
            "ADAA must at least halve the folded image: naive={naive} adaa={adaa}"
        );
    }
}
