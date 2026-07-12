//! Z Audio Saturator — warm tanh-style saturation with even-harmonic
//! "warmth", a tilt tone control, automatic level compensation, dry/wet
//! mix and output trim. A DC blocker removes the offset the even-harmonic
//! term introduces under signal.
//!
//! Web ids 660-664 — a fresh block (ring mod 620s, distortion 640s).
//! A future native build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, TAU_GAIN,
};

pub const P_DRIVE: u32 = 660;
pub const P_WARMTH: u32 = 661;
pub const P_TONE: u32 = 662;
pub const P_MIX: u32 = 663;
pub const P_OUTPUT: u32 = 664;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.saturator\0",
    name: b"Z Audio Saturator\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Warm level-compensated saturation with tilt tone\0",
    features: &[b"audio-effect\0", b"distortion\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

fn def(id: u32, name: &'static [u8], min: f64, max: f64, default: f64) -> ParamDef {
    ParamDef {
        id,
        flags: PARAM_IS_AUTOMATABLE,
        name,
        module: b"\0",
        min,
        max,
        default,
    }
}

pub fn param_defs() -> Vec<ParamDef> {
    vec![
        def(P_DRIVE, b"Drive\0", 0.0, 24.0, 6.0),
        def(P_WARMTH, b"Warmth\0", 0.0, 1.0, 0.3),
        def(P_TONE, b"Tone\0", -1.0, 1.0, 0.0),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 1.0),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0),
    ]
}

#[derive(Clone, Copy)]
pub struct SaturatorParams {
    pub drive_db: f32,
    pub warmth: f32,
    /// Tilt tone: -1 dark … +1 bright.
    pub tone: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for SaturatorParams {
    fn default() -> Self {
        Self {
            drive_db: 6.0,
            warmth: 0.3,
            tone: 0.0,
            mix: 1.0,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// The static saturation curve (drive already applied via `g`), exposed so
/// the UI mirrors it. `tanh(g)` normalization keeps a full-scale input at
/// full scale regardless of drive.
#[inline]
pub fn saturate(g: f32, warmth: f32, x: f32) -> f32 {
    let driven = g * x;
    // The bias term is even in the input, so positive and negative
    // half-waves clip differently — that asymmetry is what adds the warm
    // even harmonics. tanh² keeps it bounded and zero at rest.
    let bias = warmth * 0.4 * driven.tanh() * driven.tanh();
    (driven + bias).tanh() / g.tanh().max(1e-3)
}

/// Tilt-tone crossover + DC blocker per channel.
#[derive(Default, Clone, Copy)]
struct ChannelState {
    lp: f32,
    dc_x: f32,
    dc_y: f32,
}

impl ChannelState {
    #[inline]
    fn tick(&mut self, x: f32, lp_a: f32, low_gain: f32, high_gain: f32) -> f32 {
        let dc = x - self.dc_x + 0.995 * self.dc_y;
        self.dc_x = x;
        self.dc_y = dc;
        self.lp += lp_a * (dc - self.lp);
        self.lp * low_gain + (dc - self.lp) * high_gain
    }
}

pub struct SaturatorEngine {
    params: SaturatorParams,
    sample_rate: f32,
    left: ChannelState,
    right: ChannelState,
    /// Anti-zipper smoothing: drive/warmth/tilt gains/mix/output are all
    /// gain-like.
    sm_drive: Smoothed,
    sm_warmth: Smoothed,
    sm_low_gain: Smoothed,
    sm_high_gain: Smoothed,
    sm_mix: Smoothed,
    sm_out: Smoothed,
    snapped: bool,
}

impl SaturatorEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(sr, tau);
            s
        };
        Self {
            params: SaturatorParams::default(),
            sample_rate: sr,
            left: ChannelState::default(),
            right: ChannelState::default(),
            sm_drive: smoother(TAU_GAIN),
            sm_warmth: smoother(TAU_GAIN),
            sm_low_gain: smoother(TAU_GAIN),
            sm_high_gain: smoother(TAU_GAIN),
            sm_mix: smoother(TAU_GAIN),
            sm_out: smoother(TAU_GAIN),
            snapped: false,
        }
    }

    pub fn params(&self) -> &SaturatorParams {
        &self.params
    }

    pub fn set_params(&mut self, p: SaturatorParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.left = ChannelState::default();
        self.right = ChannelState::default();
        self.snapped = false;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        // Tilt pivot at 800 Hz, ±6 dB swing at the extremes.
        let lp_a = 1.0 - (-core::f32::consts::TAU * 800.0 / self.sample_rate).exp();
        self.sm_drive.set_target(db_to_gain(p.drive_db));
        self.sm_warmth.set_target(p.warmth);
        self.sm_low_gain.set_target(db_to_gain(-6.0 * p.tone));
        self.sm_high_gain.set_target(db_to_gain(6.0 * p.tone));
        self.sm_mix.set_target(p.mix);
        self.sm_out.set_target(db_to_gain(p.output_db));
        if !self.snapped {
            self.sm_drive.snap();
            self.sm_warmth.snap();
            self.sm_low_gain.snap();
            self.sm_high_gain.snap();
            self.sm_mix.snap();
            self.sm_out.snap();
            self.snapped = true;
        }
        for i in 0..out_l.len() {
            let g = self.sm_drive.tick().max(1.0);
            let warmth = self.sm_warmth.tick();
            let low_gain = self.sm_low_gain.tick();
            let high_gain = self.sm_high_gain.tick();
            let mix = self.sm_mix.tick();
            let dry = 1.0 - mix;
            let out_gain = self.sm_out.tick();
            let wet_l = self
                .left
                .tick(saturate(g, warmth, in_l[i]), lp_a, low_gain, high_gain);
            let wet_r = self
                .right
                .tick(saturate(g, warmth, in_r[i]), lp_a, low_gain, high_gain);
            out_l[i] = (in_l[i] * dry + wet_l * mix) * out_gain;
            out_r[i] = (in_r[i] * dry + wet_r * mix) * out_gain;
        }
    }
}

pub fn apply_param(p: &mut SaturatorParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_DRIVE => p.drive_db = v.clamp(0.0, 24.0),
        P_WARMTH => p.warmth = v.clamp(0.0, 1.0),
        P_TONE => p.tone = v.clamp(-1.0, 1.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &SaturatorParams, id: u32) -> f64 {
    (match id {
        P_DRIVE => p.drive_db,
        P_WARMTH => p.warmth,
        P_TONE => p.tone,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebSaturator {
    engine: SaturatorEngine,
}

impl Plugin for ZAudioWebSaturator {
    fn new() -> Self {
        Self {
            engine: SaturatorEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = SaturatorEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebSaturator>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_gain_jump_is_smoothed() {
        // Jump output +24 dB and drive mid-render: the output must glide,
        // not step.
        let mut e = SaturatorEngine::new(48_000.0);
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
        p.drive_db = 20.0;
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
            assert!((660..=664).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn saturation_is_level_compensated_at_full_scale() {
        for drive_db in [0.0, 6.0, 12.0, 24.0] {
            let g = db_to_gain(drive_db).max(1.0);
            let y = saturate(g, 0.0, 1.0);
            assert!((y - 1.0).abs() < 0.05, "drive {drive_db}: y {y}");
        }
    }

    #[test]
    fn warmth_skews_the_curve_asymmetric() {
        let g = db_to_gain(12.0);
        let up = saturate(g, 1.0, 0.5);
        let down = saturate(g, 1.0, -0.5);
        assert!(
            (up + down).abs() > 0.01,
            "curve should be asymmetric: {up} vs {down}"
        );
    }

    #[test]
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = SaturatorEngine::new(48_000.0);
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
    fn output_stays_bounded_under_heavy_drive_and_warmth() {
        let mut e = SaturatorEngine::new(48_000.0);
        let mut p = *e.params();
        p.drive_db = 24.0;
        p.warmth = 1.0;
        e.set_params(p);
        let input: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.2).sin()).collect();
        let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
        e.process(&input, &input, &mut l, &mut r);
        let peak = l.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 2.0, "peak {peak}");
        assert!(l.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn tilt_brightens_or_darkens() {
        let hf: Vec<f32> = (0..4_800).map(|i| (i as f32 * 1.5).sin() * 0.4).collect();
        let energy = |buf: &[f32]| buf.iter().map(|s| s * s).sum::<f32>();
        let run = |tone: f32| {
            let mut e = SaturatorEngine::new(48_000.0);
            let mut p = *e.params();
            p.tone = tone;
            e.set_params(p);
            let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
            e.process(&hf, &hf, &mut l, &mut r);
            energy(&l)
        };
        let dark = run(-1.0);
        let flat = run(0.0);
        let bright = run(1.0);
        assert!(dark < flat && flat < bright, "{dark} {flat} {bright}");
    }

    #[test]
    fn settles_back_to_silence_after_a_burst() {
        let mut e = SaturatorEngine::new(48_000.0);
        let mut p = *e.params();
        p.drive_db = 18.0;
        p.warmth = 1.0;
        e.set_params(p);
        let mut input: Vec<f32> = (0..2_400).map(|i| (i as f32 * 0.2).sin()).collect();
        input.extend(std::iter::repeat(0.0).take(9_600));
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(&input, &input, &mut l, &mut r);
        let tail_peak = l[n - 480..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail_peak < 0.01, "tail peak {tail_peak}");
    }
}
