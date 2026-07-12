//! Z Audio Gate — stereo-linked noise gate. A fast peak detector
//! (max of |L|, |R| through a ~0.5 ms attack / ~10 ms decay one-pole)
//! drives an open / hold / close state machine: above the threshold the
//! gate targets unity, below it a hold timer keeps it open, and once the
//! hold expires the gain falls to the range floor. The gain itself moves
//! with separate attack (opening) and release (closing) one-poles and is
//! applied identically to both channels, followed by an output trim.
//!
//! Web ids 900-905 — a fresh block (tremolo uses 880s). A future native
//! build must mirror these ids one-to-one.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus, Smoothed,
    PARAM_IS_AUTOMATABLE, TAU_GAIN,
};

pub const P_THRESHOLD: u32 = 900;
pub const P_ATTACK: u32 = 901;
pub const P_HOLD: u32 = 902;
pub const P_RELEASE: u32 = 903;
pub const P_RANGE: u32 = 904;
pub const P_OUTPUT: u32 = 905;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.gate\0",
    name: b"Z Audio Gate\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Noise gate with hold, range floor and linked detection\0",
    features: &[b"audio-effect\0", b"gate\0", b"stereo\0"],
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
        def(P_THRESHOLD, b"Threshold\0", -70.0, 0.0, -40.0),
        def(P_ATTACK, b"Attack\0", 0.1, 100.0, 1.0),
        def(P_HOLD, b"Hold\0", 0.0, 500.0, 50.0),
        def(P_RELEASE, b"Release\0", 5.0, 2_000.0, 150.0),
        def(P_RANGE, b"Range\0", -80.0, 0.0, -80.0),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0),
    ]
}

#[derive(Clone, Copy)]
pub struct GateParams {
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    pub range_db: f32,
    pub output_db: f32,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            attack_ms: 1.0,
            hold_ms: 50.0,
            release_ms: 150.0,
            range_db: -80.0,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Per-sample one-pole coefficient for a time constant in milliseconds.
#[inline]
fn coef(ms: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (ms.max(1e-3) * 0.001 * sample_rate)).exp()
}

pub struct GateEngine {
    params: GateParams,
    sample_rate: f32,
    /// Peak envelope of max(|l|, |r|).
    env: f32,
    /// Smoothed gain shared by both channels. Starts open (1.0).
    gain: f32,
    /// Hold countdown in samples; reloaded whenever env >= threshold.
    hold_remaining: u32,
    /// Anti-zipper smoothing for the output trim. The gate gain itself
    /// already moves through its attack/release one-poles.
    sm_out: Smoothed,
    snapped: bool,
}

impl GateEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |tau: f32| {
            let mut s = Smoothed::new(0.0);
            s.configure(sr, tau);
            s
        };
        Self {
            params: GateParams::default(),
            sample_rate: sr,
            env: 0.0,
            gain: 1.0,
            hold_remaining: 0,
            sm_out: smoother(TAU_GAIN),
            snapped: false,
        }
    }

    pub fn params(&self) -> &GateParams {
        &self.params
    }

    pub fn set_params(&mut self, p: GateParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 1.0;
        self.hold_remaining = 0;
        self.snapped = false;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let fs = self.sample_rate;
        let thr = db_to_gain(p.threshold_db);
        let range = db_to_gain(p.range_db);
        self.sm_out.set_target(db_to_gain(p.output_db));
        if !self.snapped {
            self.sm_out.snap();
            self.snapped = true;
        }
        // Fixed detector: fast rise so transients open the gate, ~10 ms
        // fall so the envelope rides sine peaks instead of zero crossings.
        let det_att = coef(0.5, fs);
        let det_rel = coef(10.0, fs);
        let att = coef(p.attack_ms, fs);
        let rel = coef(p.release_ms, fs);
        let hold_samples = (p.hold_ms * 0.001 * fs) as u32;
        for i in 0..out_l.len() {
            let x = in_l[i].abs().max(in_r[i].abs());
            let d = if x > self.env { det_att } else { det_rel };
            self.env += d * (x - self.env);
            let target = if self.env >= thr {
                self.hold_remaining = hold_samples;
                1.0
            } else if self.hold_remaining > 0 {
                self.hold_remaining -= 1;
                1.0
            } else {
                range
            };
            let g = if target > self.gain { att } else { rel };
            self.gain += g * (target - self.gain);
            let out_gain = self.sm_out.tick();
            out_l[i] = in_l[i] * self.gain * out_gain;
            out_r[i] = in_r[i] * self.gain * out_gain;
        }
    }
}

pub fn apply_param(p: &mut GateParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_THRESHOLD => p.threshold_db = v.clamp(-70.0, 0.0),
        P_ATTACK => p.attack_ms = v.clamp(0.1, 100.0),
        P_HOLD => p.hold_ms = v.clamp(0.0, 500.0),
        P_RELEASE => p.release_ms = v.clamp(5.0, 2_000.0),
        P_RANGE => p.range_db = v.clamp(-80.0, 0.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &GateParams, id: u32) -> f64 {
    (match id {
        P_THRESHOLD => p.threshold_db,
        P_ATTACK => p.attack_ms,
        P_HOLD => p.hold_ms,
        P_RELEASE => p.release_ms,
        P_RANGE => p.range_db,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebGate {
    engine: GateEngine,
}

impl Plugin for ZAudioWebGate {
    fn new() -> Self {
        Self {
            engine: GateEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = GateEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebGate>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: usize = 48_000;

    fn defaults() -> GateEngine {
        GateEngine::new(FS as f32)
    }

    fn sine(n: usize, hz: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / FS as f32).sin() * amp)
            .collect()
    }

    fn run(e: &mut GateEngine, input: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; input.len()], vec![0.0; input.len()]);
        e.process(input, input, &mut l, &mut r);
        l
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn output_gain_jump_is_smoothed() {
        // A loud steady sine keeps the gate open; jump output +24 dB
        // mid-render: the output must glide, not step.
        let mut e = defaults();
        let n = 9_600;
        let input = sine(n, 1_000.0, 0.5);
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
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 6);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((900..=905).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = GateParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.min);
            assert!((param_value(&p, def.id) - def.min).abs() < 1e-6);
        }
    }

    #[test]
    fn loud_signal_passes_unchanged() {
        // -6 dB sine vs a -40 dB threshold: gate stays open, output == input.
        let mut e = defaults();
        let input = sine(FS, 1_000.0, 0.5);
        let out = run(&mut e, &input);
        for (a, b) in input[FS / 10..].iter().zip(out[FS / 10..].iter()) {
            assert!((a - b).abs() <= 0.01 * a.abs().max(1e-3), "in {a} out {b}");
        }
    }

    #[test]
    fn quiet_signal_is_attenuated_by_range() {
        // -60 dB sine under a -40 dB threshold settles at ~range dB down.
        let mut e = defaults();
        let mut p = *e.params();
        p.hold_ms = 0.0;
        p.release_ms = 50.0;
        p.range_db = -60.0;
        e.set_params(p);
        let input = sine(FS, 1_000.0, 0.001);
        let out = run(&mut e, &input);
        let atten_db = 20.0 * (rms(&out[FS - 4_800..]) / rms(&input[FS - 4_800..])).log10();
        assert!((atten_db + 60.0).abs() < 3.0, "attenuation {atten_db} dB");
    }

    #[test]
    fn hold_keeps_the_gate_open_then_release_closes_it() {
        // 200 ms burst, then a -50 dB probe. With hold 100 ms the probe
        // still passes shortly after the burst, but is gone well after
        // hold + 5 * release.
        let mut e = defaults();
        let mut p = *e.params();
        p.hold_ms = 100.0;
        p.release_ms = 50.0;
        e.set_params(p);
        let burst_len = FS / 5;
        let mut input = sine(burst_len, 1_000.0, 0.5);
        input.extend(sine(FS, 1_000.0, 0.003));
        let out = run(&mut e, &input);
        let probe_rms = rms(&input[burst_len + 1_200..burst_len + 2_400]);
        // 25-50 ms after the burst: inside hold (< 100 ms), still open.
        let early = rms(&out[burst_len + 1_200..burst_len + 2_400]);
        assert!(early > 0.7 * probe_rms, "early {early} probe {probe_rms}");
        // 600-700 ms after the burst: hold + 5 * release long gone, closed.
        let late = rms(&out[burst_len + 28_800..burst_len + 33_600]);
        assert!(late < 0.1 * probe_rms, "late {late} probe {probe_rms}");
    }

    #[test]
    fn attack_opens_the_gate_within_a_few_ms() {
        // 500 ms of silence fully closes the gate (release 20 ms), then a
        // DC step to 0.5: with a 1 ms attack the gain passes 0.5 fast.
        let mut e = defaults();
        let mut p = *e.params();
        p.release_ms = 20.0;
        e.set_params(p);
        let step_at = FS / 2;
        let mut input = vec![0.0f32; step_at];
        input.extend(std::iter::repeat(0.5).take(FS / 10));
        let out = run(&mut e, &input);
        assert!(out[step_at + 240] > 0.25, "5 ms in: {}", out[step_at + 240]);
        // And it really was closed just before the step.
        assert!(out[step_at - 1].abs() < 1e-3);
    }

    #[test]
    fn zero_range_is_a_passthrough() {
        // Range 0 dB means the closed gain equals the open gain: the gate
        // does nothing even to a signal far below the threshold.
        let mut e = defaults();
        let mut p = *e.params();
        p.range_db = 0.0;
        e.set_params(p);
        let input = sine(FS / 2, 1_000.0, 0.001);
        let out = run(&mut e, &input);
        for (a, b) in input.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn output_stays_finite_and_bounded_at_extremes() {
        let mut e = defaults();
        let mut p = *e.params();
        p.threshold_db = 0.0;
        p.attack_ms = 0.1;
        p.hold_ms = 0.0;
        p.release_ms = 5.0;
        p.range_db = -80.0;
        p.output_db = 24.0;
        e.set_params(p);
        let input: Vec<f32> = (0..FS / 2).map(|i| (i as f32 * 0.9).sin()).collect();
        let out = run(&mut e, &input);
        for s in &out {
            assert!(s.is_finite());
            assert!(s.abs() <= 16.0, "sample {s}");
        }
    }
}
