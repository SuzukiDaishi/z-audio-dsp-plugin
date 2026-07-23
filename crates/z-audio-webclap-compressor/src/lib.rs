//! Z Audio Compressor — WCLAP build of the enhanced feed-forward
//! compressor (see `engine.rs` for the DSP: log-domain ballistics,
//! program-dependent auto release, sidechain HPF, lookahead, auto makeup
//! and a warmth saturator).
//!
//! Params 140-149 mirror `z_audio_dsp::ParamId` one-to-one for backward
//! compatibility; the enhancement block lives at web ids 980-984.

use std::sync::OnceLock;

pub mod engine;
pub mod protocol;

use engine::{EnhancedCompressor, EnhancedCompressorParams, MAX_LOOKAHEAD_MS, SC_HPF_OFF_HZ};
use protocol::encode_meter;
use wclap_plugin::{
    init_plugin, send_to_ui, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    Smoothed, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_FREQ, TAU_GAIN,
};
use z_audio_dsp::{DetectorMode, ParamId, ParamUnit};

/// Enhancement params — a fresh web-id block (vocoder 960s is the
/// previous highest). A future native build must mirror these one-to-one.
pub const P_SC_HPF: u32 = 980;
pub const P_LOOKAHEAD: u32 = 981;
pub const P_AUTO_RELEASE: u32 = 982;
pub const P_AUTO_MAKEUP: u32 = 983;
pub const P_WARMTH: u32 = 984;

/// The compressor consumes params per `set_params` call, so smoothing
/// happens at this sub-block granularity: gliding params are re-pushed
/// once per chunk (0.67 ms at 48 kHz) instead of stepping per host block.
const SMOOTH_CHUNK: usize = 32;

/// Anti-zipper smoothing of the trim gains, mix, warmth and sidechain
/// HPF frequency. Threshold/ratio/knee changes pass through the
/// gain-computer ballistics and stay instant.
struct CompressorSmoothers {
    in_db: Smoothed,
    makeup_db: Smoothed,
    mix: Smoothed,
    warmth: Smoothed,
    sc_hpf: Smoothed,
}

impl CompressorSmoothers {
    fn new(sample_rate: f32) -> Self {
        let rate = sample_rate.max(1.0) / SMOOTH_CHUNK as f32;
        let mk = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(rate, tau);
            s
        };
        Self {
            in_db: mk(TAU_GAIN),
            makeup_db: mk(TAU_GAIN),
            mix: mk(TAU_GAIN),
            warmth: mk(TAU_GAIN),
            sc_hpf: mk(TAU_FREQ),
        }
    }

    fn set_targets(&mut self, p: &EnhancedCompressorParams) {
        self.in_db.set_target(p.input_gain_db);
        self.makeup_db.set_target(p.makeup_gain_db);
        self.mix.set_target(p.mix);
        self.warmth.set_target(p.warmth);
        self.sc_hpf.set_target(p.sc_hpf_hz);
    }

    fn snap_all(&mut self) {
        self.in_db.snap();
        self.makeup_db.snap();
        self.mix.snap();
        self.warmth.snap();
        self.sc_hpf.snap();
    }

    fn all_settled(&self) -> bool {
        self.in_db.is_settled(1.0e-3)
            && self.makeup_db.is_settled(1.0e-3)
            && self.mix.is_settled(1.0e-4)
            && self.warmth.is_settled(1.0e-4)
            && self.sc_hpf.is_settled(1.0e-2)
    }

    fn tick_and_apply(&mut self, base: EnhancedCompressorParams) -> EnhancedCompressorParams {
        let mut p = base;
        p.input_gain_db = self.in_db.tick();
        p.makeup_gain_db = self.makeup_db.tick();
        p.mix = self.mix.tick();
        p.warmth = self.warmth.tick();
        p.sc_hpf_hz = self.sc_hpf.tick();
        p
    }
}

/// Chunked processing core (a free function so tests can drive it on
/// plain slices): pushes smoothed params into the effect once per chunk
/// while gliding or dirty, then renders the chunk in place.
fn process_smoothed(
    compressor: &mut EnhancedCompressor,
    sm: &mut CompressorSmoothers,
    params: EnhancedCompressorParams,
    dirty: &mut bool,
    snapped: &mut bool,
    out_l: &mut [f32],
    out_r: &mut [f32],
) {
    sm.set_targets(&params);
    if !*snapped {
        sm.snap_all();
        *snapped = true;
        *dirty = true;
    }
    let n = out_l.len();
    let mut at = 0;
    while at < n {
        let m = SMOOTH_CHUNK.min(n - at);
        if *dirty || !sm.all_settled() {
            compressor.set_params(sm.tick_and_apply(params));
            *dirty = false;
        }
        compressor.process_stereo(&mut out_l[at..at + m], &mut out_r[at..at + m]);
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
    version: b"0.2.0\0",
    description:
        b"Feed-forward compressor with auto release, sidechain HPF, lookahead and warmth\0",
    features: &[b"audio-effect\0", b"compressor\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioWebCompressor {
    compressor: EnhancedCompressor,
    params: EnhancedCompressorParams,
    sample_rate: f32,
    max_block_size: usize,
    smoothers: CompressorSmoothers,
    dirty: bool,
    snapped: bool,
    ui_seen: bool,
    meter_countdown: usize,
    meter_in_peak: f32,
    meter_out_peak: f32,
}

impl ZAudioWebCompressor {
    fn param_defs() -> Vec<ParamDef> {
        let mut defs: Vec<ParamDef> = PARAM_IDS.iter().copied().map(param_def).collect();
        let manual =
            |id: u32, name: &'static [u8], min: f64, max: f64, default: f64, stepped| ParamDef {
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
            };
        defs.push(manual(
            P_SC_HPF,
            b"SC HPF\0",
            SC_HPF_OFF_HZ as f64,
            500.0,
            SC_HPF_OFF_HZ as f64,
            false,
        ));
        defs.push(manual(
            P_LOOKAHEAD,
            b"Lookahead\0",
            0.0,
            MAX_LOOKAHEAD_MS as f64,
            0.0,
            false,
        ));
        defs.push(manual(
            P_AUTO_RELEASE,
            b"Auto Release\0",
            0.0,
            1.0,
            1.0,
            true,
        ));
        defs.push(manual(P_AUTO_MAKEUP, b"Auto Makeup\0", 0.0, 1.0, 0.0, true));
        defs.push(manual(P_WARMTH, b"Warmth\0", 0.0, 1.0, 0.15, false));
        defs
    }
}

impl Plugin for ZAudioWebCompressor {
    fn new() -> Self {
        let mut compressor = EnhancedCompressor::default();
        compressor.prepare(48_000.0, 128);
        Self {
            compressor,
            params: EnhancedCompressorParams::default(),
            sample_rate: 48_000.0,
            max_block_size: 128,
            smoothers: CompressorSmoothers::new(48_000.0),
            dirty: false,
            snapped: false,
            ui_seen: false,
            meter_countdown: 0,
            meter_in_peak: 0.0,
            meter_out_peak: 0.0,
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
        PARAMS.get_or_init(Self::param_defs)
    }

    fn get_param(&self, id: u32) -> f64 {
        match id {
            P_SC_HPF => return self.params.sc_hpf_hz as f64,
            P_LOOKAHEAD => return self.params.lookahead_ms as f64,
            P_AUTO_RELEASE => return self.params.auto_release as u8 as f64,
            P_AUTO_MAKEUP => return self.params.auto_makeup as u8 as f64,
            P_WARMTH => return self.params.warmth as f64,
            _ => {}
        }
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
        let value = value as f32;
        match id {
            P_SC_HPF => self.params.sc_hpf_hz = value.clamp(SC_HPF_OFF_HZ, 500.0),
            P_LOOKAHEAD => self.params.lookahead_ms = value.clamp(0.0, MAX_LOOKAHEAD_MS),
            P_AUTO_RELEASE => self.params.auto_release = value >= 0.5,
            P_AUTO_MAKEUP => self.params.auto_makeup = value >= 0.5,
            P_WARMTH => self.params.warmth = value.clamp(0.0, 1.0),
            _ => {
                let Some(param_id) = id_to_param(id) else {
                    return;
                };
                let value = value.clamp(param_id.metadata().min, param_id.metadata().max);
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
                    _ => return,
                }
            }
        }
        // The smoothed params land per chunk in process(); non-smoothed
        // ones ride along on the same set_params push.
        self.dirty = true;
    }

    fn latency_samples(&self) -> u32 {
        self.compressor.latency_samples()
    }

    fn on_ui_message(&mut self, bytes: &[u8]) -> bool {
        if bytes == b"\x65ready" {
            self.ui_seen = true;
            return true;
        }
        false
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        match ctx.stereo_io() {
            Some(io) => {
                io.output_l.copy_from_slice(io.input_l);
                io.output_r.copy_from_slice(io.input_r);
                if self.ui_seen {
                    self.meter_in_peak = self.meter_in_peak.max(peak2(io.output_l, io.output_r));
                }
                process_smoothed(
                    &mut self.compressor,
                    &mut self.smoothers,
                    self.params,
                    &mut self.dirty,
                    &mut self.snapped,
                    io.output_l,
                    io.output_r,
                );
                if self.ui_seen {
                    self.meter_out_peak = self.meter_out_peak.max(peak2(io.output_l, io.output_r));
                }
            }
            None => silence(ctx),
        }

        if self.ui_seen {
            self.meter_countdown = self.meter_countdown.saturating_sub(frames);
            if self.meter_countdown == 0 {
                self.meter_countdown = (self.sample_rate / 30.0) as usize;
                send_to_ui(&encode_meter(
                    self.compressor.take_gr_meter(),
                    peak_to_db(self.meter_in_peak),
                    peak_to_db(self.meter_out_peak),
                ));
                self.meter_in_peak = 0.0;
                self.meter_out_peak = 0.0;
            }
        }

        ProcessStatus::Continue
    }
}

fn peak2(l: &[f32], r: &[f32]) -> f32 {
    l.iter()
        .chain(r.iter())
        .fold(0.0_f32, |m, s| m.max(s.abs()))
}

fn peak_to_db(peak: f32) -> f32 {
    if peak <= 1.0e-9 {
        -90.0
    } else {
        (20.0 * peak.log10()).clamp(-90.0, 24.0)
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

#[cfg_attr(target_arch = "wasm32", no_mangle)]
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

    fn fresh(sample_rate: f32) -> EnhancedCompressor {
        let mut c = EnhancedCompressor::default();
        c.prepare(sample_rate, 4_096);
        c.set_params(EnhancedCompressorParams::default());
        c
    }

    #[test]
    fn chunked_processing_matches_single_shot_for_constant_params() {
        let sr = 48_000.0;
        let input = noise(4_096, 0.5);
        let (mut a_l, mut a_r) = (input.clone(), input.clone());
        let (mut b_l, mut b_r) = (input.clone(), input.clone());

        let mut single = fresh(sr);
        single.process_stereo(&mut a_l, &mut a_r);

        let mut chunked = fresh(sr);
        let mut sm = CompressorSmoothers::new(sr);
        let (mut dirty, mut snapped) = (false, false);
        process_smoothed(
            &mut chunked,
            &mut sm,
            EnhancedCompressorParams::default(),
            &mut dirty,
            &mut snapped,
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
        let mut params = EnhancedCompressorParams::default();
        let half = n / 2;
        let (l_first, l_rest) = l.split_at_mut(half);
        let (r_first, r_rest) = r.split_at_mut(half);
        process_smoothed(
            &mut comp,
            &mut sm,
            params,
            &mut dirty,
            &mut snapped,
            l_first,
            r_first,
        );
        params.makeup_gain_db += 20.0;
        dirty = true;
        process_smoothed(
            &mut comp,
            &mut sm,
            params,
            &mut dirty,
            &mut snapped,
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

    #[test]
    fn new_params_round_trip_through_the_plugin() {
        let mut p = ZAudioWebCompressor::new();
        p.set_param(P_SC_HPF, 120.0);
        p.set_param(P_LOOKAHEAD, 5.0);
        p.set_param(P_AUTO_RELEASE, 0.0);
        p.set_param(P_AUTO_MAKEUP, 1.0);
        p.set_param(P_WARMTH, 0.4);
        assert_eq!(p.get_param(P_SC_HPF), 120.0);
        assert_eq!(p.get_param(P_LOOKAHEAD), 5.0);
        assert_eq!(p.get_param(P_AUTO_RELEASE), 0.0);
        assert_eq!(p.get_param(P_AUTO_MAKEUP), 1.0);
        assert!((p.get_param(P_WARMTH) - 0.4).abs() < 1.0e-6);
        // Existing block still works.
        p.set_param(ParamId::CompressorThreshold as u32, -30.0);
        assert_eq!(p.get_param(ParamId::CompressorThreshold as u32), -30.0);
    }
}
