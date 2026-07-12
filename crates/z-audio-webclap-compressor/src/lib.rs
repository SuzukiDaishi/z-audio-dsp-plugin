use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_GAIN,
};
use z_audio_dsp::{
    Compressor, CompressorParams, DetectorMode, Effect, ParamId, ParamUnit, ProcessContext,
};

/// The compressor consumes params per `set_params` call, so smoothing
/// happens at this sub-block granularity: gliding params are re-pushed
/// once per chunk (0.67 ms at 48 kHz) instead of stepping per host block.
const SMOOTH_CHUNK: usize = 32;

/// Anti-zipper smoothing of the trim gains and mix. Threshold/ratio/knee
/// changes pass through the gain-computer ballistics and stay instant.
struct CompressorSmoothers {
    in_db: Smoothed,
    makeup_db: Smoothed,
    mix: Smoothed,
}

impl CompressorSmoothers {
    fn new(sample_rate: f32) -> Self {
        let rate = sample_rate.max(1.0) / SMOOTH_CHUNK as f32;
        let mk = || {
            let mut s = Smoothed::new(0.0);
            s.configure(rate, TAU_GAIN);
            s
        };
        Self {
            in_db: mk(),
            makeup_db: mk(),
            mix: mk(),
        }
    }

    fn set_targets(&mut self, p: &CompressorParams) {
        self.in_db.set_target(p.input_gain_db);
        self.makeup_db.set_target(p.makeup_gain_db);
        self.mix.set_target(p.mix);
    }

    fn snap_all(&mut self) {
        self.in_db.snap();
        self.makeup_db.snap();
        self.mix.snap();
    }

    fn all_settled(&self) -> bool {
        self.in_db.is_settled(1.0e-3)
            && self.makeup_db.is_settled(1.0e-3)
            && self.mix.is_settled(1.0e-4)
    }

    fn tick_and_apply(&mut self, base: CompressorParams) -> CompressorParams {
        let mut p = base;
        p.input_gain_db = self.in_db.tick();
        p.makeup_gain_db = self.makeup_db.tick();
        p.mix = self.mix.tick();
        p
    }
}

/// Chunked processing core (a free function so tests can drive it on
/// plain slices): pushes smoothed params into the effect once per chunk
/// while gliding or dirty, then renders the chunk in place.
#[allow(clippy::too_many_arguments)]
fn process_smoothed(
    compressor: &mut Compressor,
    sm: &mut CompressorSmoothers,
    params: CompressorParams,
    dirty: &mut bool,
    snapped: &mut bool,
    sample_rate: f32,
    out_l: &mut [f32],
    out_r: &mut [f32],
) {
    sm.set_targets(&params);
    if !*snapped {
        sm.snap_all();
        *snapped = true;
        *dirty = true;
    }
    let events = [];
    let n = out_l.len();
    let mut at = 0;
    while at < n {
        let m = SMOOTH_CHUNK.min(n - at);
        if *dirty || !sm.all_settled() {
            compressor.set_params(sm.tick_and_apply(params));
            *dirty = false;
        }
        let process_ctx = ProcessContext::new(sample_rate, m, 120.0, &events);
        compressor.process_stereo(&process_ctx, &mut out_l[at..at + m], &mut out_r[at..at + m]);
        at += m;
    }
}

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
    smoothers: CompressorSmoothers,
    dirty: bool,
    snapped: bool,
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
            smoothers: CompressorSmoothers::new(48_000.0),
            dirty: false,
            snapped: false,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.max_block_size = (max_frames as usize).max(1);
        self.compressor
            .prepare(self.sample_rate, self.max_block_size);
        self.compressor.set_params(self.params);
        self.smoothers = CompressorSmoothers::new(self.sample_rate);
        self.snapped = false;
    }

    fn reset(&mut self) {
        self.compressor.reset();
        self.snapped = false;
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
        // The smoothed params land per chunk in process(); non-smoothed
        // ones ride along on the same set_params push.
        self.dirty = true;
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        match ctx.stereo_io() {
            Some(io) => {
                io.output_l.copy_from_slice(io.input_l);
                io.output_r.copy_from_slice(io.input_r);
                process_smoothed(
                    &mut self.compressor,
                    &mut self.smoothers,
                    self.params,
                    &mut self.dirty,
                    &mut self.snapped,
                    self.sample_rate,
                    io.output_l,
                    io.output_r,
                );
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
    init_plugin::<ZAudioWebCompressor>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn fresh(sample_rate: f32) -> Compressor {
        let mut c = Compressor::default();
        c.prepare(sample_rate, 4_096);
        c.set_params(CompressorParams::default());
        c
    }

    #[test]
    fn chunked_processing_matches_single_shot_for_constant_params() {
        let sr = 48_000.0;
        let input = noise(4_096, 0.5);
        let (mut a_l, mut a_r) = (input.clone(), input.clone());
        let (mut b_l, mut b_r) = (input.clone(), input.clone());

        let mut single = fresh(sr);
        let events = [];
        let pctx = ProcessContext::new(sr, a_l.len(), 120.0, &events);
        single.process_stereo(&pctx, &mut a_l, &mut a_r);

        let mut chunked = fresh(sr);
        let mut sm = CompressorSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        process_smoothed(
            &mut chunked,
            &mut sm,
            CompressorParams::default(),
            &mut dirty,
            &mut snapped,
            sr,
            &mut b_l,
            &mut b_r,
        );

        assert_eq!(a_l, b_l);
        assert_eq!(a_r, b_r);
    }

    #[test]
    fn makeup_gain_jump_is_smoothed() {
        // Jump makeup gain +20 dB mid-render: the level must glide over the
        // smoothing window, not step at the block boundary.
        let sr = 48_000.0;
        let n = 9_600;
        let input = noise(n, 0.1);
        let (mut l, mut r) = (input.clone(), input.clone());
        let mut comp = fresh(sr);
        let mut sm = CompressorSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        let mut params = CompressorParams::default();
        let half = n / 2;
        let (l_first, l_rest) = l.split_at_mut(half);
        let (r_first, r_rest) = r.split_at_mut(half);
        process_smoothed(
            &mut comp, &mut sm, params, &mut dirty, &mut snapped, sr, l_first, r_first,
        );
        params.makeup_gain_db += 20.0;
        dirty = true;
        process_smoothed(
            &mut comp, &mut sm, params, &mut dirty, &mut snapped, sr, l_rest, r_rest,
        );
        let rms = |buf: &[f32]| -> f32 {
            (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
        };
        let just_after = rms(&l_rest[..64]);
        let settled = rms(&l_rest[l_rest.len() - 1_024..]);
        assert!(
            just_after < settled * 0.5,
            "gain landed instantly: just_after={just_after} settled={settled}"
        );
    }
}
