//! Z Audio Tremolo — stereo amplitude modulation with a sine / triangle /
//! square LFO, adjustable stereo LFO phase offset (180° = auto-pan) and
//! output trim. The square wave is run through a ~2 ms one-pole smoother
//! so full-depth on/off gating stays click-free.
//!
//! Web ids 880-884 — a fresh block (ring mod uses 620s, distortion 640s).
//! A future native build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

pub const P_RATE: u32 = 880;
pub const P_DEPTH: u32 = 881;
pub const P_WAVE: u32 = 882;
pub const P_PHASE: u32 = 883;
pub const P_OUTPUT: u32 = 884;

pub const WAVE_SINE: u8 = 0;
pub const WAVE_TRIANGLE: u8 = 1;
pub const WAVE_SQUARE: u8 = 2;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.tremolo\0",
    name: b"Z Audio Tremolo\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Tremolo / auto-pan with a sine/tri/square LFO\0",
    features: &[b"audio-effect\0", b"tremolo\0", b"stereo\0"],
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
        def(P_RATE, b"Rate\0", 0.1, 20.0, 4.0, false),
        def(P_DEPTH, b"Depth\0", 0.0, 1.0, 0.6, false),
        def(P_WAVE, b"Wave\0", 0.0, 2.0, 0.0, true),
        def(P_PHASE, b"Stereo Phase\0", 0.0, 180.0, 0.0, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct TremoloParams {
    pub rate_hz: f32,
    pub depth: f32,
    pub wave: u8,
    pub phase_deg: f32,
    pub output_db: f32,
}

impl Default for TremoloParams {
    fn default() -> Self {
        Self {
            rate_hz: 4.0,
            depth: 0.6,
            wave: WAVE_SINE,
            phase_deg: 0.0,
            output_db: 0.0,
        }
    }
}

/// LFO value in [-1, 1] for `phase` in cycles (wraps). The square wave is
/// sign(sin) — the raw edge; the engine smooths it with a ~2 ms one-pole.
#[inline]
pub fn lfo(wave: u8, phase: f32) -> f32 {
    let t = phase - phase.floor();
    match wave {
        WAVE_TRIANGLE => {
            if t < 0.5 {
                4.0 * t - 1.0
            } else {
                3.0 - 4.0 * t
            }
        }
        WAVE_SQUARE => {
            if t < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => (core::f32::consts::TAU * t).sin(),
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

pub struct TremoloEngine {
    params: TremoloParams,
    sample_rate: f32,
    phase: f32,
    /// One-pole states that de-click the square LFO edges (per channel,
    /// because the stereo phase offset puts the edges at different times).
    smooth_l: f32,
    smooth_r: f32,
}

impl TremoloEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            params: TremoloParams::default(),
            sample_rate: sample_rate.max(1.0),
            phase: 0.0,
            smooth_l: 0.0,
            smooth_r: 0.0,
        }
    }

    pub fn params(&self) -> &TremoloParams {
        &self.params
    }

    pub fn set_params(&mut self, p: TremoloParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.smooth_l = 0.0;
        self.smooth_r = 0.0;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let inc = p.rate_hz / self.sample_rate;
        let offset = p.phase_deg / 360.0;
        let out_gain = db_to_gain(p.output_db);
        // ~2 ms one-pole, applied to the square LFO only: it keeps
        // depth-1 on/off gating click-free without dulling sine/tri.
        let smooth_a = 1.0 - (-1.0 / (0.002 * self.sample_rate)).exp();
        for i in 0..out_l.len() {
            let raw_l = lfo(p.wave, self.phase);
            let raw_r = lfo(p.wave, self.phase + offset);
            let (vl, vr) = if p.wave == WAVE_SQUARE {
                self.smooth_l += smooth_a * (raw_l - self.smooth_l);
                self.smooth_r += smooth_a * (raw_r - self.smooth_r);
                (self.smooth_l, self.smooth_r)
            } else {
                // Track so switching to square starts from the live value.
                self.smooth_l = raw_l;
                self.smooth_r = raw_r;
                (raw_l, raw_r)
            };
            // Gain in [1 - depth, 1]: depth 1 sweeps full-on to silence.
            let gain_l = 1.0 - p.depth * (0.5 + 0.5 * vl);
            let gain_r = 1.0 - p.depth * (0.5 + 0.5 * vr);
            out_l[i] = in_l[i] * gain_l * out_gain;
            out_r[i] = in_r[i] * gain_r * out_gain;
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

pub fn apply_param(p: &mut TremoloParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_RATE => p.rate_hz = v.clamp(0.1, 20.0),
        P_DEPTH => p.depth = v.clamp(0.0, 1.0),
        P_WAVE => p.wave = v.clamp(0.0, 2.0).round() as u8,
        P_PHASE => p.phase_deg = v.clamp(0.0, 180.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &TremoloParams, id: u32) -> f64 {
    (match id {
        P_RATE => p.rate_hz,
        P_DEPTH => p.depth,
        P_WAVE => p.wave as f32,
        P_PHASE => p.phase_deg,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebTremolo {
    engine: TremoloEngine,
}

impl Plugin for ZAudioWebTremolo {
    fn new() -> Self {
        Self {
            engine: TremoloEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = TremoloEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebTremolo>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> TremoloEngine {
        TremoloEngine::new(48_000.0)
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 5);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((880..=884).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = TremoloParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!((param_value(&p, def.id) - def.max).abs() < 1e-6);
        }
    }

    #[test]
    fn zero_depth_is_a_clean_passthrough() {
        let mut e = defaults();
        let mut p = *e.params();
        p.depth = 0.0;
        e.set_params(p);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn full_depth_sine_sweeps_between_silence_and_unity() {
        // DC input exposes the gain curve directly: over one 4 Hz cycle at
        // depth 1 it must reach both ~0 (trough) and ~1 (crest).
        let mut e = defaults();
        let mut p = *e.params();
        p.depth = 1.0;
        e.set_params(p);
        let input = vec![1.0f32; 12_000]; // one cycle at 4 Hz / 48 kHz
        let (mut l, mut r) = (vec![0.0; 12_000], vec![0.0; 12_000]);
        e.process(&input, &input, &mut l, &mut r);
        let min = l.iter().fold(f32::MAX, |m, s| m.min(*s));
        let max = l.iter().fold(f32::MIN, |m, s| m.max(*s));
        assert!(min < 0.02, "trough {min}");
        assert!(max > 0.98, "crest {max}");
    }

    #[test]
    fn stereo_phase_180_anti_correlates_the_channels() {
        // With a sine LFO at 180°, gain_l + gain_r == 2 - depth exactly:
        // when L dips, R rises by the same amount (auto-pan).
        let mut e = defaults();
        let mut p = *e.params();
        p.depth = 1.0;
        p.phase_deg = 180.0;
        e.set_params(p);
        let input = vec![1.0f32; 12_000];
        let (mut l, mut r) = (vec![0.0; 12_000], vec![0.0; 12_000]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in l.iter().zip(r.iter()) {
            assert!((a + b - 1.0).abs() < 1e-3, "l {a} r {b}");
        }
        let diff: f32 = l.iter().zip(r.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0, "channels should differ, diff {diff}");
    }

    #[test]
    fn square_wave_is_smoothed_against_clicks() {
        // Full-depth square gating of DC 1.0: the ~2 ms one-pole must keep
        // sample-to-sample jumps small while still reaching on and off.
        let mut e = defaults();
        let mut p = *e.params();
        p.depth = 1.0;
        p.wave = WAVE_SQUARE;
        e.set_params(p);
        let input = vec![1.0f32; 24_000]; // two cycles at 4 Hz
        let (mut l, mut r) = (vec![0.0; 24_000], vec![0.0; 24_000]);
        e.process(&input, &input, &mut l, &mut r);
        let max_jump = l
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(max_jump < 0.35, "max jump {max_jump}");
        let min = l.iter().fold(f32::MAX, |m, s| m.min(*s));
        let max = l.iter().fold(f32::MIN, |m, s| m.max(*s));
        assert!(min < 0.05 && max > 0.95, "min {min} max {max}");
    }

    #[test]
    fn output_gain_scales_the_result() {
        let mut e = defaults();
        let mut p = *e.params();
        p.depth = 0.0;
        p.output_db = -6.0;
        e.set_params(p);
        let input = vec![1.0f32; 64];
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        e.process(&input, &input, &mut l, &mut r);
        assert!((l[10] - 0.501).abs() < 0.01);
    }

    #[test]
    fn output_stays_finite_and_bounded_at_extremes() {
        for wave in [WAVE_SINE, WAVE_TRIANGLE, WAVE_SQUARE] {
            let mut e = defaults();
            let mut p = *e.params();
            p.rate_hz = 20.0;
            p.depth = 1.0;
            p.wave = wave;
            p.phase_deg = 180.0;
            p.output_db = 24.0;
            e.set_params(p);
            let input: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.9).sin()).collect();
            let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
            e.process(&input, &input, &mut l, &mut r);
            for s in l.iter().chain(r.iter()) {
                assert!(s.is_finite());
                assert!(s.abs() <= 16.0, "wave {wave} sample {s}");
            }
        }
    }
}
