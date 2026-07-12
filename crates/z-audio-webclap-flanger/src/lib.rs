//! Z Audio Flanger — stereo flanger: a short modulated delay line with
//! bipolar feedback, a shared sine LFO with stereo spread, dry/wet mix
//! and output trim. Classic topology: the delayed tap recirculates into
//! the delay-line input, the same tap is the wet signal.
//!
//! Web ids 840-846 — a fresh block (ring mod 620s, distortion 640s). A
//! future native build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_GAIN, TAU_TIME,
};

pub const P_MANUAL: u32 = 840;
pub const P_RATE: u32 = 841;
pub const P_DEPTH: u32 = 842;
pub const P_FEEDBACK: u32 = 843;
pub const P_SPREAD: u32 = 844;
pub const P_MIX: u32 = 845;
pub const P_OUTPUT: u32 = 846;

/// Delay-line capacity: manual max (10 ms) swings up to ×1.9 (19 ms),
/// 32 ms leaves comfortable headroom.
const MAX_DELAY_MS: f32 = 32.0;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.flanger\0",
    name: b"Z Audio Flanger\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Stereo flanger with bipolar feedback and stereo spread\0",
    features: &[b"audio-effect\0", b"flanger\0", b"stereo\0"],
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
        def(P_MANUAL, b"Manual\0", 0.5, 10.0, 2.0, false),
        def(P_RATE, b"Rate\0", 0.02, 5.0, 0.3, false),
        def(P_DEPTH, b"Depth\0", 0.0, 1.0, 0.7, false),
        def(P_FEEDBACK, b"Feedback\0", -0.95, 0.95, 0.5, false),
        def(P_SPREAD, b"Spread\0", 0.0, 1.0, 0.5, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 0.5, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct FlangerParams {
    pub manual_ms: f32,
    pub rate_hz: f32,
    pub depth: f32,
    pub feedback: f32,
    pub spread: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for FlangerParams {
    fn default() -> Self {
        Self {
            manual_ms: 2.0,
            rate_hz: 0.3,
            depth: 0.7,
            feedback: 0.5,
            spread: 0.5,
            mix: 0.5,
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

pub struct FlangerEngine {
    params: FlangerParams,
    sample_rate: f32,
    left: DelayLine,
    right: DelayLine,
    lfo_phase: f32,
    /// Anti-zipper smoothing: manual/depth/spread move the read taps
    /// (slewed slowly), feedback/mix/output are gain-like.
    sm_base: Smoothed,
    sm_swing: Smoothed,
    sm_spread: Smoothed,
    sm_feedback: Smoothed,
    sm_mix: Smoothed,
    sm_out: Smoothed,
    snapped: bool,
}

impl FlangerEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let len = (MAX_DELAY_MS * 0.001 * sr) as usize + 4;
        let smoother = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(sr, tau);
            s
        };
        Self {
            params: FlangerParams::default(),
            sample_rate: sr,
            left: DelayLine::new(len),
            right: DelayLine::new(len),
            lfo_phase: 0.0,
            sm_base: smoother(TAU_TIME),
            sm_swing: smoother(TAU_TIME),
            sm_spread: smoother(TAU_TIME),
            sm_feedback: smoother(TAU_GAIN),
            sm_mix: smoother(TAU_GAIN),
            sm_out: smoother(TAU_GAIN),
            snapped: false,
        }
    }

    pub fn params(&self) -> &FlangerParams {
        &self.params
    }

    pub fn set_params(&mut self, p: FlangerParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.left.clear();
        self.right.clear();
        self.lfo_phase = 0.0;
        self.snapped = false;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let inc = p.rate_hz / self.sample_rate;
        // The LFO swings the delay between manual*(1-depth*0.9) and
        // manual*(1+depth*0.9); the right channel runs spread*0.5 cycles late.
        self.sm_base.set_target(p.manual_ms * 0.001 * self.sample_rate);
        self.sm_swing.set_target(p.depth * 0.9);
        self.sm_spread.set_target(p.spread);
        self.sm_feedback.set_target(p.feedback);
        self.sm_mix.set_target(p.mix);
        self.sm_out.set_target(db_to_gain(p.output_db));
        if !self.snapped {
            self.sm_base.snap();
            self.sm_swing.snap();
            self.sm_spread.snap();
            self.sm_feedback.snap();
            self.sm_mix.snap();
            self.sm_out.snap();
            self.snapped = true;
        }
        for i in 0..out_l.len() {
            let base = self.sm_base.tick();
            let swing = self.sm_swing.tick();
            let spread = self.sm_spread.tick();
            let feedback = self.sm_feedback.tick();
            let mix = self.sm_mix.tick();
            let dry = 1.0 - mix;
            let out_gain = self.sm_out.tick();
            let lfo_l = (core::f32::consts::TAU * self.lfo_phase).sin();
            let lfo_r = (core::f32::consts::TAU * (self.lfo_phase + spread * 0.5)).sin();
            let tap_l = self.left.read(base * (1.0 + swing * lfo_l));
            let tap_r = self.right.read(base * (1.0 + swing * lfo_r));
            // Classic flanger: the tap recirculates into the line input.
            self.left.push(in_l[i] + tap_l * feedback);
            self.right.push(in_r[i] + tap_r * feedback);
            out_l[i] = (in_l[i] * dry + tap_l * mix) * out_gain;
            out_r[i] = (in_r[i] * dry + tap_r * mix) * out_gain;
            self.lfo_phase += inc;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
    }
}

pub fn apply_param(p: &mut FlangerParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_MANUAL => p.manual_ms = v.clamp(0.5, 10.0),
        P_RATE => p.rate_hz = v.clamp(0.02, 5.0),
        P_DEPTH => p.depth = v.clamp(0.0, 1.0),
        P_FEEDBACK => p.feedback = v.clamp(-0.95, 0.95),
        P_SPREAD => p.spread = v.clamp(0.0, 1.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &FlangerParams, id: u32) -> f64 {
    (match id {
        P_MANUAL => p.manual_ms,
        P_RATE => p.rate_hz,
        P_DEPTH => p.depth,
        P_FEEDBACK => p.feedback,
        P_SPREAD => p.spread,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebFlanger {
    engine: FlangerEngine,
}

impl Plugin for ZAudioWebFlanger {
    fn new() -> Self {
        Self {
            engine: FlangerEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = FlangerEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebFlanger>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> FlangerEngine {
        FlangerEngine::new(48_000.0)
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

    fn render(e: &mut FlangerEngine, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(input, input, &mut l, &mut r);
        (l, r)
    }

    #[test]
    fn output_gain_jump_is_smoothed() {
        // Jump output +24 dB and manual delay mid-render: the output must
        // glide, not step.
        let mut e = defaults();
        let n = 9_600;
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.02).sin() * 0.5).collect();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        let half = n / 2;
        e.process(&input[..half], &input[..half], &mut l[..half], &mut r[..half]);
        let mut p = *e.params();
        p.output_db = 24.0;
        p.manual_ms = 9.0;
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
        assert_eq!(defs.len(), 7);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((840..=846).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = FlangerParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!((param_value(&p, def.id) - def.max).abs() < 1e-6);
        }
    }

    #[test]
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = defaults();
        let mut p = *e.params();
        p.mix = 0.0;
        e.set_params(p);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let (l, _r) = render(&mut e, &input);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn full_wet_flanged_sine_differs_from_the_input() {
        let mut e = defaults();
        let mut p = *e.params();
        p.mix = 1.0;
        p.rate_hz = 2.0;
        e.set_params(p);
        let input: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        let (l, _r) = render(&mut e, &input);
        let diff: f32 = input.iter().zip(l.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 10.0, "flanged output too close to dry, diff {diff}");
    }

    #[test]
    fn bipolar_feedback_polarity_changes_the_sound() {
        let input = noise(9_600, 0.4);
        let mut pos = defaults();
        let mut p = *pos.params();
        p.feedback = 0.9;
        pos.set_params(p);
        let (l_pos, _) = render(&mut pos, &input);

        let mut neg = defaults();
        p.feedback = -0.9;
        neg.set_params(p);
        let (l_neg, _) = render(&mut neg, &input);

        let diff: f32 = l_pos
            .iter()
            .zip(l_neg.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(diff > 10.0, "feedback polarity had no effect, diff {diff}");
    }

    #[test]
    fn spread_decorrelates_the_channels() {
        let input = noise(9_600, 0.4);

        let mut wide = defaults();
        let mut p = *wide.params();
        p.spread = 1.0;
        p.rate_hz = 2.0;
        wide.set_params(p);
        let (l, r) = render(&mut wide, &input);
        let diff: f32 = l.iter().zip(r.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 1.0, "spread=1 should decorrelate, diff {diff}");

        let mut mono = defaults();
        p.spread = 0.0;
        mono.set_params(p);
        let (l, r) = render(&mut mono, &input);
        for (a, b) in l.iter().zip(r.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "spread=0 should keep channels identical"
            );
        }
    }

    #[test]
    fn max_feedback_stays_bounded_over_a_long_render() {
        let mut e = defaults();
        let mut p = *e.params();
        p.feedback = 0.95;
        p.depth = 1.0;
        p.rate_hz = 5.0;
        p.mix = 1.0;
        e.set_params(p);
        let input = noise(96_000, 0.2); // 2 s at 48 kHz
        let (l, r) = render(&mut e, &input);
        let peak = l
            .iter()
            .chain(r.iter())
            .fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak.is_finite() && peak < 4.0, "peak {peak}");
    }

    #[test]
    fn impulse_response_is_a_comb_with_the_first_echo_at_manual() {
        let mut e = defaults();
        let mut p = *e.params();
        p.manual_ms = 2.0; // 96 samples at 48 kHz
        p.depth = 0.0;
        p.feedback = 0.5;
        p.mix = 1.0;
        e.set_params(p);
        let mut input = vec![0.0f32; 480];
        input[0] = 1.0;
        let (l, _r) = render(&mut e, &input);
        // Nothing before the first tap...
        let early: f32 = l[..90].iter().map(|s| s.abs()).sum();
        assert!(early < 1e-3, "energy before the first echo: {early}");
        // ...a full-strength echo at manual delay, then a feedback-scaled one.
        assert!(l[96].abs() > 0.9, "first echo {}", l[96]);
        assert!((l[192] - 0.5).abs() < 0.05, "second echo {}", l[192]);
    }
}
