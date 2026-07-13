//! Channel-vocoder DSP: a log-spaced bandpass filterbank analyzes the
//! audio input (the modulator) into per-band envelopes, which then gate
//! the same filterbank applied to a polyphonic oscillator carrier.
//!
//! The carrier is MIDI-driven (8 voices) with a free-running fallback
//! oscillator that fades in whenever no note is held, so the effect
//! makes sound with no MIDI connected at all. A white-noise blend adds
//! consonant intelligibility — the noise rides the same carrier
//! filterbank, so the modulator envelopes gate it band by band.

use wclap_plugin::{Smoothed, TAU_FREQ, TAU_GAIN};

pub const MAX_BANDS: usize = 32;
pub const MAX_VOICES: usize = 8;
pub const FREQ_LO: f32 = 80.0;
pub const FREQ_HI: f32 = 12_000.0;

pub const WAVE_SAW: u8 = 0;
pub const WAVE_SQUARE: u8 = 1;
pub const WAVE_PULSE: u8 = 2;

/// Per-voice linear gate ramp — just a click guard; the modulator
/// envelopes do the real shaping.
const VOICE_RAMP_S: f32 = 0.002;
/// Crossfade for the free-run fallback oscillator.
const FREERUN_TAU: f32 = 0.010;
/// Compensates the level lost summing narrow bandpass bands.
const MAKEUP: f32 = 2.0;
/// Headroom scale for the summed carrier voices.
const CARRIER_LEVEL: f32 = 0.25;

#[derive(Clone, Copy)]
pub struct VocoderParams {
    pub bands: usize,
    pub wave: u8,
    pub pitch_hz: f32,
    pub free_run: bool,
    pub noise: f32,
    pub shift: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for VocoderParams {
    fn default() -> Self {
        Self {
            bands: 16,
            wave: WAVE_SAW,
            pitch_hz: 110.0,
            free_run: true,
            noise: 0.15,
            shift: 0.0,
            attack_ms: 5.0,
            release_ms: 80.0,
            mix: 1.0,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// RBJ biquad (transposed direct form II), same shape as the OTT/EQ
/// crates, plus the constant-0dB-peak bandpass the vocoder needs.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// RBJ bandpass, constant 0 dB peak gain.
    fn bandpass(freq: f32, q: f32, sample_rate: f32) -> Self {
        let f = freq.clamp(10.0, sample_rate * 0.45);
        let w = core::f32::consts::TAU * f / sample_rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q.max(0.05));
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            b1: 0.0,
            b2: -alpha / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn clear(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Geometric band layout over [`FREQ_LO`], [`FREQ_HI`]: centers are the
/// geometric means of adjacent edges, and every band spans the same
/// number of octaves, so one Q fits all stages.
pub fn band_centers(bands: usize) -> ([f32; MAX_BANDS], f32) {
    let n = bands.clamp(1, MAX_BANDS);
    let ratio = FREQ_HI / FREQ_LO;
    let mut centers = [0.0f32; MAX_BANDS];
    for (k, c) in centers.iter_mut().enumerate().take(n) {
        let lo = FREQ_LO * ratio.powf(k as f32 / n as f32);
        let hi = FREQ_LO * ratio.powf((k + 1) as f32 / n as f32);
        *c = (lo * hi).sqrt();
    }
    // Constant bandwidth in octaves -> Q per RBJ's bw formula.
    let bw = (ratio.log2()) / n as f32;
    let pow = 2.0f32.powf(bw);
    let q = pow.sqrt() / (pow - 1.0);
    (centers, q)
}

/// Fractional-index envelope lookup with linear interpolation; positions
/// outside `[0, bands - 1]` fade to zero (the formant-shift remap).
pub fn env_at(env: &[f32], bands: usize, pos: f32) -> f32 {
    if bands == 0 || pos <= -1.0 || pos >= bands as f32 {
        return 0.0;
    }
    let i0 = pos.floor();
    let frac = pos - i0;
    let i0 = i0 as isize;
    let get = |i: isize| -> f32 {
        if i < 0 || i >= bands as isize {
            0.0
        } else {
            env[i as usize]
        }
    };
    get(i0) * (1.0 - frac) + get(i0 + 1) * frac
}

/// Naive oscillator for `phase` in [0, 1) — aliasing is masked by the
/// carrier filterbank, same trade-off as the ring mod's carrier.
#[inline]
fn osc_sample(wave: u8, phase: f32) -> f32 {
    let t = phase - phase.floor();
    match wave {
        WAVE_SQUARE => {
            if t < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        WAVE_PULSE => {
            if t < 0.25 {
                1.0
            } else {
                -1.0
            }
        }
        _ => 2.0 * t - 1.0,
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    key: u8,
    gate: bool,
    vel: f32,
    amp: f32,
    phase: f32,
    inc: f32,
    age: u32,
}

impl Voice {
    #[inline]
    fn active(&self) -> bool {
        self.gate || self.amp > 1.0e-4
    }
}

pub struct VocoderEngine {
    params: VocoderParams,
    sample_rate: f32,
    configured_bands: usize,
    mod_bp: [[Biquad; 2]; MAX_BANDS],
    car_bp: [[Biquad; 2]; MAX_BANDS],
    env: [f32; MAX_BANDS],
    voices: [Voice; MAX_VOICES],
    age_counter: u32,
    free_phase: f32,
    free_amp: Smoothed,
    noise_state: u32,
    sm_noise: Smoothed,
    sm_mix: Smoothed,
    sm_out: Smoothed,
    sm_shift: Smoothed,
    snapped: bool,
}

impl VocoderEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |tau: f32, initial: f32| {
            let mut s = Smoothed::new(initial);
            s.configure(sr, tau);
            s
        };
        let mut engine = Self {
            params: VocoderParams::default(),
            sample_rate: sr,
            configured_bands: 0,
            mod_bp: [[Biquad::default(); 2]; MAX_BANDS],
            car_bp: [[Biquad::default(); 2]; MAX_BANDS],
            env: [0.0; MAX_BANDS],
            voices: [Voice::default(); MAX_VOICES],
            age_counter: 0,
            free_phase: 0.0,
            free_amp: smoother(FREERUN_TAU, 0.0),
            noise_state: 0x9E37_79B9,
            sm_noise: smoother(TAU_GAIN, 0.0),
            sm_mix: smoother(TAU_GAIN, 0.0),
            sm_out: smoother(TAU_GAIN, 1.0),
            sm_shift: smoother(TAU_FREQ, 0.0),
            snapped: false,
        };
        engine.reconfigure_filters();
        engine
    }

    pub fn params(&self) -> &VocoderParams {
        &self.params
    }

    pub fn set_params(&mut self, p: VocoderParams) {
        self.params = p;
    }

    pub fn band_count(&self) -> usize {
        self.configured_bands
    }

    /// Modulator band envelopes (pre-shift, linear), for the UI meter.
    pub fn envelopes(&self) -> &[f32] {
        &self.env[..self.configured_bands]
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    pub fn reset(&mut self) {
        for bank in [&mut self.mod_bp, &mut self.car_bp] {
            for pair in bank.iter_mut() {
                for b in pair.iter_mut() {
                    b.clear();
                }
            }
        }
        self.env = [0.0; MAX_BANDS];
        self.voices = [Voice::default(); MAX_VOICES];
        self.free_phase = 0.0;
        self.snapped = false;
    }

    fn reconfigure_filters(&mut self) {
        let bands = self.params.bands.clamp(1, MAX_BANDS);
        let (centers, q) = band_centers(bands);
        for k in 0..bands {
            let bp = Biquad::bandpass(centers[k], q, self.sample_rate);
            self.mod_bp[k] = [bp; 2];
            self.car_bp[k] = [bp; 2];
        }
        self.env = [0.0; MAX_BANDS];
        self.configured_bands = bands;
    }

    pub fn note_on(&mut self, key: u8, velocity: f32) {
        let vel = velocity.clamp(0.0, 1.0).max(0.05);
        let slot = self
            .voices
            .iter()
            .position(|v| v.active() && v.key == key)
            .or_else(|| self.voices.iter().position(|v| !v.active()))
            .unwrap_or_else(|| {
                // Steal the quietest voice, oldest on ties.
                let mut best = 0;
                for i in 1..MAX_VOICES {
                    let (a, b) = (&self.voices[i], &self.voices[best]);
                    if (a.amp, a.age) < (b.amp, b.age) {
                        best = i;
                    }
                }
                best
            });
        let v = &mut self.voices[slot];
        let retrigger = !(v.active() && v.key == key);
        if retrigger {
            v.phase = 0.0;
        }
        v.key = key;
        v.gate = true;
        v.vel = vel;
        v.inc = 440.0 * 2.0f32.powf((key as f32 - 69.0) / 12.0) / self.sample_rate;
        self.age_counter = self.age_counter.wrapping_add(1);
        v.age = self.age_counter;
    }

    pub fn note_off(&mut self, key: u8) {
        for v in &mut self.voices {
            if v.gate && v.key == key {
                v.gate = false;
            }
        }
    }

    #[inline]
    fn next_noise(&mut self) -> f32 {
        // xorshift32 -> [-1, 1)
        let mut x = self.noise_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise_state = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        if p.bands.clamp(1, MAX_BANDS) != self.configured_bands {
            self.reconfigure_filters();
        }
        let bands = self.configured_bands;

        self.sm_noise.set_target(p.noise);
        self.sm_mix.set_target(p.mix);
        self.sm_out.set_target(db_to_gain(p.output_db));
        self.sm_shift.set_target(p.shift);
        if !self.snapped {
            self.sm_noise.snap();
            self.sm_mix.snap();
            self.sm_out.snap();
            self.sm_shift.snap();
            self.snapped = true;
        }

        let env_coeff = |ms: f32| {
            let samples = (ms.max(0.01) * 0.001 * self.sample_rate).max(1.0);
            1.0 - (-1.0 / samples).exp()
        };
        let atk = env_coeff(p.attack_ms);
        let rel = env_coeff(p.release_ms);
        let ramp = 1.0 / (VOICE_RAMP_S * self.sample_rate);
        let free_inc = p.pitch_hz.max(0.1) / self.sample_rate;

        let any_gated = self.voices.iter().any(|v| v.gate);
        self.free_amp
            .set_target(if p.free_run && !any_gated { 1.0 } else { 0.0 });

        for i in 0..out_l.len().min(out_r.len()) {
            let mono = 0.5 * (in_l[i] + in_r[i]);

            // Carrier: MIDI voices + free-run fallback, with a matching
            // white-noise blend gated by overall carrier activity so the
            // noise never drones on after all voices release.
            let mut osc = 0.0f32;
            let mut activity = 0.0f32;
            for v in &mut self.voices {
                let target = if v.gate { v.vel } else { 0.0 };
                if v.amp < target {
                    v.amp = (v.amp + ramp).min(target);
                } else if v.amp > target {
                    v.amp = (v.amp - ramp).max(target);
                }
                if !v.active() {
                    continue;
                }
                osc += osc_sample(p.wave, v.phase) * v.amp;
                activity += v.amp;
                v.phase += v.inc;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
            }
            let fa = self.free_amp.tick();
            if fa > 1.0e-4 {
                osc += osc_sample(p.wave, self.free_phase) * fa;
                activity += fa;
            }
            self.free_phase += free_inc;
            if self.free_phase >= 1.0 {
                self.free_phase -= 1.0;
            }

            let n_amt = self.sm_noise.tick();
            let noise = self.next_noise() * activity.min(1.0);
            let carrier = (osc * (1.0 - n_amt) + noise * n_amt) * CARRIER_LEVEL;

            let shift = self.sm_shift.tick();
            let mix = self.sm_mix.tick();
            let out_gain = self.sm_out.tick();

            let mut wet = 0.0f32;
            for k in 0..bands {
                let m1 = self.mod_bp[k][0].tick(mono);
                let m = self.mod_bp[k][1].tick(m1);
                let level = m.abs();
                let coeff = if level > self.env[k] { atk } else { rel };
                self.env[k] += (level - self.env[k]) * coeff;
                let c1 = self.car_bp[k][0].tick(carrier);
                let c = self.car_bp[k][1].tick(c1);
                wet += c * env_at(&self.env, bands, k as f32 - shift);
            }
            wet *= MAKEUP;

            let dry = 1.0 - mix;
            out_l[i] = (in_l[i] * dry + wet * mix) * out_gain;
            out_r[i] = (in_r[i] * dry + wet * mix) * out_gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_centers_are_log_spaced_and_bounded() {
        for bands in [8usize, 16, 32] {
            let (centers, q) = band_centers(bands);
            assert!(q > 0.5, "q {q}");
            assert!(centers[0] >= FREQ_LO);
            assert!(centers[bands - 1] <= FREQ_HI);
            let ratio = centers[1] / centers[0];
            for k in 1..bands {
                let r = centers[k] / centers[k - 1];
                assert!(
                    (r - ratio).abs() < 1.0e-3 * ratio,
                    "band {k}: {r} vs {ratio}"
                );
                assert!(centers[k] > centers[k - 1]);
            }
        }
    }

    #[test]
    fn env_at_interpolates_fractionally_and_fades_at_the_edges() {
        let mut env = [0.0f32; MAX_BANDS];
        env[1] = 1.0;
        assert_eq!(env_at(&env, 16, 1.0), 1.0);
        assert!((env_at(&env, 16, 0.5) - 0.5).abs() < 1.0e-6);
        assert!((env_at(&env, 16, 1.5) - 0.5).abs() < 1.0e-6);
        // Below band 0 the missing neighbor counts as zero.
        env[1] = 0.0;
        env[0] = 1.0;
        assert!((env_at(&env, 16, -0.5) - 0.5).abs() < 1.0e-6);
        assert_eq!(env_at(&env, 16, -1.0), 0.0);
        assert_eq!(env_at(&env, 16, 16.0), 0.0);
        assert_eq!(env_at(&env, 0, 0.0), 0.0);
    }

    #[test]
    fn voice_stealing_keeps_at_most_max_voices() {
        let mut e = VocoderEngine::new(48_000.0);
        for key in 40..52u8 {
            e.note_on(key, 0.8);
        }
        assert_eq!(e.active_voices(), MAX_VOICES);
        // The most recent key must still hold a voice.
        e.note_off(51);
        assert_eq!(e.active_voices(), MAX_VOICES - 1);
    }
}
