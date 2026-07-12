use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_GAIN, TAU_TIME,
};
use z_audio_dsp::{
    Effect, ParamId, ParamUnit, ParametricReverb, ParametricReverbParams, ProcessContext,
};

/// The reverb consumes params per `set_params` call, so smoothing happens
/// at this sub-block granularity: gliding params are re-pushed once per
/// chunk (0.67 ms at 48 kHz) instead of stepping once per host block.
const SMOOTH_CHUNK: usize = 32;

/// Anti-zipper smoothing of the audibly-jumpy reverb params. Mix, width,
/// early/late and output are gain-like; room size and pre-delay move
/// delay lengths and slew at the slower time constant.
struct ReverbSmoothers {
    mix: Smoothed,
    room: Smoothed,
    predelay: Smoothed,
    width: Smoothed,
    early_late: Smoothed,
    out_db: Smoothed,
}

impl ReverbSmoothers {
    fn new(sample_rate: f32) -> Self {
        let rate = sample_rate.max(1.0) / SMOOTH_CHUNK as f32;
        let mk = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(rate, tau);
            s
        };
        Self {
            mix: mk(TAU_GAIN),
            room: mk(TAU_TIME),
            predelay: mk(TAU_TIME),
            width: mk(TAU_GAIN),
            early_late: mk(TAU_GAIN),
            out_db: mk(TAU_GAIN),
        }
    }

    fn set_targets(&mut self, p: &ParametricReverbParams) {
        self.mix.set_target(p.mix);
        self.room.set_target(p.room_size);
        self.predelay.set_target(p.pre_delay_ms);
        self.width.set_target(p.width);
        self.early_late.set_target(p.early_late_mix);
        self.out_db.set_target(p.output_gain_db);
    }

    fn snap_all(&mut self) {
        self.mix.snap();
        self.room.snap();
        self.predelay.snap();
        self.width.snap();
        self.early_late.snap();
        self.out_db.snap();
    }

    fn all_settled(&self) -> bool {
        self.mix.is_settled(1.0e-4)
            && self.room.is_settled(1.0e-4)
            && self.predelay.is_settled(1.0e-3)
            && self.width.is_settled(1.0e-4)
            && self.early_late.is_settled(1.0e-4)
            && self.out_db.is_settled(1.0e-3)
    }

    fn tick_and_apply(&mut self, base: ParametricReverbParams) -> ParametricReverbParams {
        let mut p = base;
        p.mix = self.mix.tick();
        p.room_size = self.room.tick();
        p.pre_delay_ms = self.predelay.tick();
        p.width = self.width.tick();
        p.early_late_mix = self.early_late.tick();
        p.output_gain_db = self.out_db.tick();
        p
    }
}

/// Chunked processing core (a free function so tests can drive it on
/// plain slices): pushes smoothed params into the effect once per chunk
/// while gliding or dirty, then renders the chunk in place.
#[allow(clippy::too_many_arguments)]
fn process_smoothed(
    reverb: &mut ParametricReverb,
    sm: &mut ReverbSmoothers,
    params: ParametricReverbParams,
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
            reverb.set_params(sm.tick_and_apply(params));
            *dirty = false;
        }
        let process_ctx = ProcessContext::new(sample_rate, m, 120.0, &events);
        reverb.process_stereo(&process_ctx, &mut out_l[at..at + m], &mut out_r[at..at + m]);
        at += m;
    }
}

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
    smoothers: ReverbSmoothers,
    dirty: bool,
    snapped: bool,
}

impl Plugin for ZAudioWebReverb {
    fn new() -> Self {
        let mut reverb = ParametricReverb::default();
        reverb.prepare(48_000.0, 128);
        Self {
            reverb,
            params: ParametricReverbParams::default(),
            sample_rate: 48_000.0,
            smoothers: ReverbSmoothers::new(48_000.0),
            dirty: false,
            snapped: false,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.reverb
            .prepare(self.sample_rate, (max_frames as usize).max(1));
        self.reverb.set_params(self.params);
        self.smoothers = ReverbSmoothers::new(self.sample_rate);
        self.snapped = false;
    }

    fn reset(&mut self) {
        self.reverb.reset();
        self.snapped = false;
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
                    &mut self.reverb,
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
    init_plugin::<ZAudioWebReverb>(&PLUGIN_DEF);
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

    fn fresh(sample_rate: f32) -> ParametricReverb {
        let mut r = ParametricReverb::default();
        r.prepare(sample_rate, 4_096);
        r.set_params(ParametricReverbParams::default());
        r
    }

    #[test]
    fn chunked_processing_matches_single_shot_for_constant_params() {
        // The 32-sample sub-chunking must be inaudible bookkeeping: with
        // constant params it renders bit-identically to one big block.
        let sr = 48_000.0;
        let input = noise(4_096, 0.5);
        let (mut a_l, mut a_r) = (input.clone(), input.clone());
        let (mut b_l, mut b_r) = (input.clone(), input.clone());

        let mut single = fresh(sr);
        let events = [];
        let pctx = ProcessContext::new(sr, a_l.len(), 120.0, &events);
        single.process_stereo(&pctx, &mut a_l, &mut a_r);

        let mut chunked = fresh(sr);
        let mut sm = ReverbSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        process_smoothed(
            &mut chunked,
            &mut sm,
            ParametricReverbParams::default(),
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
    fn output_gain_jump_is_smoothed() {
        // Jump output gain +24 dB mid-render: the level must glide over
        // the smoothing window, not step at the block boundary.
        let sr = 48_000.0;
        let n = 9_600;
        let input = noise(n, 0.4);
        let (mut l, mut r) = (input.clone(), input.clone());
        let mut reverb = fresh(sr);
        let mut sm = ReverbSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        let mut params = ParametricReverbParams::default();
        let half = n / 2;
        let (l_first, l_rest) = l.split_at_mut(half);
        let (r_first, r_rest) = r.split_at_mut(half);
        process_smoothed(
            &mut reverb,
            &mut sm,
            params,
            &mut dirty,
            &mut snapped,
            sr,
            l_first,
            r_first,
        );
        params.output_gain_db += 24.0;
        dirty = true;
        process_smoothed(
            &mut reverb,
            &mut sm,
            params,
            &mut dirty,
            &mut snapped,
            sr,
            l_rest,
            r_rest,
        );
        // Compare short-window RMS right after the jump against the fully
        // settled level: the first window must still be far below it.
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
