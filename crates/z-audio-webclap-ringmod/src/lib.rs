//! Z Audio Ring Modulator — stereo ring modulation with a built-in carrier
//! oscillator (sine / triangle / saw / square), optional stereo carrier
//! phase offset, dry/wet mix and output trim.
//!
//! Web ids 620-624 — a fresh block (simple synth 100s, sampler 300s,
//! granular 400s, wavetable 500s). A future native build must mirror
//! these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_GAIN,
};

pub const P_FREQ: u32 = 620;
pub const P_WAVE: u32 = 621;
pub const P_STEREO: u32 = 622;
pub const P_MIX: u32 = 623;
pub const P_OUTPUT: u32 = 624;

pub const WAVE_SINE: u8 = 0;
pub const WAVE_TRIANGLE: u8 = 1;
pub const WAVE_SAW: u8 = 2;
pub const WAVE_SQUARE: u8 = 3;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.ringmod\0",
    name: b"Z Audio Ring Mod\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Ring modulator with a sine/tri/saw/square carrier\0",
    features: &[b"audio-effect\0", b"modulation\0", b"stereo\0"],
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
        def(P_FREQ, b"Frequency\0", 0.5, 8_000.0, 440.0, false),
        def(P_WAVE, b"Carrier Wave\0", 0.0, 3.0, 0.0, true),
        def(P_STEREO, b"Stereo Offset\0", 0.0, 180.0, 0.0, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 1.0, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct RingModParams {
    pub freq_hz: f32,
    pub wave: u8,
    pub stereo_deg: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for RingModParams {
    fn default() -> Self {
        Self {
            freq_hz: 440.0,
            wave: WAVE_SINE,
            stereo_deg: 0.0,
            mix: 1.0,
            output_db: 0.0,
        }
    }
}

/// Carrier value for `phase` in [0, 1).
#[inline]
fn carrier(wave: u8, phase: f32) -> f32 {
    let t = phase - phase.floor();
    match wave {
        WAVE_TRIANGLE => {
            if t < 0.5 {
                4.0 * t - 1.0
            } else {
                3.0 - 4.0 * t
            }
        }
        WAVE_SAW => 2.0 * t - 1.0,
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

pub struct RingModEngine {
    params: RingModParams,
    sample_rate: f32,
    phase: f32,
    /// Anti-zipper smoothing: stereo offset/mix/output are gain-like.
    /// Carrier frequency stays raw — the phase accumulator keeps it
    /// click-free already.
    sm_offset: Smoothed,
    sm_mix: Smoothed,
    sm_out: Smoothed,
    snapped: bool,
}

impl RingModEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(sr, tau);
            s
        };
        Self {
            params: RingModParams::default(),
            sample_rate: sr,
            phase: 0.0,
            sm_offset: smoother(TAU_GAIN),
            sm_mix: smoother(TAU_GAIN),
            sm_out: smoother(TAU_GAIN),
            snapped: false,
        }
    }

    pub fn params(&self) -> &RingModParams {
        &self.params
    }

    pub fn set_params(&mut self, p: RingModParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.snapped = false;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let inc = p.freq_hz / self.sample_rate;
        self.sm_offset.set_target(p.stereo_deg / 360.0);
        self.sm_mix.set_target(p.mix);
        self.sm_out.set_target(db_to_gain(p.output_db));
        if !self.snapped {
            self.sm_offset.snap();
            self.sm_mix.snap();
            self.sm_out.snap();
            self.snapped = true;
        }
        for i in 0..out_l.len() {
            let offset = self.sm_offset.tick();
            let mix = self.sm_mix.tick();
            let dry = 1.0 - mix;
            let out_gain = self.sm_out.tick();
            let cl = carrier(p.wave, self.phase);
            let cr = carrier(p.wave, self.phase + offset);
            out_l[i] = (in_l[i] * dry + in_l[i] * cl * mix) * out_gain;
            out_r[i] = (in_r[i] * dry + in_r[i] * cr * mix) * out_gain;
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

pub fn apply_param(p: &mut RingModParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_FREQ => p.freq_hz = v.clamp(0.5, 8_000.0),
        P_WAVE => p.wave = v.clamp(0.0, 3.0).round() as u8,
        P_STEREO => p.stereo_deg = v.clamp(0.0, 180.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &RingModParams, id: u32) -> f64 {
    (match id {
        P_FREQ => p.freq_hz,
        P_WAVE => p.wave as f32,
        P_STEREO => p.stereo_deg,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebRingMod {
    engine: RingModEngine,
}

impl Plugin for ZAudioWebRingMod {
    fn new() -> Self {
        Self {
            engine: RingModEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = RingModEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebRingMod>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> RingModEngine {
        RingModEngine::new(48_000.0)
    }

    #[test]
    fn output_gain_jump_is_smoothed() {
        // Jump output +24 dB and mix mid-render: the output must glide,
        // not step.
        let mut e = defaults();
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
        p.mix = 0.6;
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
            assert!((620..=624).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = RingModParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!((param_value(&p, def.id) - def.max).abs() < 1e-6);
        }
    }

    #[test]
    fn full_wet_sine_carrier_produces_sidebands_not_dry() {
        // DC input through a sine carrier must come out as the carrier
        // itself: pure ring modulation, no dry bleed at mix = 1.
        let mut e = defaults();
        let mut p = *e.params();
        p.freq_hz = 1_000.0;
        e.set_params(p);
        let input = vec![0.5f32; 480];
        let (mut l, mut r) = (vec![0.0; 480], vec![0.0; 480]);
        e.process(&input, &input, &mut l, &mut r);
        let peak = l.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
        // The carrier is bipolar, so the output must change sign.
        assert!(l.iter().any(|s| *s > 0.1) && l.iter().any(|s| *s < -0.1));
    }

    #[test]
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = defaults();
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
    fn stereo_offset_decorrelates_channels() {
        let mut e = defaults();
        let mut p = *e.params();
        p.stereo_deg = 90.0;
        p.freq_hz = 100.0;
        e.set_params(p);
        let input = vec![0.5f32; 960];
        let (mut l, mut r) = (vec![0.0; 960], vec![0.0; 960]);
        e.process(&input, &input, &mut l, &mut r);
        let diff: f32 = l.iter().zip(r.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 1.0, "channels should differ, diff {diff}");
    }

    #[test]
    fn output_gain_scales_the_result() {
        let mut e = defaults();
        let mut p = *e.params();
        p.mix = 0.0;
        p.output_db = -6.0;
        e.set_params(p);
        let input = vec![1.0f32; 64];
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        e.process(&input, &input, &mut l, &mut r);
        assert!((l[10] - 0.501).abs() < 0.01);
    }
}
