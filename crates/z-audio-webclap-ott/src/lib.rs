//! Z Audio OTT — Vital/OTT-style 3-band upward+downward compressor.
//! Linkwitz-Riley 4th-order crossovers split the signal into low/mid/high;
//! each band gets a stereo-linked envelope follower and is squeezed toward
//! a fixed internal target from BOTH sides: loud material is compressed
//! down, quiet material is pulled up. Depth scales the whole effect,
//! Time scales the ballistics, and Up/Down weight the two directions.
//!
//! Web ids 940-950 — a fresh block (hyperdim 920s is the previous
//! highest). A future native build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

pub const P_DEPTH: u32 = 940;
pub const P_TIME: u32 = 941;
pub const P_IN_GAIN: u32 = 942;
pub const P_OUT_GAIN: u32 = 943;
pub const P_LOW_GAIN: u32 = 944;
pub const P_MID_GAIN: u32 = 945;
pub const P_HIGH_GAIN: u32 = 946;
pub const P_UPWARD: u32 = 947;
pub const P_DOWNWARD: u32 = 948;
pub const P_XOVER_LOW: u32 = 949;
pub const P_XOVER_HIGH: u32 = 950;

/// Loudness every band is squeezed toward, in dBFS.
const TARGET_DB: f32 = -30.0;
/// Downward slope: 4:1 above the target.
const DOWN_SLOPE: f32 = 1.0 - 1.0 / 4.0;
/// Upward slope: 2:1 below the target.
const UP_SLOPE: f32 = 1.0 - 1.0 / 2.0;
/// Hard cap on gain change in either direction.
const MAX_GAIN_DB: f32 = 24.0;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.ott\0",
    name: b"Z Audio OTT\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"OTT-style 3-band upward/downward compressor with Linkwitz-Riley crossovers\0",
    features: &[b"audio-effect\0", b"compressor\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

fn def(id: u32, name: &'static [u8], min: f64, max: f64, default: f64, stepped: bool) -> ParamDef {
    ParamDef {
        id,
        flags: if stepped {
            PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED
        } else {
            PARAM_IS_AUTOMATABLE
        },
        name,
        module: b"\0",
        min,
        max,
        default,
    }
}

pub fn param_defs() -> Vec<ParamDef> {
    vec![
        def(P_DEPTH, b"Depth\0", 0.0, 1.0, 1.0, false),
        def(P_TIME, b"Time\0", 0.1, 4.0, 1.0, false),
        def(P_IN_GAIN, b"In Gain\0", -24.0, 24.0, 0.0, false),
        def(P_OUT_GAIN, b"Out Gain\0", -24.0, 24.0, 0.0, false),
        def(P_LOW_GAIN, b"Low Gain\0", -12.0, 12.0, 0.0, false),
        def(P_MID_GAIN, b"Mid Gain\0", -12.0, 12.0, 0.0, false),
        def(P_HIGH_GAIN, b"High Gain\0", -12.0, 12.0, 0.0, false),
        def(P_UPWARD, b"Upward\0", 0.0, 1.0, 1.0, false),
        def(P_DOWNWARD, b"Downward\0", 0.0, 1.0, 1.0, false),
        def(P_XOVER_LOW, b"Low X-Over\0", 40.0, 400.0, 120.0, false),
        def(P_XOVER_HIGH, b"High X-Over\0", 1000.0, 8000.0, 2500.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct OttParams {
    pub depth: f32,
    pub time: f32,
    pub in_gain_db: f32,
    pub out_gain_db: f32,
    pub band_gain_db: [f32; 3],
    pub upward: f32,
    pub downward: f32,
    pub xover_low_hz: f32,
    pub xover_high_hz: f32,
}

impl Default for OttParams {
    fn default() -> Self {
        Self {
            depth: 1.0,
            time: 1.0,
            in_gain_db: 0.0,
            out_gain_db: 0.0,
            band_gain_db: [0.0; 3],
            upward: 1.0,
            downward: 1.0,
            xover_low_hz: 120.0,
            xover_high_hz: 2500.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1.0e-6).log10()
}

/// RBJ biquad (transposed direct form II).
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
    fn lowpass(freq: f32, sample_rate: f32) -> Self {
        Self::rbj(freq, sample_rate, false)
    }

    fn highpass(freq: f32, sample_rate: f32) -> Self {
        Self::rbj(freq, sample_rate, true)
    }

    fn rbj(freq: f32, sample_rate: f32, highpass: bool) -> Self {
        let f = freq.clamp(10.0, sample_rate * 0.45);
        let w = core::f32::consts::TAU * f / sample_rate;
        let (sin, cos) = w.sin_cos();
        // Butterworth Q (α = sin/(2Q), Q = 1/√2) — two cascaded stages
        // make a Linkwitz-Riley 4th-order section.
        let alpha = sin * core::f32::consts::FRAC_1_SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if highpass {
            ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0)
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0)
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
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

/// One channel's crossover network: LR4 low split, then LR4 mid/high split.
#[derive(Clone, Copy, Default)]
struct Crossover {
    low_lp: [Biquad; 2],
    low_hp: [Biquad; 2],
    high_lp: [Biquad; 2],
    high_hp: [Biquad; 2],
}

impl Crossover {
    fn configure(&mut self, f_low: f32, f_high: f32, sample_rate: f32) {
        for b in &mut self.low_lp {
            *b = Biquad::lowpass(f_low, sample_rate);
        }
        for b in &mut self.low_hp {
            *b = Biquad::highpass(f_low, sample_rate);
        }
        for b in &mut self.high_lp {
            *b = Biquad::lowpass(f_high, sample_rate);
        }
        for b in &mut self.high_hp {
            *b = Biquad::highpass(f_high, sample_rate);
        }
    }

    /// Split one sample into (low, mid, high).
    #[inline]
    fn split(&mut self, x: f32) -> (f32, f32, f32) {
        let low1 = self.low_lp[0].tick(x);
        let low = self.low_lp[1].tick(low1);
        let rest1 = self.low_hp[0].tick(x);
        let rest = self.low_hp[1].tick(rest1);
        let mid1 = self.high_lp[0].tick(rest);
        let mid = self.high_lp[1].tick(mid1);
        let high1 = self.high_hp[0].tick(rest);
        let high = self.high_hp[1].tick(high1);
        (low, mid, high)
    }

    fn clear(&mut self) {
        for b in self
            .low_lp
            .iter_mut()
            .chain(self.low_hp.iter_mut())
            .chain(self.high_lp.iter_mut())
            .chain(self.high_hp.iter_mut())
        {
            b.clear();
        }
    }
}

pub struct OttEngine {
    params: OttParams,
    sample_rate: f32,
    xover_l: Crossover,
    xover_r: Crossover,
    /// Configured crossover frequencies (rebuilt when the params move).
    configured: (f32, f32),
    /// Per-band stereo-linked envelope (linear peak).
    env: [f32; 3],
}

impl OttEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let p = OttParams::default();
        let mut e = Self {
            params: p,
            sample_rate: sr,
            xover_l: Crossover::default(),
            xover_r: Crossover::default(),
            configured: (0.0, 0.0),
            env: [0.0; 3],
        };
        e.reconfigure();
        e
    }

    fn reconfigure(&mut self) {
        let p = self.params;
        self.xover_l
            .configure(p.xover_low_hz, p.xover_high_hz, self.sample_rate);
        self.xover_r
            .configure(p.xover_low_hz, p.xover_high_hz, self.sample_rate);
        self.configured = (p.xover_low_hz, p.xover_high_hz);
    }

    pub fn params(&self) -> &OttParams {
        &self.params
    }

    pub fn set_params(&mut self, p: OttParams) {
        self.params = p;
        if self.configured != (p.xover_low_hz, p.xover_high_hz) {
            self.reconfigure();
        }
    }

    pub fn reset(&mut self) {
        self.xover_l.clear();
        self.xover_r.clear();
        self.env = [0.0; 3];
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let in_gain = db_to_gain(p.in_gain_db);
        let out_gain = db_to_gain(p.out_gain_db);
        let band_gain = [
            db_to_gain(p.band_gain_db[0]),
            db_to_gain(p.band_gain_db[1]),
            db_to_gain(p.band_gain_db[2]),
        ];
        // OTT-fast ballistics, scaled by the Time knob.
        let atk = 1.0 - (-1.0 / (0.003 * p.time * self.sample_rate)).exp();
        let rel = 1.0 - (-1.0 / (0.080 * p.time * self.sample_rate)).exp();

        for i in 0..out_l.len() {
            let xl = in_l[i] * in_gain;
            let xr = in_r[i] * in_gain;
            let bl = self.xover_l.split(xl);
            let br = self.xover_r.split(xr);
            let bands_l = [bl.0, bl.1, bl.2];
            let bands_r = [br.0, br.1, br.2];

            let mut yl = 0.0f32;
            let mut yr = 0.0f32;
            for b in 0..3 {
                // Stereo-linked peak follower.
                let level = bands_l[b].abs().max(bands_r[b].abs());
                let coeff = if level > self.env[b] { atk } else { rel };
                self.env[b] += (level - self.env[b]) * coeff;

                let level_db = gain_to_db(self.env[b]);
                let mut gain_db = 0.0f32;
                if level_db > TARGET_DB {
                    gain_db -= (level_db - TARGET_DB) * DOWN_SLOPE * p.depth * p.downward;
                } else {
                    gain_db += (TARGET_DB - level_db) * UP_SLOPE * p.depth * p.upward;
                }
                let g = db_to_gain(gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB)) * band_gain[b];
                yl += bands_l[b] * g;
                yr += bands_r[b] * g;
            }
            out_l[i] = yl * out_gain;
            out_r[i] = yr * out_gain;
        }
    }
}

pub fn apply_param(p: &mut OttParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_DEPTH => p.depth = v.clamp(0.0, 1.0),
        P_TIME => p.time = v.clamp(0.1, 4.0),
        P_IN_GAIN => p.in_gain_db = v.clamp(-24.0, 24.0),
        P_OUT_GAIN => p.out_gain_db = v.clamp(-24.0, 24.0),
        P_LOW_GAIN => p.band_gain_db[0] = v.clamp(-12.0, 12.0),
        P_MID_GAIN => p.band_gain_db[1] = v.clamp(-12.0, 12.0),
        P_HIGH_GAIN => p.band_gain_db[2] = v.clamp(-12.0, 12.0),
        P_UPWARD => p.upward = v.clamp(0.0, 1.0),
        P_DOWNWARD => p.downward = v.clamp(0.0, 1.0),
        P_XOVER_LOW => p.xover_low_hz = v.clamp(40.0, 400.0),
        P_XOVER_HIGH => p.xover_high_hz = v.clamp(1000.0, 8000.0),
        _ => {}
    }
}

pub fn param_value(p: &OttParams, id: u32) -> f64 {
    (match id {
        P_DEPTH => p.depth,
        P_TIME => p.time,
        P_IN_GAIN => p.in_gain_db,
        P_OUT_GAIN => p.out_gain_db,
        P_LOW_GAIN => p.band_gain_db[0],
        P_MID_GAIN => p.band_gain_db[1],
        P_HIGH_GAIN => p.band_gain_db[2],
        P_UPWARD => p.upward,
        P_DOWNWARD => p.downward,
        P_XOVER_LOW => p.xover_low_hz,
        P_XOVER_HIGH => p.xover_high_hz,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebOtt {
    engine: OttEngine,
}

impl Plugin for ZAudioWebOtt {
    fn new() -> Self {
        Self {
            engine: OttEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = OttEngine::new(sample_rate as f32);
        self.engine.set_params(params);
        // set_params only reconfigures on a frequency change; force it so
        // the new sample rate always lands in the crossovers.
        self.engine.reconfigure();
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(param_defs)
    }

    fn get_param(&self, id: u32) -> f64 {
        param_value(self.engine.params(), id)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let mut p = *self.engine.params();
        apply_param(&mut p, id, value);
        self.engine.set_params(p);
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        match ctx.stereo_io() {
            Some(io) => {
                self.engine
                    .process(io.input_l, io.input_r, io.output_l, io.output_r);
            }
            None => silence(ctx),
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioWebOtt>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> OttEngine {
        OttEngine::new(48_000.0)
    }

    fn sine(n: usize, freq: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * freq * i as f32 / 48_000.0).sin() * amp)
            .collect()
    }

    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn render(e: &mut OttEngine, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(input, input, &mut l, &mut r);
        (l, r)
    }

    /// RMS of the steady-state tail (skips the ballistics settling).
    fn tail_rms(buf: &[f32]) -> f32 {
        let tail = &buf[buf.len() / 2..];
        (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt()
    }

    fn goertzel(buf: &[f32], freq: f32) -> f64 {
        let w = core::f64::consts::TAU * freq as f64 / 48_000.0;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in buf {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let re = s1 - s2 * w.cos();
        let im = s2 * w.sin();
        re * re + im * im
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 11);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((940..=950).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = OttParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!((param_value(&p, def.id) - def.max).abs() < 1e-6);
        }
    }

    #[test]
    fn zero_depth_is_nearly_transparent() {
        // The LR4 crossover sum is allpass — magnitude within a fraction
        // of a dB across the band.
        let mut e = defaults();
        let mut p = *e.params();
        p.depth = 0.0;
        e.set_params(p);
        for freq in [80.0, 500.0, 1000.0, 5000.0] {
            let input = sine(24_000, freq, 0.3);
            let (l, _) = render(&mut e, &input);
            let ratio = tail_rms(&l) / tail_rms(&input);
            assert!(
                (0.89..=1.12).contains(&ratio),
                "{freq} Hz passthrough ratio {ratio}"
            );
            e.reset();
        }
    }

    #[test]
    fn quiet_material_is_pulled_up() {
        let mut e = defaults();
        let input = sine(48_000, 500.0, 0.005); // ~-46 dBFS, well below target
        let (l, _) = render(&mut e, &input);
        let ratio = tail_rms(&l) / tail_rms(&input);
        assert!(ratio > 2.0, "upward compression must boost: ratio {ratio}");
    }

    #[test]
    fn loud_material_is_compressed_down() {
        let mut e = defaults();
        let input = sine(48_000, 500.0, 0.8); // ~-2 dBFS, well above target
        let (l, _) = render(&mut e, &input);
        let ratio = tail_rms(&l) / tail_rms(&input);
        assert!(ratio < 0.5, "downward compression must duck: ratio {ratio}");
    }

    #[test]
    fn squeeze_reduces_the_dynamic_range() {
        // The gap between a loud and a quiet passage must shrink.
        let mut e = defaults();
        let quiet_in = sine(48_000, 500.0, 0.01);
        let loud_in = sine(48_000, 500.0, 0.8);
        let (quiet_out, _) = render(&mut e, &quiet_in);
        e.reset();
        let (loud_out, _) = render(&mut e, &loud_in);
        let in_range = tail_rms(&loud_in) / tail_rms(&quiet_in);
        let out_range = tail_rms(&loud_out) / tail_rms(&quiet_out);
        assert!(
            out_range < in_range * 0.25,
            "dynamic range must shrink: in {in_range} out {out_range}"
        );
    }

    #[test]
    fn band_gain_shapes_the_spectrum() {
        // 50 Hz sits >1 octave under the 120 Hz crossover so LR4 leakage
        // into the mid band stays negligible.
        let mut input = sine(48_000, 50.0, 0.2);
        let high = sine(48_000, 5_000.0, 0.2);
        for (a, b) in input.iter_mut().zip(high.iter()) {
            *a += b;
        }
        let ratio_with_low_gain = |gain_db: f32| {
            let mut e = defaults();
            let mut p = *e.params();
            p.band_gain_db[0] = gain_db;
            e.set_params(p);
            let (l, _) = render(&mut e, &input);
            let tail = &l[24_000..];
            goertzel(tail, 50.0) / goertzel(tail, 5_000.0)
        };
        let flat = ratio_with_low_gain(0.0);
        let cut = ratio_with_low_gain(-12.0);
        assert!(
            cut < flat * 0.25,
            "low-band cut must remove low energy: flat {flat} cut {cut}"
        );
    }

    #[test]
    fn output_stays_bounded_and_finite_on_noise() {
        let mut e = defaults();
        let mut p = *e.params();
        p.in_gain_db = 24.0;
        p.time = 0.1;
        e.set_params(p);
        let input = noise(96_000, 0.9);
        let (l, r) = render(&mut e, &input);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak.is_finite() && peak < 8.0, "peak {peak}");
    }
}
