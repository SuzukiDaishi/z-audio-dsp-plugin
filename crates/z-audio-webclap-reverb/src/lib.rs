use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{
    Effect, ParamId, ParamUnit, ParametricReverb, ParametricReverbParams, ProcessContext,
};

const PARAM_IDS: [ParamId; 13] = [
    ParamId::ReverbMix,
    ParamId::ReverbRoomSize,
    ParamId::ReverbDecay,
    ParamId::ReverbPreDelay,
    ParamId::ReverbDiffusion,
    ParamId::ReverbDamping,
    ParamId::ReverbLowCut,
    ParamId::ReverbHighCut,
    ParamId::ReverbModRate,
    ParamId::ReverbModDepth,
    ParamId::ReverbWidth,
    ParamId::ReverbEarlyLateMix,
    ParamId::ReverbOutputGain,
];

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.parametric-reverb\0",
    name: b"Z Audio Parametric Reverb\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"FDN parametric reverb built on z-audio-dsp\0",
    features: &[b"audio-effect\0", b"reverb\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioWebReverb {
    reverb: ParametricReverb,
    params: ParametricReverbParams,
    sample_rate: f32,
}

impl Plugin for ZAudioWebReverb {
    fn new() -> Self {
        let mut reverb = ParametricReverb::default();
        reverb.prepare(48_000.0, 128);
        Self {
            reverb,
            params: ParametricReverbParams::default(),
            sample_rate: 48_000.0,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.reverb
            .prepare(self.sample_rate, (max_frames as usize).max(1));
        self.reverb.set_params(self.params);
    }

    fn reset(&mut self) {
        self.reverb.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(|| PARAM_IDS.iter().copied().map(param_def).collect())
    }

    fn get_param(&self, id: u32) -> f64 {
        match id_to_param(id) {
            Some(ParamId::ReverbMix) => self.params.mix as f64,
            Some(ParamId::ReverbRoomSize) => self.params.room_size as f64,
            Some(ParamId::ReverbDecay) => self.params.decay_time_sec as f64,
            Some(ParamId::ReverbPreDelay) => self.params.pre_delay_ms as f64,
            Some(ParamId::ReverbDiffusion) => self.params.diffusion as f64,
            Some(ParamId::ReverbDamping) => self.params.damping as f64,
            Some(ParamId::ReverbLowCut) => self.params.low_cut_hz as f64,
            Some(ParamId::ReverbHighCut) => self.params.high_cut_hz as f64,
            Some(ParamId::ReverbModRate) => self.params.modulation_rate_hz as f64,
            Some(ParamId::ReverbModDepth) => self.params.modulation_depth as f64,
            Some(ParamId::ReverbWidth) => self.params.width as f64,
            Some(ParamId::ReverbEarlyLateMix) => self.params.early_late_mix as f64,
            Some(ParamId::ReverbOutputGain) => self.params.output_gain_db as f64,
            _ => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Some(param_id) = id_to_param(id) else {
            return;
        };
        let value = (value as f32).clamp(param_id.metadata().min, param_id.metadata().max);
        match param_id {
            ParamId::ReverbMix => self.params.mix = value,
            ParamId::ReverbRoomSize => self.params.room_size = value,
            ParamId::ReverbDecay => self.params.decay_time_sec = value,
            ParamId::ReverbPreDelay => self.params.pre_delay_ms = value,
            ParamId::ReverbDiffusion => self.params.diffusion = value,
            ParamId::ReverbDamping => self.params.damping = value,
            ParamId::ReverbLowCut => self.params.low_cut_hz = value,
            ParamId::ReverbHighCut => self.params.high_cut_hz = value,
            ParamId::ReverbModRate => self.params.modulation_rate_hz = value,
            ParamId::ReverbModDepth => self.params.modulation_depth = value,
            ParamId::ReverbWidth => self.params.width = value,
            ParamId::ReverbEarlyLateMix => self.params.early_late_mix = value,
            ParamId::ReverbOutputGain => self.params.output_gain_db = value,
            _ => {}
        }
        self.reverb.set_params(self.params);
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        match ctx.stereo_io() {
            Some(io) => {
                io.output_l.copy_from_slice(io.input_l);
                io.output_r.copy_from_slice(io.input_r);
                let events = [];
                let process_ctx = ProcessContext::new(self.sample_rate, frames, 120.0, &events);
                self.reverb
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
    init_plugin::<ZAudioWebReverb>(&PLUGIN_DEF);
}
