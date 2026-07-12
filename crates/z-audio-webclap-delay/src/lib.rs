//! Z Audio Delay — stereo delay with independent (or linked) per-channel
//! times, feedback with in-loop low/high damping filters, an optional
//! ping-pong topology, dry/wet mix and output trim.
//!
//! Web ids 800-808 — a fresh block (ring mod uses 620s, distortion 640s).
//! A future native build must mirror these ids one-to-one.
//!
//! Topology (per sample):
//!
//! * Normal: each channel feeds itself.
//!   `line[ch] <- in[ch] + damp(tap[ch]) * feedback`
//! * Ping-pong: the mono input sum feeds the LEFT line only, and the taps
//!   cross-feed — the L tap into the R line and the R tap into the L line —
//!   so successive echoes alternate L, R, L, R…
//!   `line_l <- (in_l + in_r) * 0.5 + damp(tap_r) * feedback`
//!   `line_r <-                      damp(tap_l) * feedback`
//!
//! The damping filters (one-pole LP, then one-pole HP built as
//! `lp_out - lp2`) sit INSIDE the feedback loop, so every trip around the
//! loop darkens and thins the echo. The wet output is the raw (pre-damp)
//! tap.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED, TAU_FREQ, TAU_GAIN,
};

pub const P_TIME_L: u32 = 800;
pub const P_TIME_R: u32 = 801;
pub const P_LINK: u32 = 802;
pub const P_FEEDBACK: u32 = 803;
pub const P_PINGPONG: u32 = 804;
pub const P_DAMP_LP: u32 = 805;
pub const P_DAMP_HP: u32 = 806;
pub const P_MIX: u32 = 807;
pub const P_OUTPUT: u32 = 808;

/// Maximum delay time the buffers are sized for, in seconds.
pub const MAX_DELAY_SECONDS: f32 = 2.0;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.delay\0",
    name: b"Z Audio Delay\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Stereo delay with ping-pong and damped feedback\0",
    features: &[b"audio-effect\0", b"delay\0", b"stereo\0"],
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
        def(P_TIME_L, b"Time L\0", 1.0, 2_000.0, 350.0, false),
        def(P_TIME_R, b"Time R\0", 1.0, 2_000.0, 350.0, false),
        def(P_LINK, b"Link\0", 0.0, 1.0, 1.0, true),
        def(P_FEEDBACK, b"Feedback\0", 0.0, 0.95, 0.4, false),
        def(P_PINGPONG, b"Ping Pong\0", 0.0, 1.0, 0.0, true),
        def(P_DAMP_LP, b"Damp LP\0", 500.0, 20_000.0, 8_000.0, false),
        def(P_DAMP_HP, b"Damp HP\0", 10.0, 2_000.0, 60.0, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 0.35, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct DelayParams {
    pub time_l_ms: f32,
    pub time_r_ms: f32,
    pub link: bool,
    pub feedback: f32,
    pub ping_pong: bool,
    pub damp_lp_hz: f32,
    pub damp_hp_hz: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            time_l_ms: 350.0,
            time_r_ms: 350.0,
            link: true,
            feedback: 0.4,
            ping_pong: false,
            damp_lp_hz: 8_000.0,
            damp_hp_hz: 60.0,
            mix: 0.35,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// A per-sample smoother starting at 0 (snapped to its real target on the
/// engine's first processed block).
fn gain_smoother(sample_rate: f32, tau: f32) -> Smoothed {
    let mut s = Smoothed::new(0.0);
    s.configure(sample_rate, tau);
    s
}

/// Linear-interpolated read `delay` samples behind `write`.
#[inline]
fn read_tap(buf: &[f32], write: usize, delay: f32) -> f32 {
    let n = buf.len();
    // `delay` is clamped to [1, n - 2] by the caller, so `pos` stays >= 0.
    let pos = write as f32 - delay + n as f32;
    let base = pos.floor();
    let frac = pos - base;
    let i0 = (base as usize) % n;
    let i1 = (i0 + 1) % n;
    buf[i0] + (buf[i1] - buf[i0]) * frac
}

/// Per-channel feedback damping: one-pole LP, then one-pole HP realised as
/// `lp_out - lp2` (lp2 tracks the low end that the HP removes).
#[derive(Default, Clone, Copy)]
struct DampState {
    lp: f32,
    lp2: f32,
}

impl DampState {
    #[inline]
    fn tick(&mut self, x: f32, a_lp: f32, a_hp: f32) -> f32 {
        self.lp += a_lp * (x - self.lp);
        self.lp2 += a_hp * (self.lp - self.lp2);
        self.lp - self.lp2
    }
}

pub struct DelayEngine {
    params: DelayParams,
    sample_rate: f32,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write: usize,
    /// Smoothed delay lengths in samples (one-pole toward the target to
    /// avoid clicks on time changes).
    del_l: f32,
    del_r: f32,
    /// The smoothers snap to their targets on the first processed sample
    /// after construction/reset so a fresh engine doesn't sweep up from 0.
    snapped: bool,
    damp_l: DampState,
    damp_r: DampState,
    /// Anti-zipper smoothing for the gain-like params and the damping
    /// coefficients (the delay times have their own slew above).
    sm_feedback: Smoothed,
    sm_mix: Smoothed,
    sm_out: Smoothed,
    sm_a_lp: Smoothed,
    sm_a_hp: Smoothed,
}

impl DelayEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        // 2 s of delay plus interpolation guard samples; allocated here,
        // never in process().
        let len = (MAX_DELAY_SECONDS * sample_rate) as usize + 4;
        Self {
            params: DelayParams::default(),
            sample_rate,
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            write: 0,
            del_l: 0.0,
            del_r: 0.0,
            snapped: false,
            damp_l: DampState::default(),
            damp_r: DampState::default(),
            sm_feedback: gain_smoother(sample_rate, TAU_GAIN),
            sm_mix: gain_smoother(sample_rate, TAU_GAIN),
            sm_out: gain_smoother(sample_rate, TAU_GAIN),
            sm_a_lp: gain_smoother(sample_rate, TAU_FREQ),
            sm_a_hp: gain_smoother(sample_rate, TAU_FREQ),
        }
    }

    pub fn params(&self) -> &DelayParams {
        &self.params
    }

    pub fn set_params(&mut self, p: DelayParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write = 0;
        self.snapped = false;
        self.damp_l = DampState::default();
        self.damp_r = DampState::default();
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let n = self.buf_l.len();
        let max_del = (n - 2) as f32;
        let ms_to_samples = self.sample_rate / 1_000.0;
        // Link: the left time drives both channels.
        let time_r_ms = if p.link { p.time_l_ms } else { p.time_r_ms };
        let target_l = (p.time_l_ms * ms_to_samples).clamp(1.0, max_del);
        let target_r = (time_r_ms * ms_to_samples).clamp(1.0, max_del);
        self.sm_feedback.set_target(p.feedback);
        self.sm_mix.set_target(p.mix);
        self.sm_out.set_target(db_to_gain(p.output_db));
        self.sm_a_lp.set_target(
            1.0 - (-core::f32::consts::TAU * p.damp_lp_hz.min(self.sample_rate * 0.45)
                / self.sample_rate)
                .exp(),
        );
        self.sm_a_hp.set_target(
            1.0 - (-core::f32::consts::TAU * p.damp_hp_hz.min(self.sample_rate * 0.45)
                / self.sample_rate)
                .exp(),
        );
        if !self.snapped {
            self.del_l = target_l;
            self.del_r = target_r;
            self.sm_feedback.snap();
            self.sm_mix.snap();
            self.sm_out.snap();
            self.sm_a_lp.snap();
            self.sm_a_hp.snap();
            self.snapped = true;
        }
        // ~0.0008 per sample at 48 kHz, scaled so the sweep speed is
        // constant in seconds regardless of rate.
        let smooth = (0.0008 * 48_000.0 / self.sample_rate).clamp(0.0, 1.0);
        for i in 0..out_l.len() {
            self.del_l += smooth * (target_l - self.del_l);
            self.del_r += smooth * (target_r - self.del_r);
            let a_lp = self.sm_a_lp.tick();
            let a_hp = self.sm_a_hp.tick();
            let feedback = self.sm_feedback.tick();
            let mix = self.sm_mix.tick();
            let dry = 1.0 - mix;
            let out_gain = self.sm_out.tick();
            let tap_l = read_tap(&self.buf_l, self.write, self.del_l);
            let tap_r = read_tap(&self.buf_r, self.write, self.del_r);
            // Damping sits inside the loop: every round trip is filtered.
            let fb_l = self.damp_l.tick(tap_l, a_lp, a_hp) * feedback;
            let fb_r = self.damp_r.tick(tap_r, a_lp, a_hp) * feedback;
            if p.ping_pong {
                // Mono input into the L line; taps cross-feed L->R->L.
                let mono = (in_l[i] + in_r[i]) * 0.5;
                self.buf_l[self.write] = mono + fb_r;
                self.buf_r[self.write] = fb_l;
            } else {
                self.buf_l[self.write] = in_l[i] + fb_l;
                self.buf_r[self.write] = in_r[i] + fb_r;
            }
            self.write += 1;
            if self.write >= n {
                self.write = 0;
            }
            out_l[i] = (in_l[i] * dry + tap_l * mix) * out_gain;
            out_r[i] = (in_r[i] * dry + tap_r * mix) * out_gain;
        }
    }
}

pub fn apply_param(p: &mut DelayParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_TIME_L => p.time_l_ms = v.clamp(1.0, 2_000.0),
        P_TIME_R => p.time_r_ms = v.clamp(1.0, 2_000.0),
        P_LINK => p.link = v >= 0.5,
        P_FEEDBACK => p.feedback = v.clamp(0.0, 0.95),
        P_PINGPONG => p.ping_pong = v >= 0.5,
        P_DAMP_LP => p.damp_lp_hz = v.clamp(500.0, 20_000.0),
        P_DAMP_HP => p.damp_hp_hz = v.clamp(10.0, 2_000.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &DelayParams, id: u32) -> f64 {
    (match id {
        P_TIME_L => p.time_l_ms,
        P_TIME_R => p.time_r_ms,
        P_LINK => {
            if p.link {
                1.0
            } else {
                0.0
            }
        }
        P_FEEDBACK => p.feedback,
        P_PINGPONG => {
            if p.ping_pong {
                1.0
            } else {
                0.0
            }
        }
        P_DAMP_LP => p.damp_lp_hz,
        P_DAMP_HP => p.damp_hp_hz,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebDelay {
    engine: DelayEngine,
}

impl Plugin for ZAudioWebDelay {
    fn new() -> Self {
        Self {
            engine: DelayEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = DelayEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebDelay>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn engine_with(edit: impl FnOnce(&mut DelayParams)) -> DelayEngine {
        let mut e = DelayEngine::new(SR);
        let mut p = *e.params();
        edit(&mut p);
        e.set_params(p);
        e
    }

    fn peak_in(buf: &[f32], range: std::ops::Range<usize>) -> f32 {
        buf[range].iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 9);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((800..=808).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = DelayParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!(
                (param_value(&p, def.id) - def.max).abs() < 1e-6,
                "id {} max",
                def.id
            );
            apply_param(&mut p, def.id, def.min);
            assert!(
                (param_value(&p, def.id) - def.min).abs() < 1e-6,
                "id {} min",
                def.id
            );
        }
    }

    #[test]
    fn output_gain_jump_is_smoothed() {
        // Jump output +24 dB and mix mid-render: the level must glide, not
        // step at the block boundary.
        let mut e = engine_with(|p| {
            p.mix = 0.2;
            p.feedback = 0.3;
        });
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
        p.mix = 0.5;
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
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = engine_with(|p| p.mix = 0.0);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn impulse_echoes_back_at_the_delay_time() {
        let mut e = engine_with(|p| {
            p.time_l_ms = 250.0;
            p.link = true;
            p.feedback = 0.0;
            p.mix = 1.0;
        });
        let n = 24_000;
        let mut input = vec![0.0f32; n];
        input[0] = 1.0;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(&input, &input, &mut l, &mut r);
        let expected = (0.250 * SR) as usize; // 12_000
        let peak_idx = l
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap()
            .0;
        let tolerance = (expected as f32 * 0.02) as usize;
        assert!(
            peak_idx.abs_diff(expected) <= tolerance,
            "echo at {peak_idx}, expected {expected} ± {tolerance}"
        );
        assert!(l[peak_idx].abs() > 0.5, "echo too quiet: {}", l[peak_idx]);
    }

    #[test]
    fn feedback_produces_a_quieter_second_echo() {
        let mut e = engine_with(|p| {
            p.time_l_ms = 100.0;
            p.link = true;
            p.feedback = 0.5;
            p.mix = 1.0;
        });
        let n = 24_000;
        let mut input = vec![0.0f32; n];
        input[0] = 1.0;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(&input, &input, &mut l, &mut r);
        let first = peak_in(&l, 4_300..5_300); // around 4_800
        let second = peak_in(&l, 9_100..10_100); // around 9_600
        assert!(first > 0.5, "first echo missing: {first}");
        assert!(second > 0.05, "second echo missing: {second}");
        assert!(
            second < first * 0.8,
            "second {second} not quieter than first {first}"
        );
    }

    #[test]
    fn ping_pong_alternates_channels() {
        let mut e = engine_with(|p| {
            p.time_l_ms = 100.0;
            p.link = true;
            p.feedback = 0.7;
            p.ping_pong = true;
            p.mix = 1.0;
        });
        let n = 24_000;
        let mut input = vec![0.0f32; n];
        input[0] = 1.0;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(&input, &input, &mut l, &mut r);
        // First echo (~4_800) rides the L line; second (~9_600) the R line.
        let (l1, r1) = (peak_in(&l, 4_600..5_000), peak_in(&r, 4_600..5_000));
        let (l2, r2) = (peak_in(&l, 9_400..9_800), peak_in(&r, 9_400..9_800));
        assert!(l1 > 0.5, "first echo missing on L: {l1}");
        assert!(l1 > r1 * 5.0, "first echo not mostly L: L {l1} R {r1}");
        assert!(r2 > 0.1, "second echo missing on R: {r2}");
        assert!(r2 > l2 * 5.0, "second echo not mostly R: L {l2} R {r2}");
    }

    #[test]
    fn output_stays_bounded_at_max_feedback() {
        let mut e = engine_with(|p| {
            p.time_l_ms = 80.0;
            p.link = true;
            p.feedback = 0.95;
            p.mix = 1.0;
        });
        // 4 s of pseudo-random noise through the loop, processed in blocks.
        let mut seed = 0x1234_5678u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 8_388_608.0 - 1.0
        };
        let mut peak = 0.0f32;
        let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
        let mut input = vec![0.0f32; 512];
        for _ in 0..(4.0 * SR / 512.0) as usize {
            for s in input.iter_mut() {
                *s = noise() * 0.5;
            }
            e.process(&input, &input, &mut l, &mut r);
            for s in l.iter().chain(r.iter()) {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
        }
        assert!(peak < 20.0, "peak {peak} exploded");
    }
}
