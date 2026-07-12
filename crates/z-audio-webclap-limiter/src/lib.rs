use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_GAIN,
};
use z_audio_dsp::{Effect, Limiter, LimiterParams, ParamId, ParamUnit, ProcessContext};

/// The limiter consumes params per `set_params` call, so smoothing happens
/// at this sub-block granularity: gliding params are re-pushed once per
/// chunk (0.67 ms at 48 kHz) instead of stepping once per host block.
const SMOOTH_CHUNK: usize = 32;

/// Anti-zipper smoothing of the trim gains. Ceiling/threshold stay
/// instant — clamping fast is the limiter's job, and release smooths the
/// recovery anyway.
struct LimiterSmoothers {
    in_db: Smoothed,
    out_db: Smoothed,
}

impl LimiterSmoothers {
    fn new(sample_rate: f32) -> Self {
        let rate = sample_rate.max(1.0) / SMOOTH_CHUNK as f32;
        let mk = || {
            let mut s = Smoothed::new(0.0);
            s.configure(rate, TAU_GAIN);
            s
        };
        Self {
            in_db: mk(),
            out_db: mk(),
        }
    }

    fn set_targets(&mut self, p: &LimiterParams) {
        self.in_db.set_target(p.input_gain_db);
        self.out_db.set_target(p.output_gain_db);
    }

    fn snap_all(&mut self) {
        self.in_db.snap();
        self.out_db.snap();
    }

    fn all_settled(&self) -> bool {
        self.in_db.is_settled(1.0e-3) && self.out_db.is_settled(1.0e-3)
    }

    fn tick_and_apply(&mut self, base: LimiterParams) -> LimiterParams {
        let mut p = base;
        p.input_gain_db = self.in_db.tick();
        p.output_gain_db = self.out_db.tick();
        p
    }
}

/// Chunked processing core (a free function so tests can drive it on
/// plain slices): pushes smoothed params into the effect once per chunk
/// while gliding or dirty, then renders the chunk in place.
#[allow(clippy::too_many_arguments)]
fn process_smoothed(
    limiter: &mut Limiter,
    sm: &mut LimiterSmoothers,
    params: LimiterParams,
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
            limiter.set_params(sm.tick_and_apply(params));
            *dirty = false;
        }
        let process_ctx = ProcessContext::new(sample_rate, m, 120.0, &events);
        limiter.process_stereo(&process_ctx, &mut out_l[at..at + m], &mut out_r[at..at + m]);
        at += m;
    }
}

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
    smoothers: LimiterSmoothers,
    dirty: bool,
    snapped: bool,
}

impl Plugin for ZAudioWebLimiter {
    fn new() -> Self {
        let mut limiter = Limiter::default();
        limiter.prepare(48_000.0, 128);
        Self {
            limiter,
            params: LimiterParams::default(),
            sample_rate: 48_000.0,
            smoothers: LimiterSmoothers::new(48_000.0),
            dirty: false,
            snapped: false,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.limiter
            .prepare(self.sample_rate, (max_frames as usize).max(1));
        self.limiter.set_params(self.params);
        self.smoothers = LimiterSmoothers::new(self.sample_rate);
        self.snapped = false;
    }

    fn reset(&mut self) {
        self.limiter.reset();
        self.snapped = false;
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
                    &mut self.limiter,
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

    fn fresh(sample_rate: f32) -> Limiter {
        let mut lim = Limiter::default();
        lim.prepare(sample_rate, 4_096);
        lim.set_params(LimiterParams::default());
        lim
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
        let mut sm = LimiterSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        process_smoothed(
            &mut chunked,
            &mut sm,
            LimiterParams::default(),
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
    fn input_gain_jump_is_smoothed() {
        // Jump input gain mid-render on a quiet signal (below threshold, so
        // the limiter is transparent): the level must glide.
        let sr = 48_000.0;
        let n = 9_600;
        let input = noise(n, 0.02);
        let (mut l, mut r) = (input.clone(), input.clone());
        let mut limiter = fresh(sr);
        let mut sm = LimiterSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        let mut params = LimiterParams::default();
        let half = n / 2;
        let (l_first, l_rest) = l.split_at_mut(half);
        let (r_first, r_rest) = r.split_at_mut(half);
        process_smoothed(
            &mut limiter,
            &mut sm,
            params,
            &mut dirty,
            &mut snapped,
            sr,
            l_first,
            r_first,
        );
        params.input_gain_db += 20.0;
        dirty = true;
        process_smoothed(
            &mut limiter,
            &mut sm,
            params,
            &mut dirty,
            &mut snapped,
            sr,
            l_rest,
            r_rest,
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
