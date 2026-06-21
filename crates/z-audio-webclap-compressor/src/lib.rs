use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{
    Compressor, CompressorParams, DetectorMode, Effect, ParamId, ParamUnit, ProcessContext,
};

const PARAM_IDS: [ParamId; 10] = [
    ParamId::CompressorInputGain,
    ParamId::CompressorThreshold,
    ParamId::CompressorRatio,
    ParamId::CompressorKnee,
    ParamId::CompressorAttack,
    ParamId::CompressorRelease,
    ParamId::CompressorMakeupGain,
    ParamId::CompressorMix,
    ParamId::CompressorDetectorMode,
    ParamId::CompressorStereoLink,
];

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.compressor\0",
    name: b"Z Audio Compressor\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Feed-forward compressor built on z-audio-dsp\0",
    features: &[b"audio-effect\0", b"compressor\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioWebCompressor {
    compressor: Compressor,
    params: CompressorParams,
    sample_rate: f32,
    max_block_size: usize,
}

impl Plugin for ZAudioWebCompressor {
    fn new() -> Self {
        let mut compressor = Compressor::default();
        compressor.prepare(48_000.0, 128);
        Self {
            compressor,
            params: CompressorParams::default(),
            sample_rate: 48_000.0,
            max_block_size: 128,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.max_block_size = (max_frames as usize).max(1);
        self.compressor
            .prepare(self.sample_rate, self.max_block_size);
        self.compressor.set_params(self.params);
    }

    fn reset(&mut self) {
        self.compressor.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(|| PARAM_IDS.iter().copied().map(param_def).collect())
    }

    fn get_param(&self, id: u32) -> f64 {
        match id_to_param(id) {
            Some(ParamId::CompressorInputGain) => self.params.input_gain_db as f64,
            Some(ParamId::CompressorThreshold) => self.params.threshold_db as f64,
            Some(ParamId::CompressorRatio) => self.params.ratio as f64,
            Some(ParamId::CompressorKnee) => self.params.knee_db as f64,
            Some(ParamId::CompressorAttack) => self.params.attack_ms as f64,
            Some(ParamId::CompressorRelease) => self.params.release_ms as f64,
            Some(ParamId::CompressorMakeupGain) => self.params.makeup_gain_db as f64,
            Some(ParamId::CompressorMix) => self.params.mix as f64,
            Some(ParamId::CompressorDetectorMode) => {
                self.params.detector_mode.to_param_value() as f64
            }
            Some(ParamId::CompressorStereoLink) => self.params.stereo_link as f64,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(param_id) = id_to_param(id) else {
            return;
        };
        let value = (value as f32).clamp(param_id.metadata().min, param_id.metadata().max);
        match param_id {
            ParamId::CompressorInputGain => self.params.input_gain_db = value,
            ParamId::CompressorThreshold => self.params.threshold_db = value,
            ParamId::CompressorRatio => self.params.ratio = value,
            ParamId::CompressorKnee => self.params.knee_db = value,
            ParamId::CompressorAttack => self.params.attack_ms = value,
            ParamId::CompressorRelease => self.params.release_ms = value,
            ParamId::CompressorMakeupGain => self.params.makeup_gain_db = value,
            ParamId::CompressorMix => self.params.mix = value,
            ParamId::CompressorDetectorMode => {
                self.params.detector_mode = DetectorMode::from_param_value(value)
            }
            ParamId::CompressorStereoLink => self.params.stereo_link = value,
            _ => {}
        }
        self.compressor.set_params(self.params);
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        process_effect(ctx, self.sample_rate, &mut self.compressor)
    }
}

fn process_effect(
    ctx: &mut ProcessCtx,
    sample_rate: f32,
    effect: &mut impl Effect,
) -> ProcessStatus {
    let frames = ctx.frames();
    match ctx.stereo_io() {
        Some(io) => {
            io.output_l.copy_from_slice(io.input_l);
            io.output_r.copy_from_slice(io.input_r);
            let events = [];
            let process_ctx = ProcessContext::new(sample_rate, frames, 120.0, &events);
            effect.process_stereo(&process_ctx, io.output_l, io.output_r);
        }
        None => silence(ctx),
    }
    ProcessStatus::Continue
}

fn id_to_param(id: u32) -> Option<ParamId> {
    PARAM_IDS.iter().copied().find(|param| *param as u32 == id)
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
    init_plugin::<ZAudioWebCompressor>(&PLUGIN_DEF);
}
