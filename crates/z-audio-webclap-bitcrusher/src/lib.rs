//! Z Audio Bitcrusher — bit-depth reduction (1-16 bits, fractional) plus
//! sample-rate reduction (1-64x sample-and-hold), dry/wet mix and output
//! trim.
//!
//! Web ids 680-683 — a fresh block (ring mod 620s, distortion 640s,
//! saturator 660s). A future native build must mirror these ids
//! one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE,
};

pub const P_BITS: u32 = 680;
pub const P_DOWNSAMPLE: u32 = 681;
pub const P_MIX: u32 = 682;
pub const P_OUTPUT: u32 = 683;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.bitcrusher\0",
    name: b"Z Audio Bitcrusher\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Bit-depth and sample-rate reduction\0",
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
        def(P_BITS, b"Bits\0", 1.0, 16.0, 8.0),
        def(P_DOWNSAMPLE, b"Downsample\0", 1.0, 64.0, 4.0),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 1.0),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0),
    ]
}

#[derive(Clone, Copy)]
pub struct BitcrusherParams {
    pub bits: f32,
    pub downsample: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for BitcrusherParams {
    fn default() -> Self {
        Self {
            bits: 8.0,
            downsample: 4.0,
            mix: 1.0,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Mid-tread quantizer, exposed so the UI mirrors it exactly. `bits` may
/// be fractional; the step size interpolates smoothly between depths.
#[inline]
pub fn quantize(bits: f32, x: f32) -> f32 {
    let step = 2.0_f32.powf(1.0 - bits.clamp(1.0, 16.0));
    ((x / step).round() * step).clamp(-1.0, 1.0)
}

#[derive(Default, Clone, Copy)]
struct ChannelState {
    counter: f32,
    held: f32,
}

pub struct BitcrusherEngine {
    params: BitcrusherParams,
    left: ChannelState,
    right: ChannelState,
}

impl BitcrusherEngine {
    pub fn new(_sample_rate: f32) -> Self {
        Self {
            params: BitcrusherParams::default(),
            left: ChannelState::default(),
            right: ChannelState::default(),
        }
    }

    pub fn params(&self) -> &BitcrusherParams {
        &self.params
    }

    pub fn set_params(&mut self, p: BitcrusherParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.left = ChannelState::default();
        self.right = ChannelState::default();
    }

    #[inline]
    fn crush(state: &mut ChannelState, x: f32, bits: f32, factor: f32) -> f32 {
        state.counter += 1.0;
        if state.counter >= factor {
            state.counter -= factor;
            state.held = quantize(bits, x);
        }
        state.held
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let factor = p.downsample.max(1.0);
        let dry = 1.0 - p.mix;
        let out_gain = db_to_gain(p.output_db);
        for i in 0..out_l.len() {
            let wet_l = Self::crush(&mut self.left, in_l[i], p.bits, factor);
            let wet_r = Self::crush(&mut self.right, in_r[i], p.bits, factor);
            out_l[i] = (in_l[i] * dry + wet_l * p.mix) * out_gain;
            out_r[i] = (in_r[i] * dry + wet_r * p.mix) * out_gain;
        }
    }
}

pub fn apply_param(p: &mut BitcrusherParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_BITS => p.bits = v.clamp(1.0, 16.0),
        P_DOWNSAMPLE => p.downsample = v.clamp(1.0, 64.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &BitcrusherParams, id: u32) -> f64 {
    (match id {
        P_BITS => p.bits,
        P_DOWNSAMPLE => p.downsample,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebBitcrusher {
    engine: BitcrusherEngine,
}

impl Plugin for ZAudioWebBitcrusher {
    fn new() -> Self {
        Self {
            engine: BitcrusherEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = BitcrusherEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebBitcrusher>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 4);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((680..=683).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn one_bit_quantizes_to_three_levels() {
        for x in [-0.9, -0.3, 0.2, 0.8] {
            let y = quantize(1.0, x);
            assert!(
                y == -1.0 || y == 0.0 || y == 1.0,
                "1-bit quantize({x}) = {y}"
            );
        }
    }

    #[test]
    fn sixteen_bits_is_nearly_transparent() {
        for i in 0..100 {
            let x = (i as f32 / 50.0) - 1.0;
            assert!((quantize(16.0, x) - x).abs() < 1.0e-4);
        }
    }

    #[test]
    fn downsample_holds_values() {
        let mut e = BitcrusherEngine::new(48_000.0);
        let mut p = *e.params();
        p.bits = 16.0;
        p.downsample = 8.0;
        e.set_params(p);
        let input: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        e.process(&input, &input, &mut l, &mut r);
        // A ramp through an 8x sample-and-hold has at most 64/8 + 1 distinct values.
        let mut distinct: Vec<f32> = l.clone();
        distinct.dedup();
        assert!(distinct.len() <= 9, "{} distinct values", distinct.len());
    }

    #[test]
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = BitcrusherEngine::new(48_000.0);
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
    fn crushed_sine_stays_bounded_and_steppy() {
        let mut e = BitcrusherEngine::new(48_000.0);
        let mut p = *e.params();
        p.bits = 3.0;
        p.downsample = 1.0;
        e.set_params(p);
        let input: Vec<f32> = (0..960).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();
        let (mut l, mut r) = (vec![0.0; 960], vec![0.0; 960]);
        e.process(&input, &input, &mut l, &mut r);
        assert!(l.iter().all(|s| s.abs() <= 1.0));
        // 3 bits = step 0.25: every output lands on a lattice point.
        for s in &l {
            let lattice = (s / 0.25).round() * 0.25;
            assert!((s - lattice).abs() < 1e-6, "{s} off-lattice");
        }
    }
}
