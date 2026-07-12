//! One-pole parameter smoothing (anti-zipper).
//!
//! UI knob drags arrive as a stream of stepped param values, one per
//! process block; engines that apply them raw produce audible zipper
//! noise (and a hard click on a single large jump). Every plugin in the
//! workspace routes gain/mix/frequency-like params through [`Smoothed`]
//! so changes glide over a few milliseconds instead.
//!
//! Mirrors `z_audio_dsp::math::SmoothedParam`, re-implemented here
//! because this crate is the one dependency every WCLAP plugin already
//! shares — and it is `no_std`, so the coefficient uses the algebraic
//! form `x / (1 + x)` (with `x = 1 / (tau · rate)`) instead of
//! `1 − exp(−x)`. The two agree within a few percent over every
//! rate/tau combination used here, which is far below audibility.
//!
//! Two invariants engines rely on for bit-exact tests:
//! - a **settled** smoother's [`Smoothed::tick`] returns exactly
//!   `current` (the delta is literally `0.0`), and
//! - [`Smoothed::snap`] lands on the pending target with no transition
//!   (the "first block after construction/reset" idiom).

/// Time constant for gain-like params: output/master gain, mix,
/// feedback, depth, drive, levels, trims.
pub const TAU_GAIN: f32 = 0.005;
/// Time constant for frequency-like params: filter cutoff/center, tone,
/// Q, crossover frequencies.
pub const TAU_FREQ: f32 = 0.020;
/// Time constant for delay-time-like params, where the value moves a
/// read position through a buffer and must slew, not step.
pub const TAU_TIME: f32 = 0.050;

/// One-pole smoothed parameter value.
#[derive(Debug, Clone, Copy)]
pub struct Smoothed {
    current: f32,
    target: f32,
    coeff: f32,
}

impl Smoothed {
    /// A smoother resting at `initial` that jumps instantly until
    /// [`Smoothed::configure`] sets a real time constant.
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            coeff: 1.0,
        }
    }

    /// Set the tick rate and time constant. `update_rate` is how often
    /// [`Smoothed::tick`] is called per second: the sample rate for
    /// per-sample smoothing, or `sample_rate / CHUNK` for chunked
    /// smoothing. Non-positive `tau_seconds` disables smoothing.
    pub fn configure(&mut self, update_rate: f32, tau_seconds: f32) {
        if tau_seconds <= 0.0 {
            self.coeff = 1.0;
        } else {
            // no_std stand-in for 1 - exp(-x); see module docs.
            let x = 1.0 / (tau_seconds * update_rate.max(1.0));
            self.coeff = x / (1.0 + x);
        }
    }

    /// Set the value the smoother glides toward.
    pub fn set_target(&mut self, v: f32) {
        self.target = v;
    }

    /// Jump to `v` with no transition.
    pub fn set_immediate(&mut self, v: f32) {
        self.current = v;
        self.target = v;
    }

    /// Land on the pending target with no transition — the first-block
    /// idiom, so a freshly built/reset engine renders its configured
    /// params exactly instead of sweeping in from stale values.
    pub fn snap(&mut self) {
        self.current = self.target;
    }

    /// Advance one step toward the target and return the new value.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        self.current += (self.target - self.current) * self.coeff;
        self.current
    }

    /// The value as of the last tick.
    #[inline]
    pub fn current(&self) -> f32 {
        self.current
    }

    /// The value the smoother is gliding toward.
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether current is within `eps` of the target.
    #[inline]
    pub fn is_settled(&self, eps: f32) -> bool {
        (self.target - self.current).abs() <= eps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_monotonically_to_the_target() {
        let mut s = Smoothed::new(0.0);
        s.configure(48_000.0, TAU_GAIN);
        s.set_target(1.0);
        let mut prev = 0.0f32;
        for _ in 0..48_000 {
            let v = s.tick();
            assert!(v >= prev && v <= 1.0, "non-monotone: {prev} -> {v}");
            prev = v;
        }
        // ~1s at tau 5ms: fully settled for practical purposes.
        assert!((prev - 1.0).abs() < 1.0e-4, "settled at {prev}");
    }

    #[test]
    fn settled_tick_is_exactly_a_no_op() {
        // Bit-exactness guard: engines assert exact transparency when
        // params never move, which requires the delta to be literally 0.
        let mut s = Smoothed::new(0.25);
        s.configure(48_000.0, TAU_GAIN);
        for _ in 0..64 {
            assert_eq!(s.tick(), 0.25);
        }
        s.set_target(0.5);
        s.snap();
        for _ in 0..64 {
            assert_eq!(s.tick(), 0.5);
        }
    }

    #[test]
    fn set_immediate_and_zero_tau_bypass_smoothing() {
        let mut s = Smoothed::new(0.0);
        s.configure(48_000.0, TAU_GAIN);
        s.set_immediate(0.7);
        assert_eq!(s.tick(), 0.7);
        s.configure(48_000.0, 0.0);
        s.set_target(-1.5);
        assert_eq!(s.tick(), -1.5);
    }

    #[test]
    fn coefficient_tracks_the_update_rate() {
        // Chunked smoothing (rate/32) must land ~as fast in wall-clock
        // time as per-sample smoothing.
        let steps_to_half = |rate: f32| -> f32 {
            let mut s = Smoothed::new(0.0);
            s.configure(rate, TAU_GAIN);
            s.set_target(1.0);
            let mut n = 0;
            while s.tick() < 0.5 {
                n += 1;
            }
            n as f32 / rate
        };
        let per_sample = steps_to_half(48_000.0);
        let chunked = steps_to_half(48_000.0 / 32.0);
        assert!(
            (per_sample - chunked).abs() < 0.01,
            "half-life differs: {per_sample}s vs {chunked}s"
        );
    }
}
