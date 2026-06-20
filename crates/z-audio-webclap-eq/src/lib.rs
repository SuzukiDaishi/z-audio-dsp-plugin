//! Z Audio Simple EQ, packaged as a real WCLAP audio-effect plugin.
//!
//! A thin `wclap-plugin` (see `crates/wclap-plugin`) front end around
//! `z-audio-dsp::ThreeBandButterworthEq` — three independently switchable
//! Butterworth filters (low-pass / band-pass / high-pass), connected in
//! series, exposed as `clap.params`. All three bands start disabled
//! (pass-through); see `ThreeBandButterworthEq::new`'s doc comment for why.
//!
//! Sibling to `crates/z-audio-webclap` (the instrument); this one is a pure
//! audio_in -> audio_out effect with no note input.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{
    ButterworthKind, Effect, ParamId, ParamUnit, ProcessContext, ThreeBandButterworthEq,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.simple-eq\0",
    name: b"Z Audio Simple EQ\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"A simple 3-band EQ (low-pass / band-pass / high-pass) built on z-audio-dsp\0",
    features: &[b"audio-effect\0", b"equalizer\0", b"eq\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

/// The fifteen `Eq*` parameter ids, in the same order `ParamId::ALL` declares
/// them. Every other `ParamId` (synth-only: generator/envelope/LFO/etc.)
/// is irrelevant to a bare EQ effect and excluded.
fn is_eq_param(id: ParamId) -> bool {
    matches!(
        id,
        ParamId::EqLowEnabled
            | ParamId::EqLowFreq
            | ParamId::EqLowType
            | ParamId::EqMidEnabled
            | ParamId::EqMidFreq
            | ParamId::EqMidType
            | ParamId::EqHighEnabled
            | ParamId::EqHighFreq
            | ParamId::EqHighType
            | ParamId::EqLowGainDb
            | ParamId::EqLowQ
            | ParamId::EqMidGainDb
            | ParamId::EqMidQ
            | ParamId::EqHighGainDb
            | ParamId::EqHighQ
    )
}

fn build_params() -> Vec<ParamDef> {
    ParamId::ALL
        .iter()
        .copied()
        .filter(|id| is_eq_param(*id))
        .map(|id| {
            let m = id.metadata();
            let mut name_bytes = m.name.as_bytes().to_vec();
            name_bytes.push(0);
            let name: &'static [u8] = Box::leak(name_bytes.into_boxed_slice());
            let flags = match m.unit {
                ParamUnit::Enum | ParamUnit::Boolean => PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED,
                ParamUnit::Linear | ParamUnit::Hertz | ParamUnit::Seconds => PARAM_IS_AUTOMATABLE,
            };
            ParamDef {
                id: id as u32,
                flags,
                name,
                module: b"\0",
                min: m.min as f64,
                max: m.max as f64,
                default: m.default as f64,
            }
        })
        .collect()
}

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioSimpleEq {
    eq: ThreeBandButterworthEq,
    sample_rate: f32,
    max_block_size: usize,
}

impl ZAudioSimpleEq {
    fn find_param(id: u32) -> Option<ParamId> {
        ParamId::ALL
            .iter()
            .copied()
            .find(|p| *p as u32 == id && is_eq_param(*p))
    }
}

impl Plugin for ZAudioSimpleEq {
    fn new() -> Self {
        let mut eq = ThreeBandButterworthEq::new();
        eq.prepare(48_000.0, 128);
        Self {
            eq,
            sample_rate: 48_000.0,
            max_block_size: 128,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.max_block_size = (max_frames as usize).max(1);
        self.eq.prepare(self.sample_rate, self.max_block_size);
    }

    fn reset(&mut self) {
        self.eq.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(build_params)
    }

    fn get_param(&self, id: u32) -> f64 {
        match Self::find_param(id) {
            Some(ParamId::EqLowEnabled) => bool_to_f64(self.eq.low.enabled),
            Some(ParamId::EqLowFreq) => self.eq.low.frequency_hz as f64,
            Some(ParamId::EqLowType) => self.eq.low.kind.to_param_value() as f64,
            Some(ParamId::EqLowGainDb) => self.eq.low.gain_db as f64,
            Some(ParamId::EqLowQ) => self.eq.low.q as f64,
            Some(ParamId::EqMidEnabled) => bool_to_f64(self.eq.mid.enabled),
            Some(ParamId::EqMidFreq) => self.eq.mid.frequency_hz as f64,
            Some(ParamId::EqMidType) => self.eq.mid.kind.to_param_value() as f64,
            Some(ParamId::EqMidGainDb) => self.eq.mid.gain_db as f64,
            Some(ParamId::EqMidQ) => self.eq.mid.q as f64,
            Some(ParamId::EqHighEnabled) => bool_to_f64(self.eq.high.enabled),
            Some(ParamId::EqHighFreq) => self.eq.high.frequency_hz as f64,
            Some(ParamId::EqHighType) => self.eq.high.kind.to_param_value() as f64,
            Some(ParamId::EqHighGainDb) => self.eq.high.gain_db as f64,
            Some(ParamId::EqHighQ) => self.eq.high.q as f64,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(param_id) = Self::find_param(id) else {
            return;
        };
        let m = param_id.metadata();
        let clamped = (value as f32).clamp(m.min, m.max);
        let flag = value >= 0.5;
        match param_id {
            ParamId::EqLowEnabled => self.eq.low.enabled = flag,
            ParamId::EqLowFreq => self.eq.low.frequency_hz = clamped,
            ParamId::EqLowType => self.eq.low.kind = ButterworthKind::from_param_value(clamped),
            ParamId::EqLowGainDb => self.eq.low.gain_db = clamped,
            ParamId::EqLowQ => self.eq.low.q = clamped,
            ParamId::EqMidEnabled => self.eq.mid.enabled = flag,
            ParamId::EqMidFreq => self.eq.mid.frequency_hz = clamped,
            ParamId::EqMidType => self.eq.mid.kind = ButterworthKind::from_param_value(clamped),
            ParamId::EqMidGainDb => self.eq.mid.gain_db = clamped,
            ParamId::EqMidQ => self.eq.mid.q = clamped,
            ParamId::EqHighEnabled => self.eq.high.enabled = flag,
            ParamId::EqHighFreq => self.eq.high.frequency_hz = clamped,
            ParamId::EqHighType => self.eq.high.kind = ButterworthKind::from_param_value(clamped),
            ParamId::EqHighGainDb => self.eq.high.gain_db = clamped,
            ParamId::EqHighQ => self.eq.high.q = clamped,
            _ => {}
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        match ctx.stereo_io() {
            Some(io) => {
                let wclap_plugin::StereoIo {
                    input_l,
                    input_r,
                    output_l,
                    output_r,
                } = io;
                output_l.copy_from_slice(input_l);
                output_r.copy_from_slice(input_r);
                let events = [];
                let process_ctx = ProcessContext::new(self.sample_rate, frames, 120.0, &events);
                self.eq.process_stereo(&process_ctx, output_l, output_r);
            }
            None => silence(ctx),
        }

        ProcessStatus::Continue
    }
}

fn bool_to_f64(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioSimpleEq>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_all_eq_params_including_gain_and_q() {
        let ids: Vec<_> = ZAudioSimpleEq::params()
            .iter()
            .map(|param| param.id)
            .collect();

        assert_eq!(ids.len(), 15);
        for id in [
            ParamId::EqLowGainDb,
            ParamId::EqLowQ,
            ParamId::EqMidGainDb,
            ParamId::EqMidQ,
            ParamId::EqHighGainDb,
            ParamId::EqHighQ,
        ] {
            assert!(ids.contains(&(id as u32)));
        }
    }

    #[test]
    fn gain_and_q_round_trip_through_webclap_params() {
        let mut plugin = ZAudioSimpleEq::new();

        plugin.set_param(ParamId::EqMidGainDb as u32, 6.5);
        plugin.set_param(ParamId::EqMidQ as u32, 2.25);

        assert_eq!(plugin.get_param(ParamId::EqMidGainDb as u32), 6.5);
        assert_eq!(plugin.get_param(ParamId::EqMidQ as u32), 2.25);
    }
}
