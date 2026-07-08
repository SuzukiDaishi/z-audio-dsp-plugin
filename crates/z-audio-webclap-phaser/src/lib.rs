//! Z Audio Phaser — multi-stage stereo phaser: a cascade of up to 12
//! first-order allpass filters per channel swept by a shared sine LFO
//! (with stereo spread), feedback around the cascade, dry/wet mix and
//! output trim. Equal dry/wet mixing turns the allpass phase shift into
//! the classic moving notches.
//!
//! Web ids 860-867 — a fresh block (flanger 840s). A future native build
//! must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

pub const P_STAGES: u32 = 860;
pub const P_RATE: u32 = 861;
pub const P_CENTER: u32 = 862;
pub const P_DEPTH: u32 = 863;
pub const P_FEEDBACK: u32 = 864;
pub const P_SPREAD: u32 = 865;
pub const P_MIX: u32 = 866;
pub const P_OUTPUT: u32 = 867;

/// `P_STAGES` counts allpass *pairs* (1..=6), so up to 12 actual stages.
const MAX_STAGES: usize = 12;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.phaser\0",
    name: b"Z Audio Phaser\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Multi-stage stereo phaser with feedback and spread\0",
    features: &[b"audio-effect\0", b"phaser\0", b"stereo\0"],
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
        def(P_STAGES, b"Stages\0", 1.0, 6.0, 3.0, true),
        def(P_RATE, b"Rate\0", 0.02, 5.0, 0.4, false),
        def(P_CENTER, b"Center\0", 100.0, 8_000.0, 1_000.0, false),
        def(P_DEPTH, b"Depth\0", 0.0, 1.0, 0.7, false),
        def(P_FEEDBACK, b"Feedback\0", 0.0, 0.9, 0.3, false),
        def(P_SPREAD, b"Spread\0", 0.0, 1.0, 0.5, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 0.5, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct PhaserParams {
    /// Allpass pairs, 1..=6 (actual stage count is `pairs * 2`).
    pub pairs: u8,
    pub rate_hz: f32,
    pub center_hz: f32,
    pub depth: f32,
    pub feedback: f32,
    pub spread: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for PhaserParams {
    fn default() -> Self {
        Self {
            pairs: 3,
            rate_hz: 0.4,
            center_hz: 1_000.0,
            depth: 0.7,
            feedback: 0.3,
            spread: 0.5,
            mix: 0.5,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// One channel: allpass states plus the feedback tap (last chain output).
#[derive(Clone, Copy, Default)]
struct PhaserChannel {
    z: [f32; MAX_STAGES],
    fb: f32,
}

impl PhaserChannel {
    /// Run `stages` first-order allpasses sharing coefficient `a`:
    /// y[n] = a*x[n] + x[n-1] - a*y[n-1], kept as one state z per stage
    /// (transposed form: y = a*x + z; z = x - a*y).
    #[inline]
    fn tick(&mut self, x: f32, a: f32, stages: usize, feedback: f32) -> f32 {
        let mut s = x + self.fb * feedback;
        for z in self.z[..stages].iter_mut() {
            let y = a * s + *z;
            *z = s - a * y;
            s = y;
        }
        self.fb = s;
        s
    }
}

pub struct PhaserEngine {
    params: PhaserParams,
    sample_rate: f32,
    left: PhaserChannel,
    right: PhaserChannel,
    lfo_phase: f32,
}

impl PhaserEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            params: PhaserParams::default(),
            sample_rate: sample_rate.max(1.0),
            left: PhaserChannel::default(),
            right: PhaserChannel::default(),
            lfo_phase: 0.0,
        }
    }

    pub fn params(&self) -> &PhaserParams {
        &self.params
    }

    pub fn set_params(&mut self, p: PhaserParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.left = PhaserChannel::default();
        self.right = PhaserChannel::default();
        self.lfo_phase = 0.0;
    }

    /// Allpass coefficient a = (t-1)/(t+1), t = tan(pi*f/fs), for a sweep
    /// frequency f = center * 2^(depth*2*sin) clamped to 30..fs*0.45.
    #[inline]
    fn coeff(&self, lfo: f32) -> f32 {
        let p = self.params;
        let f = (p.center_hz * (p.depth * 2.0 * lfo).exp2()).clamp(30.0, self.sample_rate * 0.45);
        let t = (core::f32::consts::PI * f / self.sample_rate).tan();
        (t - 1.0) / (t + 1.0)
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let stages = (p.pairs.clamp(1, 6) as usize) * 2;
        let inc = p.rate_hz / self.sample_rate;
        let dry = 1.0 - p.mix;
        let out_gain = db_to_gain(p.output_db);
        for i in 0..out_l.len() {
            let lfo_l = (core::f32::consts::TAU * self.lfo_phase).sin();
            let lfo_r = (core::f32::consts::TAU * (self.lfo_phase + p.spread * 0.5)).sin();
            let a_l = self.coeff(lfo_l);
            let a_r = self.coeff(lfo_r);
            let wet_l = self.left.tick(in_l[i], a_l, stages, p.feedback);
            let wet_r = self.right.tick(in_r[i], a_r, stages, p.feedback);
            out_l[i] = (in_l[i] * dry + wet_l * p.mix) * out_gain;
            out_r[i] = (in_r[i] * dry + wet_r * p.mix) * out_gain;
            self.lfo_phase += inc;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
    }
}

pub fn apply_param(p: &mut PhaserParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_STAGES => p.pairs = v.clamp(1.0, 6.0).round() as u8,
        P_RATE => p.rate_hz = v.clamp(0.02, 5.0),
        P_CENTER => p.center_hz = v.clamp(100.0, 8_000.0),
        P_DEPTH => p.depth = v.clamp(0.0, 1.0),
        P_FEEDBACK => p.feedback = v.clamp(0.0, 0.9),
        P_SPREAD => p.spread = v.clamp(0.0, 1.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &PhaserParams, id: u32) -> f64 {
    (match id {
        P_STAGES => p.pairs as f32,
        P_RATE => p.rate_hz,
        P_CENTER => p.center_hz,
        P_DEPTH => p.depth,
        P_FEEDBACK => p.feedback,
        P_SPREAD => p.spread,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebPhaser {
    engine: PhaserEngine,
}

impl Plugin for ZAudioWebPhaser {
    fn new() -> Self {
        Self {
            engine: PhaserEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = PhaserEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebPhaser>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> PhaserEngine {
        PhaserEngine::new(48_000.0)
    }

    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut state = 0x8badf00du32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn render(e: &mut PhaserEngine, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(input, input, &mut l, &mut r);
        (l, r)
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 8);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((860..=867).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = PhaserParams::default();
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
    fn half_mix_phased_noise_differs_from_the_input() {
        let mut e = defaults();
        let input = noise(9_600, 0.5);
        let (l, _r) = render(&mut e, &input);
        let diff: f32 = input.iter().zip(l.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 10.0, "phased output too close to dry, diff {diff}");
    }

    #[test]
    fn stage_count_changes_the_response() {
        let input = noise(9_600, 0.5);
        let mut few = defaults();
        let mut p = *few.params();
        p.pairs = 1;
        few.set_params(p);
        let (l_few, _) = render(&mut few, &input);

        let mut many = defaults();
        p.pairs = 6;
        many.set_params(p);
        let (l_many, _) = render(&mut many, &input);

        let diff: f32 = l_few
            .iter()
            .zip(l_many.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(diff > 10.0, "stage count had no effect, diff {diff}");
    }

    #[test]
    fn feedback_changes_the_output() {
        let input = noise(9_600, 0.5);
        let mut open = defaults();
        let mut p = *open.params();
        p.feedback = 0.0;
        open.set_params(p);
        let (l_open, _) = render(&mut open, &input);

        let mut resonant = defaults();
        p.feedback = 0.9;
        resonant.set_params(p);
        let (l_res, _) = render(&mut resonant, &input);

        let diff: f32 = l_open
            .iter()
            .zip(l_res.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(diff > 10.0, "feedback had no effect, diff {diff}");
    }

    #[test]
    fn spread_decorrelates_the_channels() {
        let input = noise(9_600, 0.5);

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
        p.feedback = 0.9;
        p.pairs = 6;
        p.depth = 1.0;
        p.rate_hz = 5.0;
        p.mix = 1.0;
        e.set_params(p);
        let input = noise(96_000, 0.25); // 2 s at 48 kHz
        let (l, r) = render(&mut e, &input);
        let peak = l
            .iter()
            .chain(r.iter())
            .fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak.is_finite() && peak < 4.0, "peak {peak}");
    }

    #[test]
    fn full_wet_allpass_chain_preserves_noise_rms() {
        // A pure allpass cascade is unity-magnitude at every frequency, so
        // full-wet with no feedback must keep broadband energy intact.
        let mut e = defaults();
        let mut p = *e.params();
        p.mix = 1.0;
        p.feedback = 0.0;
        p.depth = 0.0;
        e.set_params(p);
        let input = noise(48_000, 0.5);
        let (l, _r) = render(&mut e, &input);
        let (in_rms, out_rms) = (rms(&input), rms(&l));
        assert!(
            (out_rms - in_rms).abs() < in_rms * 0.2,
            "in {in_rms}, out {out_rms}"
        );
    }
}
