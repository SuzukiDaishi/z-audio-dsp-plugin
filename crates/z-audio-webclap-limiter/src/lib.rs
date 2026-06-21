use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{Effect, Limiter, LimiterParams, ParamId, ParamUnit, ProcessContext};

const PARAM_IDS: [ParamId; 8] = [
    ParamId::LimiterInputGain,
    ParamId::LimiterThreshold,
    ParamId::LimiterCeiling,
    ParamId::LimiterRelease,
    ParamId::LimiterLookahead,
    ParamId::LimiterStereoLink,
    ParamId::LimiterTruePeak,
    ParamId::LimiterOutputGain,
];

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.limiter\0",
    name: b"Z Audio Limiter\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Lookahead peak limiter built on z-audio-dsp\0",
    features: &[b"audio-effect\0", b"limiter\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioWebLimiter {
    limiter: Limiter,
    params: LimiterParams,
    sample_rate: f32,
}

impl Plugin for ZAudioWebLimiter {
    fn new() -> Self {
        let mut limiter = Limiter::default();
        limiter.prepare(48_000.0, 128);
        Self {
            limiter,
            params: LimiterParams::default(),
            sample_rate: 48_000.0,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.limiter
            .prepare(self.sample_rate, (max_frames as usize).max(1));
        self.limiter.set_params(self.params);
    }

    fn reset(&mut self) {
        self.limiter.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(|| PARAM_IDS.iter().copied().map(param_def).collect())
    }

    fn get_param(&self, id: u32) -> f64 {
        match id_to_param(id) {
            Some(ParamId::LimiterInputGain) => self.params.input_gain_db as f64,
            Some(ParamId::LimiterThreshold) => self.params.threshold_db as f64,
            Some(ParamId::LimiterCeiling) => self.params.ceiling_db as f64,
            Some(ParamId::LimiterRelease) => self.params.release_ms as f64,
            Some(ParamId::LimiterLookahead) => self.params.lookahead_ms as f64,
            Some(ParamId::LimiterStereoLink) => self.params.stereo_link as f64,
            Some(ParamId::LimiterTruePeak) => bool_to_f64(self.params.true_peak),
            Some(ParamId::LimiterOutputGain) => self.params.output_gain_db as f64,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(param_id) = id_to_param(id) else {
            return;
        };
        let value = (value as f32).clamp(param_id.metadata().min, param_id.metadata().max);
        match param_id {
            ParamId::LimiterInputGain => self.params.input_gain_db = value,
            ParamId::LimiterThreshold => self.params.threshold_db = value,
            ParamId::LimiterCeiling => self.params.ceiling_db = value,
            ParamId::LimiterRelease => self.params.release_ms = value,
            ParamId::LimiterLookahead => self.params.lookahead_ms = value,
            ParamId::LimiterStereoLink => self.params.stereo_link = value,
            ParamId::LimiterTruePeak => self.params.true_peak = value >= 0.5,
            ParamId::LimiterOutputGain => self.params.output_gain_db = value,
            _ => {}
        }
        self.limiter.set_params(self.params);
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        match ctx.stereo_io() {
            Some(io) => {
                io.output_l.copy_from_slice(io.input_l);
                io.output_r.copy_from_slice(io.input_r);
                let events = [];
                let process_ctx = ProcessContext::new(self.sample_rate, frames, 120.0, &events);
                self.limiter
                    .process_stereo(&process_ctx, io.output_l, io.output_r);
            }
            None => silence(ctx),
        }
        ProcessStatus::Continue
    }
}

fn id_to_param(id: u32) -> Option<ParamId> {
    PARAM_IDS.iter().copied().find(|param| *param as u32 == id)
}

fn bool_to_f64(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

fn param_def(id: ParamId) -> ParamDef {
    let m = id.metadata();
    let mut name_bytes = m.name.as_bytes().to_vec();
    name_bytes.push(0);
    let name = Box::leak(name_bytes.into_boxed_slice());
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
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioWebLimiter>(&PLUGIN_DEF);
}
