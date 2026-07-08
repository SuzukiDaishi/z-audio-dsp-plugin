//! Z Audio Chorus — stereo multi-voice chorus. Each channel runs 1-3
//! modulated delay taps off a shared 64 ms delay line; a single sine LFO
//! phase accumulator drives every voice with per-voice phase offsets, and
//! the right channel adds a spread offset (up to half a cycle) for stereo
//! width. Dry/wet mix and output trim round it out.
//!
//! Web ids 820-825 — a fresh block (delay uses 800s). A future native
//! build must mirror these ids one-to-one.
//!
//! Per voice v (0..voices-1), phase offset v/voices (plus spread*0.5 on
//! the right channel):
//!
//! `delay_ms = 7 + depth * 8 * (0.5 + 0.5 * sin(2π·phase)) + v * 3`
//!
//! The wet signal is the average of the voice taps (linear-interpolated
//! reads). Worst case delay is 7 + 8 + 2*3 = 21 ms, well inside the line.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

pub const P_RATE: u32 = 820;
pub const P_DEPTH: u32 = 821;
pub const P_VOICES: u32 = 822;
pub const P_SPREAD: u32 = 823;
pub const P_MIX: u32 = 824;
pub const P_OUTPUT: u32 = 825;

/// Delay line length in seconds (64 ms).
pub const LINE_SECONDS: f32 = 0.064;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.chorus\0",
    name: b"Z Audio Chorus\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Multi-voice stereo chorus with LFO spread\0",
    features: &[b"audio-effect\0", b"chorus\0", b"stereo\0"],
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
        def(P_RATE, b"Rate\0", 0.05, 8.0, 0.8, false),
        def(P_DEPTH, b"Depth\0", 0.0, 1.0, 0.5, false),
        def(P_VOICES, b"Voices\0", 1.0, 3.0, 2.0, true),
        def(P_SPREAD, b"Spread\0", 0.0, 1.0, 0.7, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 0.5, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

#[derive(Clone, Copy)]
pub struct ChorusParams {
    pub rate_hz: f32,
    pub depth: f32,
    pub voices: u8,
    pub spread: f32,
    pub mix: f32,
    pub output_db: f32,
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self {
            rate_hz: 0.8,
            depth: 0.5,
            voices: 2,
            spread: 0.7,
            mix: 0.5,
            output_db: 0.0,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Modulated delay for voice `voice` at LFO `phase` (cycles), exposed so
/// the UI mirrors it exactly: 7 ms base + depth-scaled 0..8 ms sine sweep
/// + 3 ms static per-voice offset.
#[inline]
pub fn voice_delay_ms(depth: f32, voice: usize, phase: f32) -> f32 {
    7.0 + depth * 8.0 * (0.5 + 0.5 * (core::f32::consts::TAU * phase).sin()) + voice as f32 * 3.0
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

pub struct ChorusEngine {
    params: ChorusParams,
    sample_rate: f32,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write: usize,
    /// Shared LFO phase in [0, 1); per-voice/channel offsets are added on
    /// read, so no per-voice accumulators can drift apart.
    phase: f32,
}

impl ChorusEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        // 64 ms plus interpolation guard samples; allocated here, never in
        // process().
        let len = (LINE_SECONDS * sample_rate) as usize + 4;
        Self {
            params: ChorusParams::default(),
            sample_rate,
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            write: 0,
            phase: 0.0,
        }
    }

    pub fn params(&self) -> &ChorusParams {
        &self.params
    }

    pub fn set_params(&mut self, p: ChorusParams) {
        self.params = p;
    }

    pub fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write = 0;
        self.phase = 0.0;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let p = self.params;
        let n = self.buf_l.len();
        let max_del = (n - 2) as f32;
        let ms_to_samples = self.sample_rate / 1_000.0;
        let inc = p.rate_hz / self.sample_rate;
        let voices = p.voices.clamp(1, 3) as usize;
        let voice_gain = 1.0 / voices as f32;
        let dry = 1.0 - p.mix;
        let out_gain = db_to_gain(p.output_db);
        for i in 0..out_l.len() {
            self.buf_l[self.write] = in_l[i];
            self.buf_r[self.write] = in_r[i];
            let mut wet_l = 0.0f32;
            let mut wet_r = 0.0f32;
            for v in 0..voices {
                let phase_l = self.phase + v as f32 / voices as f32;
                // Right channel: spread shifts the LFO by 0..0.5 cycles.
                let phase_r = phase_l + p.spread * 0.5;
                let del_l =
                    (voice_delay_ms(p.depth, v, phase_l) * ms_to_samples).clamp(1.0, max_del);
                let del_r =
                    (voice_delay_ms(p.depth, v, phase_r) * ms_to_samples).clamp(1.0, max_del);
                wet_l += read_tap(&self.buf_l, self.write, del_l);
                wet_r += read_tap(&self.buf_r, self.write, del_r);
            }
            wet_l *= voice_gain;
            wet_r *= voice_gain;
            out_l[i] = (in_l[i] * dry + wet_l * p.mix) * out_gain;
            out_r[i] = (in_r[i] * dry + wet_r * p.mix) * out_gain;
            self.write += 1;
            if self.write >= n {
                self.write = 0;
            }
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

pub fn apply_param(p: &mut ChorusParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_RATE => p.rate_hz = v.clamp(0.05, 8.0),
        P_DEPTH => p.depth = v.clamp(0.0, 1.0),
        P_VOICES => p.voices = v.clamp(1.0, 3.0).round() as u8,
        P_SPREAD => p.spread = v.clamp(0.0, 1.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &ChorusParams, id: u32) -> f64 {
    (match id {
        P_RATE => p.rate_hz,
        P_DEPTH => p.depth,
        P_VOICES => p.voices as f32,
        P_SPREAD => p.spread,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebChorus {
    engine: ChorusEngine,
}

impl Plugin for ZAudioWebChorus {
    fn new() -> Self {
        Self {
            engine: ChorusEngine::new(48_000.0),
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = ChorusEngine::new(sample_rate as f32);
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
    init_plugin::<ZAudioWebChorus>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn engine_with(edit: impl FnOnce(&mut ChorusParams)) -> ChorusEngine {
        let mut e = ChorusEngine::new(SR);
        let mut p = *e.params();
        edit(&mut p);
        e.set_params(p);
        e
    }

    fn sine(len: usize) -> Vec<f32> {
        (0..len).map(|i| (i as f32 * 0.06).sin() * 0.5).collect()
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 6);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((820..=825).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = ChorusParams::default();
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
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = engine_with(|p| p.mix = 0.0);
        let input = sine(256);
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn full_wet_actually_modulates_the_signal() {
        let mut e = engine_with(|p| {
            p.mix = 1.0;
            p.depth = 1.0;
            p.rate_hz = 2.0;
        });
        let n = 48_000;
        let input = sine(n);
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        e.process(&input, &input, &mut l, &mut r);
        let peak = l.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= 1.0, "peak {peak}");
        assert!(peak > 0.1, "output nearly silent: {peak}");
        // Normalised correlation with the dry input must show the LFO
        // actually moved the taps.
        let dot: f32 = input.iter().zip(l.iter()).map(|(a, b)| a * b).sum();
        let ex: f32 = input.iter().map(|s| s * s).sum();
        let ey: f32 = l.iter().map(|s| s * s).sum();
        let corr = dot / (ex * ey).sqrt().max(1e-12);
        assert!(corr < 0.999, "output identical to input, corr {corr}");
    }

    #[test]
    fn voice_count_changes_the_output() {
        let n = 24_000;
        let input = sine(n);
        let mut one = engine_with(|p| {
            p.mix = 1.0;
            p.voices = 1;
        });
        let (mut l1, mut r1) = (vec![0.0; n], vec![0.0; n]);
        one.process(&input, &input, &mut l1, &mut r1);
        let mut three = engine_with(|p| {
            p.mix = 1.0;
            p.voices = 3;
        });
        let (mut l3, mut r3) = (vec![0.0; n], vec![0.0; n]);
        three.process(&input, &input, &mut l3, &mut r3);
        let diff: f32 = l1.iter().zip(l3.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 1.0, "1 vs 3 voices identical, diff {diff}");
    }

    #[test]
    fn spread_decorrelates_the_channels() {
        let n = 24_000;
        let input = sine(n);

        let mut wide = engine_with(|p| {
            p.mix = 1.0;
            p.spread = 1.0;
        });
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        wide.process(&input, &input, &mut l, &mut r);
        let diff: f32 = l.iter().zip(r.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 1.0, "spread=1 should decorrelate, diff {diff}");

        let mut mono = engine_with(|p| {
            p.mix = 1.0;
            p.spread = 0.0;
        });
        let (mut ml, mut mr) = (vec![0.0; n], vec![0.0; n]);
        mono.process(&input, &input, &mut ml, &mut mr);
        for (a, b) in ml.iter().zip(mr.iter()) {
            assert!((a - b).abs() < 1e-9, "spread=0 channels differ: {a} vs {b}");
        }
    }

    #[test]
    fn output_stays_bounded_on_noise() {
        let mut e = engine_with(|p| {
            p.mix = 1.0;
            p.depth = 1.0;
            p.voices = 3;
            p.rate_hz = 8.0;
        });
        let mut seed = 0x8bad_f00du32;
        let input: Vec<f32> = (0..48_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / 8_388_608.0 - 1.0) * 0.9
            })
            .collect();
        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        e.process(&input, &input, &mut l, &mut r);
        for s in l.iter().chain(r.iter()) {
            assert!(s.is_finite());
            assert!(s.abs() <= 1.2, "sample {s} out of bounds");
        }
    }
}
