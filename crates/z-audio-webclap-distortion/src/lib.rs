//! Z Audio Distortion — stereo waveshaping distortion with four shapes
//! (soft clip / hard clip / wavefold / asymmetric), a post low-pass tone
//! control, dry/wet mix and output trim. A DC blocker keeps the
//! asymmetric shape (even harmonics) from leaking offset downstream.
//!
//! Web ids 640-644 — a fresh block (ring mod uses 620s). A future native
//! build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_FREQ, TAU_GAIN,
};

pub const P_DRIVE: u32 = 640;
pub const P_TYPE: u32 = 641;
pub const P_TONE: u32 = 642;
pub const P_MIX: u32 = 643;
pub const P_OUTPUT: u32 = 644;

pub const TYPE_SOFT: u8 = 0;
pub const TYPE_HARD: u8 = 1;
pub const TYPE_FOLD: u8 = 2;
pub const TYPE_ASYM: u8 = 3;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.distortion\0",
    name: b"Z Audio Distortion\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Waveshaping distortion: soft/hard/fold/asym with tone\0",
    features: &[b"audio-effect\0", b"distortion\0", b"stereo\0"],
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
        def(P_DRIVE, b"Drive\0", 0.0, 36.0, 12.0, false),
        def(P_TYPE, b"Type\0", 0.0, 3.0, 0.0, true),
        def(P_TONE, b"Tone\0", 200.0, 20_000.0, 20_000.0, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 1.0, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct DistortionParams {
    pub drive_db: f32,
    pub shape: u8,
    pub tone_hz: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for DistortionParams {
    fn default() -> Self {
        Self {
            drive_db: 12.0,
            shape: TYPE_SOFT,
            tone_hz: 20_000.0,
            mix: 1.0,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// The waveshaper itself, exposed so the UI mirrors it exactly.
#[inline]
pub fn shape(shape: u8, x: f32) -> f32 {
    match shape {
        TYPE_HARD => x.clamp(-1.0, 1.0),
        TYPE_FOLD => (core::f32::consts::FRAC_PI_2 * x).sin(),
        TYPE_ASYM => {
            // A level-dependent bias (even in x) skews positive and negative
            // half-waves differently, producing even harmonics while staying
            // bounded, monotonic, and zero at rest. The DC blocker removes
            // the offset this creates under signal.
            let bias = 0.35 * x.tanh() * x.tanh();
            (x + bias).tanh()
        }
        _ => x.tanh(),
    }
}

/// One-pole low-pass + DC blocker per channel.
#[derive(Default, Clone, Copy)]
struct ChannelState {
    lp: f32,
    dc_x: f32,
    dc_y: f32,
}

impl ChannelState {
    #[inline]
    fn tick(&mut self, x: f32, lp_a: f32) -> f32 {
        // DC blocker: y[n] = x[n] - x[n-1] + R * y[n-1]
        let dc = x - self.dc_x + 0.995 * self.dc_y;
        self.dc_x = x;
        self.dc_y = dc;
        self.lp += lp_a * (dc - self.lp);
        self.lp
    }
}

pub struct DistortionEngine {
    params: DistortionParams,
    sample_rate: f32,
    left: ChannelState,
    right: ChannelState,
    /// Anti-zipper smoothing: drive/mix/output are gain-like, the tone
    /// low-pass coefficient sweeps a frequency.
    sm_drive: Smoothed,
    sm_lp_a: Smoothed,
    sm_mix: Smoothed,
    sm_out: Smoothed,
    snapped: bool,
}

impl DistortionEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(sr, tau);
            s
        };
        Self {
            params: DistortionParams::default(),
            sample_rate: sr,
            left: ChannelState::default(),
            right: ChannelState::default(),
            sm_drive: smoother(TAU_GAIN),
            sm_lp_a: smoother(TAU_FREQ),
            sm_mix: smoother(TAU_GAIN),
            sm_out: smoother(TAU_GAIN),
            snapped: false,
        }
    }

    pub fn params(&self) -> &DistortionParams {
        &self.params
    }

    pub fn set_params(&mut self, p: DistortionParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.left = ChannelState::default();
        self.right = ChannelState::default();
        self.snapped = false;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let fc = p.tone_hz.min(self.sample_rate * 0.45);
        self.sm_drive.set_target(db_to_gain(p.drive_db));
        self.sm_lp_a
            .set_target(1.0 - (-core::f32::consts::TAU * fc / self.sample_rate).exp());
        self.sm_mix.set_target(p.mix);
        self.sm_out.set_target(db_to_gain(p.output_db));
        if !self.snapped {
            self.sm_drive.snap();
            self.sm_lp_a.snap();
            self.sm_mix.snap();
            self.sm_out.snap();
            self.snapped = true;
        }
        for i in 0..out_l.len() {
            let drive = self.sm_drive.tick();
            let lp_a = self.sm_lp_a.tick();
            let mix = self.sm_mix.tick();
            let dry = 1.0 - mix;
            let out_gain = self.sm_out.tick();
            let wet_l = self.left.tick(shape(p.shape, in_l[i] * drive), lp_a);
            let wet_r = self.right.tick(shape(p.shape, in_r[i] * drive), lp_a);
            out_l[i] = (in_l[i] * dry + wet_l * mix) * out_gain;
            out_r[i] = (in_r[i] * dry + wet_r * mix) * out_gain;
        }
    }
}

pub fn apply_param(p: &mut DistortionParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_DRIVE => p.drive_db = v.clamp(0.0, 36.0),
        P_TYPE => p.shape = v.clamp(0.0, 3.0).round() as u8,
        P_TONE => p.tone_hz = v.clamp(200.0, 20_000.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &DistortionParams, id: u32) -> f64 {
    (match id {
        P_DRIVE => p.drive_db,
        P_TYPE => p.shape as f32,
        P_TONE => p.tone_hz,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebDistortion {
    engine: DistortionEngine,
}

impl Plugin for ZAudioWebDistortion {
    fn new() -> Self {
        Self {
            engine: DistortionEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = DistortionEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebDistortion>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_gain_jump_is_smoothed() {
        // Jump output +24 dB and drive mid-render: the output must glide,
        // not step.
        let mut e = DistortionEngine::new(48_000.0);
        let n = 9_600;
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.02).sin() * 0.5).collect();
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
        p.drive_db = 30.0;
        e.set_params(p);
        let last = l[half - 1];
        let (l2, r2) = (&mut l[half..], &mut r[half..]);
        e.process(&input[half..], &input[half..], l2, r2);
        // The transition region must never step harder than the post-jump
        // steady signal itself moves (unsmoothed, the boundary step is the
        // full +24 dB jump in one sample).
        let settle = 2_000; // ~40 ms >> every tau involved
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
        assert_eq!(defs.len(), 5);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((640..=644).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn every_shape_is_bounded_and_quiet_at_zero() {
        for s in [TYPE_SOFT, TYPE_HARD, TYPE_FOLD, TYPE_ASYM] {
            assert!(shape(s, 0.0).abs() < 1e-6, "shape {s} not zero at rest");
            for i in -100..=100 {
                let x = i as f32 * 0.16; // up to ±16
                assert!(shape(s, x).abs() <= 1.2, "shape {s} unbounded at {x}");
            }
        }
    }

    #[test]
    fn drive_saturates_a_hot_sine() {
        let mut e = DistortionEngine::new(48_000.0);
        let mut p = *e.params();
        p.drive_db = 24.0;
        e.set_params(p);
        let input: Vec<f32> = (0..960).map(|i| (i as f32 * 0.13).sin() * 0.8).collect();
        let (mut l, mut r) = (vec![0.0; 960], vec![0.0; 960]);
        e.process(&input, &input, &mut l, &mut r);
        let peak = l.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= 1.2, "peak {peak}");
        // A driven tanh flattens the tops: lots of samples near the rails.
        let near_rail = l.iter().filter(|s| s.abs() > 0.9).count();
        assert!(near_rail > 200, "only {near_rail} samples near the rails");
    }

    #[test]
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = DistortionEngine::new(48_000.0);
        let mut p = *e.params();
        p.mix = 0.0;
        e.set_params(p);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn tone_darkens_the_output() {
        let hf: Vec<f32> = (0..4_800).map(|i| (i as f32 * 2.0).sin() * 0.5).collect();
        let energy = |buf: &[f32]| buf.iter().map(|s| s * s).sum::<f32>();

        let mut open = DistortionEngine::new(48_000.0);
        let (mut l1, mut r1) = (vec![0.0; 4_800], vec![0.0; 4_800]);
        open.process(&hf, &hf, &mut l1, &mut r1);

        let mut dark = DistortionEngine::new(48_000.0);
        let mut p = *dark.params();
        p.tone_hz = 500.0;
        dark.set_params(p);
        let (mut l2, mut r2) = (vec![0.0; 4_800], vec![0.0; 4_800]);
        dark.process(&hf, &hf, &mut l2, &mut r2);

        assert!(energy(&l2) < energy(&l1) * 0.5);
    }

    #[test]
    fn asym_shape_settles_back_to_silence() {
        // After a burst, the DC blocker must pull the output back to ~0.
        let mut e = DistortionEngine::new(48_000.0);
        let mut p = *e.params();
        p.shape = TYPE_ASYM;
        p.drive_db = 24.0;
        e.set_params(p);
        let mut input: Vec<f32> = (0..2_400).map(|i| (i as f32 * 0.2).sin()).collect();
        input.extend(std::iter::repeat(0.0).take(9_600));
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(&input, &input, &mut l, &mut r);
        let tail = &l[n - 480..];
        let tail_peak = tail.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail_peak < 0.01, "tail peak {tail_peak}");
    }
}
