//! Z Audio Hyper Dimension — Serum-style two-stage widener. **Hyper** is
//! a unison-like detune cloud: up to 7 LFO-swept delay taps panned
//! alternately left/right, blended against the dry signal. **Dimension**
//! is an SDD-320-flavored expander: an antiphase pair of slowly-modulated
//! short delays cross-mixed with inverted polarity for width. Hyper feeds
//! Dimension in series; both stages have independent wet controls.
//!
//! Web ids 920-926 — a fresh block (gate 900s is the previous highest).
//! A future native build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_GAIN, TAU_TIME,
};

pub const P_HYPER_RATE: u32 = 920;
pub const P_HYPER_DETUNE: u32 = 921;
pub const P_HYPER_UNISON: u32 = 922;
pub const P_HYPER_WET: u32 = 923;
pub const P_DIM_SIZE: u32 = 924;
pub const P_DIM_WET: u32 = 925;
pub const P_OUTPUT: u32 = 926;

pub const MAX_UNISON: usize = 7;

/// Hyper taps swing 10..18 ms, Dimension sits at 3..23 ms (+1.5 ms LFO);
/// 64 ms of line leaves comfortable headroom for both.
const MAX_DELAY_MS: f32 = 64.0;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.hyperdim\0",
    name: b"Z Audio Hyper Dimension\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Hyper unison widener + dimension expander (Serum-style two-stage stereoizer)\0",
    features: &[b"audio-effect\0", b"stereo\0"],
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
        def(P_HYPER_RATE, b"Hyper Rate\0", 0.05, 8.0, 1.2, false),
        def(P_HYPER_DETUNE, b"Hyper Detune\0", 0.0, 1.0, 0.5, false),
        def(P_HYPER_UNISON, b"Hyper Unison\0", 1.0, 7.0, 4.0, true),
        def(P_HYPER_WET, b"Hyper Wet\0", 0.0, 1.0, 0.5, false),
        def(P_DIM_SIZE, b"Dim Size\0", 0.0, 1.0, 0.5, false),
        def(P_DIM_WET, b"Dim Wet\0", 0.0, 1.0, 0.3, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct HyperDimParams {
    pub hyper_rate_hz: f32,
    pub hyper_detune: f32,
    pub hyper_unison: u8,
    pub hyper_wet: f32,
    pub dim_size: f32,
    pub dim_wet: f32,
    pub output_db: f32,
}

impl Default for HyperDimParams {
    fn default() -> Self {
        Self {
            hyper_rate_hz: 1.2,
            hyper_detune: 0.5,
            hyper_unison: 4,
            hyper_wet: 0.5,
            dim_size: 0.5,
            dim_wet: 0.3,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Circular delay line with linear-interpolated fractional reads.
struct DelayLine {
    buf: Vec<f32>,
    write: usize,
}

impl DelayLine {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(4)],
            write: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.write = 0;
    }

    /// Sample `delay` samples behind the write head (clamped to the buffer).
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let d = delay.clamp(1.0, (len - 2) as f32);
        let whole = d as usize;
        let frac = d - whole as f32;
        let i0 = (self.write + len - whole) % len;
        let i1 = (self.write + len - whole - 1) % len;
        self.buf[i0] * (1.0 - frac) + self.buf[i1] * frac
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) % self.buf.len();
    }
}

/// Delay of hyper voice `v` of `n` in milliseconds at LFO phase `phase`
/// (the UI mirrors this for its visualization).
pub fn hyper_delay_ms(detune: f32, voice: usize, n: usize, phase: f32) -> f32 {
    // Spread the voice phases evenly around the LFO cycle; deeper detune
    // widens the sweep (a faster-moving tap = a bigger pitch deviation).
    let offset = voice as f32 / n.max(1) as f32;
    let lfo = (core::f32::consts::TAU * (phase + offset)).sin();
    10.0 + voice as f32 * 1.3 + detune * 7.0 * (0.5 + 0.5 * lfo)
}

pub struct HyperDimEngine {
    params: HyperDimParams,
    sample_rate: f32,
    hyper_l: DelayLine,
    hyper_r: DelayLine,
    dim_l: DelayLine,
    dim_r: DelayLine,
    hyper_phase: f32,
    dim_phase: f32,
    /// Anti-zipper smoothing: dim size and detune move delay taps
    /// (slewed slowly), the wets and output are gain-like.
    sm_dim_base: Smoothed,
    sm_detune: Smoothed,
    sm_hyper_wet: Smoothed,
    sm_dim_wet: Smoothed,
    sm_out: Smoothed,
    snapped: bool,
}

impl HyperDimEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let len = (MAX_DELAY_MS * 0.001 * sr) as usize + 4;
        let smoother = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(sr, tau);
            s
        };
        Self {
            params: HyperDimParams::default(),
            sample_rate: sr,
            hyper_l: DelayLine::new(len),
            hyper_r: DelayLine::new(len),
            dim_l: DelayLine::new(len),
            dim_r: DelayLine::new(len),
            hyper_phase: 0.0,
            dim_phase: 0.0,
            sm_dim_base: smoother(TAU_TIME),
            sm_detune: smoother(TAU_TIME),
            sm_hyper_wet: smoother(TAU_GAIN),
            sm_dim_wet: smoother(TAU_GAIN),
            sm_out: smoother(TAU_GAIN),
            snapped: false,
        }
    }

    pub fn params(&self) -> &HyperDimParams {
        &self.params
    }

    pub fn set_params(&mut self, p: HyperDimParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.hyper_l.clear();
        self.hyper_r.clear();
        self.dim_l.clear();
        self.dim_r.clear();
        self.hyper_phase = 0.0;
        self.dim_phase = 0.0;
        self.snapped = false;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let n = (p.hyper_unison as usize).clamp(1, MAX_UNISON);
        let hyper_inc = p.hyper_rate_hz / self.sample_rate;
        // Dimension's BBD-style sweep is slow and fixed — the size knob
        // moves the base delay, not the rate.
        let dim_inc = 0.25 / self.sample_rate;
        let ms = 0.001 * self.sample_rate;
        self.sm_dim_base.set_target((3.0 + p.dim_size * 20.0) * ms);
        self.sm_detune.set_target(p.hyper_detune);
        self.sm_hyper_wet.set_target(p.hyper_wet);
        self.sm_dim_wet.set_target(p.dim_wet);
        self.sm_out.set_target(db_to_gain(p.output_db));
        if !self.snapped {
            self.sm_dim_base.snap();
            self.sm_detune.snap();
            self.sm_hyper_wet.snap();
            self.sm_dim_wet.snap();
            self.sm_out.snap();
            self.snapped = true;
        }
        let dim_swing = 1.5 * ms;
        let voice_gain = 1.0 / (n as f32).sqrt();

        for i in 0..out_l.len() {
            let dim_base = self.sm_dim_base.tick();
            let detune = self.sm_detune.tick();
            let hyper_wet = self.sm_hyper_wet.tick();
            let dim_wet = self.sm_dim_wet.tick();
            let out_gain = self.sm_out.tick();
            self.hyper_l.push(in_l[i]);
            self.hyper_r.push(in_r[i]);

            // ---- Hyper: unison cloud of swept taps, alternately panned.
            let mut wet_l = 0.0f32;
            let mut wet_r = 0.0f32;
            for v in 0..n {
                let d = hyper_delay_ms(detune, v, n, self.hyper_phase) * ms;
                // Alternate pan: even voices lean left, odd lean right.
                let (gl, gr) = if n == 1 {
                    (0.7071, 0.7071)
                } else if v % 2 == 0 {
                    (0.85, 0.35)
                } else {
                    (0.35, 0.85)
                };
                wet_l += self.hyper_l.read(d) * gl;
                wet_r += self.hyper_r.read(d) * gr;
            }
            wet_l *= voice_gain;
            wet_r *= voice_gain;
            let hy_l = in_l[i] + (wet_l - in_l[i]) * hyper_wet;
            let hy_r = in_r[i] + (wet_r - in_r[i]) * hyper_wet;

            // ---- Dimension: antiphase modulated pair, cross-mixed with
            // inverted polarity (the classic SDD-320 width trick).
            let dim_lfo = (core::f32::consts::TAU * self.dim_phase).sin();
            self.dim_l.push(hy_l);
            self.dim_r.push(hy_r);
            let tap_l = self.dim_l.read(dim_base + dim_swing * dim_lfo);
            let tap_r = self.dim_r.read(dim_base - dim_swing * dim_lfo);
            let out_sl = hy_l + (tap_l - 0.7 * tap_r) * dim_wet;
            let out_sr = hy_r + (tap_r - 0.7 * tap_l) * dim_wet;

            out_l[i] = out_sl * out_gain;
            out_r[i] = out_sr * out_gain;

            self.hyper_phase += hyper_inc;
            if self.hyper_phase >= 1.0 {
                self.hyper_phase -= 1.0;
            }
            self.dim_phase += dim_inc;
            if self.dim_phase >= 1.0 {
                self.dim_phase -= 1.0;
            }
        }
    }
}

pub fn apply_param(p: &mut HyperDimParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_HYPER_RATE => p.hyper_rate_hz = v.clamp(0.05, 8.0),
        P_HYPER_DETUNE => p.hyper_detune = v.clamp(0.0, 1.0),
        P_HYPER_UNISON => p.hyper_unison = v.clamp(1.0, MAX_UNISON as f32).round() as u8,
        P_HYPER_WET => p.hyper_wet = v.clamp(0.0, 1.0),
        P_DIM_SIZE => p.dim_size = v.clamp(0.0, 1.0),
        P_DIM_WET => p.dim_wet = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &HyperDimParams, id: u32) -> f64 {
    (match id {
        P_HYPER_RATE => p.hyper_rate_hz,
        P_HYPER_DETUNE => p.hyper_detune,
        P_HYPER_UNISON => p.hyper_unison as f32,
        P_HYPER_WET => p.hyper_wet,
        P_DIM_SIZE => p.dim_size,
        P_DIM_WET => p.dim_wet,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebHyperDim {
    engine: HyperDimEngine,
}

impl Plugin for ZAudioWebHyperDim {
    fn new() -> Self {
        Self {
            engine: HyperDimEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = HyperDimEngine::new(sample_rate as f32);
        self.engine.set_params(params);
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
    init_plugin::<ZAudioWebHyperDim>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> HyperDimEngine {
        HyperDimEngine::new(48_000.0)
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

    fn render(e: &mut HyperDimEngine, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(input, input, &mut l, &mut r);
        (l, r)
    }

    fn stereo_diff(l: &[f32], r: &[f32]) -> f32 {
        l.iter().zip(r.iter()).map(|(a, b)| (a - b).abs()).sum()
    }

    #[test]
    fn output_gain_jump_is_smoothed() {
        // Jump output +24 dB and dim size mid-render: the output must
        // glide, not step.
        let mut e = defaults();
        let n = 9_600;
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.02).sin() * 0.4).collect();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        let half = n / 2;
        e.process(
            &input[..half],
            &input[..half],
            &mut l[..half],
            &mut r[..half],
        );
        let mut p = *e.params();
        p.output_db = 24.0;
        p.dim_size = 1.0;
        e.set_params(p);
        let last = l[half - 1];
        let (l2, r2) = (&mut l[half..], &mut r[half..]);
        e.process(&input[half..], &input[half..], l2, r2);
        let settle = 2_000;
        let jump_delta = l2[..settle]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold((l2[0] - last).abs(), f32::max);
        let steady_after = l2[settle..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            jump_delta < steady_after * 1.5 + 0.02,
            "zipper step {jump_delta} vs post-jump steady {steady_after}"
        );
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 7);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((920..=926).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = HyperDimParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!((param_value(&p, def.id) - def.max).abs() < 1e-6);
        }
    }

    #[test]
    fn both_stages_off_is_a_clean_passthrough() {
        let mut e = defaults();
        let mut p = *e.params();
        p.hyper_wet = 0.0;
        p.dim_wet = 0.0;
        e.set_params(p);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let (l, r) = render(&mut e, &input);
        for ((a, b), c) in input.iter().zip(l.iter()).zip(r.iter()) {
            assert!((a - b).abs() < 1e-6);
            assert!((a - c).abs() < 1e-6);
        }
    }

    #[test]
    fn hyper_widens_a_mono_input() {
        let input = noise(24_000, 0.4);
        let mut e = defaults();
        let mut p = *e.params();
        p.hyper_wet = 1.0;
        p.dim_wet = 0.0;
        p.hyper_unison = 6;
        e.set_params(p);
        let (l, r) = render(&mut e, &input);
        assert!(
            stereo_diff(&l, &r) > 10.0,
            "hyper must decorrelate the channels"
        );
    }

    #[test]
    fn single_voice_hyper_stays_centered() {
        let input = noise(24_000, 0.4);
        let mut e = defaults();
        let mut p = *e.params();
        p.hyper_wet = 1.0;
        p.dim_wet = 0.0;
        p.hyper_unison = 1;
        e.set_params(p);
        let (l, r) = render(&mut e, &input);
        assert!(
            stereo_diff(&l, &r) < 1e-3,
            "a single centered voice must stay mono"
        );
    }

    #[test]
    fn dimension_widens_and_size_changes_the_sound() {
        let input = noise(24_000, 0.4);
        let render_with_size = |size: f32| {
            let mut e = defaults();
            let mut p = *e.params();
            p.hyper_wet = 0.0;
            p.dim_wet = 1.0;
            p.dim_size = size;
            e.set_params(p);
            render(&mut e, &input)
        };
        let (l, r) = render_with_size(0.5);
        assert!(
            stereo_diff(&l, &r) > 10.0,
            "dimension must decorrelate the channels"
        );
        let (l2, _) = render_with_size(1.0);
        let diff: f32 = l.iter().zip(l2.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 10.0, "size must change the sound");
    }

    #[test]
    fn output_stays_bounded_on_noise_at_full_settings() {
        let mut e = defaults();
        let mut p = *e.params();
        p.hyper_wet = 1.0;
        p.hyper_detune = 1.0;
        p.hyper_unison = 7;
        p.dim_wet = 1.0;
        p.dim_size = 1.0;
        e.set_params(p);
        let input = noise(96_000, 0.5);
        let (l, r) = render(&mut e, &input);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak.is_finite() && peak < 4.0, "peak {peak}");
    }
}
