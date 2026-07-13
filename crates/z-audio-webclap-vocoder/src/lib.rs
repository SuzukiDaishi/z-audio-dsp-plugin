//! Z Audio Vocoder — classic channel vocoder: the audio input is the
//! modulator, a MIDI-driven polyphonic oscillator bank (with free-run
//! fallback and noise blend) is the carrier, and a log-spaced bandpass
//! filterbank with per-band envelope followers imprints the modulator's
//! spectral envelope onto the carrier. Formant shift remaps the band
//! envelopes fractionally.
//!
//! Web ids 960-969 — a fresh block (OTT 940s is the previous highest).
//! A future native build must mirror these ids one-to-one.
//!
//! First plugin in the family to declare both an audio input and a note
//! input; the on-screen keyboard also drives the carrier through the
//! `ZVCN` webview packet.

pub mod engine;
pub mod protocol;

use std::sync::OnceLock;

use engine::{VocoderEngine, VocoderParams, MAX_BANDS, WAVE_PULSE, WAVE_SAW};
use protocol::{encode_meter, parse_note_preview};
use wclap_plugin::{
    init_plugin, send_to_ui, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx,
    ProcessStatus, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

pub const P_BANDS: u32 = 960;
pub const P_WAVE: u32 = 961;
pub const P_PITCH: u32 = 962;
pub const P_FREERUN: u32 = 963;
pub const P_NOISE: u32 = 964;
pub const P_SHIFT: u32 = 965;
pub const P_ATTACK: u32 = 966;
pub const P_RELEASE: u32 = 967;
pub const P_MIX: u32 = 968;
pub const P_OUTPUT: u32 = 969;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.vocoder\0",
    name: b"Z Audio Vocoder\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Classic channel vocoder with a MIDI/free-run oscillator carrier\0",
    features: &[b"audio-effect\0", b"note-effect\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 1,
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
        def(P_BANDS, b"Bands\0", 8.0, 32.0, 16.0, true),
        def(P_WAVE, b"Carrier Wave\0", 0.0, 2.0, 0.0, true),
        def(P_PITCH, b"Free Pitch\0", 30.0, 1_000.0, 110.0, false),
        def(P_FREERUN, b"Free Run\0", 0.0, 1.0, 1.0, true),
        def(P_NOISE, b"Noise\0", 0.0, 1.0, 0.15, false),
        def(P_SHIFT, b"Formant Shift\0", -8.0, 8.0, 0.0, false),
        def(P_ATTACK, b"Attack\0", 0.1, 50.0, 5.0, false),
        def(P_RELEASE, b"Release\0", 1.0, 500.0, 80.0, false),
        def(P_MIX, b"Mix\0", 0.0, 1.0, 1.0, false),
        def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false),
    ]
}

pub fn apply_param(p: &mut VocoderParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_BANDS => p.bands = v.clamp(8.0, MAX_BANDS as f32).round() as usize,
        P_WAVE => p.wave = v.clamp(WAVE_SAW as f32, WAVE_PULSE as f32).round() as u8,
        P_PITCH => p.pitch_hz = v.clamp(30.0, 1_000.0),
        P_FREERUN => p.free_run = v >= 0.5,
        P_NOISE => p.noise = v.clamp(0.0, 1.0),
        P_SHIFT => p.shift = v.clamp(-8.0, 8.0),
        P_ATTACK => p.attack_ms = v.clamp(0.1, 50.0),
        P_RELEASE => p.release_ms = v.clamp(1.0, 500.0),
        P_MIX => p.mix = v.clamp(0.0, 1.0),
        P_OUTPUT => p.output_db = v.clamp(-24.0, 24.0),
        _ => {}
    }
}

pub fn param_value(p: &VocoderParams, id: u32) -> f64 {
    (match id {
        P_BANDS => p.bands as f32,
        P_WAVE => p.wave as f32,
        P_PITCH => p.pitch_hz,
        P_FREERUN => {
            if p.free_run {
                1.0
            } else {
                0.0
            }
        }
        P_NOISE => p.noise,
        P_SHIFT => p.shift,
        P_ATTACK => p.attack_ms,
        P_RELEASE => p.release_ms,
        P_MIX => p.mix,
        P_OUTPUT => p.output_db,
        _ => 0.0,
    }) as f64
}

struct ZAudioWebVocoder {
    engine: VocoderEngine,
    note_events: Vec<wclap_plugin::NoteEvent>,
    ui_seen: bool,
    meter_countdown: usize,
    sample_rate: f32,
}

impl Plugin for ZAudioWebVocoder {
    fn new() -> Self {
        Self {
            engine: VocoderEngine::new(48_000.0),
            note_events: Vec::with_capacity(128),
            ui_seen: false,
            meter_countdown: 0,
            sample_rate: 48_000.0,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        let params = *self.engine.params();
        self.sample_rate = sample_rate as f32;
        self.engine = VocoderEngine::new(self.sample_rate);
        self.engine.set_params(params);
        let want = (max_frames as usize).max(64);
        if self.note_events.capacity() < want {
            self.note_events
                .reserve_exact(want - self.note_events.capacity());
        }
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

    fn on_ui_message(&mut self, bytes: &[u8]) -> bool {
        if bytes == b"\x65ready" {
            self.ui_seen = true;
            return true;
        }
        if let Some((on, key, velocity)) = parse_note_preview(bytes) {
            if on {
                self.engine.note_on(key, velocity as f32 / 127.0);
            } else {
                self.engine.note_off(key);
            }
            return true;
        }
        false
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        ctx.collect_note_events(&mut self.note_events);

        let Some(io) = ctx.stereo_io() else {
            silence(ctx);
            return ProcessStatus::Continue;
        };
        let frames = io.output_l.len().min(io.output_r.len());

        // Render in segments split at note-event offsets so carrier
        // triggers land sample-accurately within the block.
        let mut start = 0usize;
        let mut event_index = 0usize;
        while start < frames {
            let mut end = frames;
            while event_index < self.note_events.len() {
                let ev = &self.note_events[event_index];
                let at = (ev.time as usize).min(frames);
                if at <= start {
                    let key = ev.key.clamp(0, 127) as u8;
                    match ev.kind {
                        NoteEventKind::On => self.engine.note_on(key, ev.velocity as f32),
                        NoteEventKind::Off => self.engine.note_off(key),
                    }
                    event_index += 1;
                    continue;
                }
                end = at;
                break;
            }
            if end > start {
                self.engine.process(
                    &io.input_l[start..end],
                    &io.input_r[start..end],
                    &mut io.output_l[start..end],
                    &mut io.output_r[start..end],
                );
            }
            start = end;
        }
        while event_index < self.note_events.len() {
            let ev = &self.note_events[event_index];
            let key = ev.key.clamp(0, 127) as u8;
            match ev.kind {
                NoteEventKind::On => self.engine.note_on(key, ev.velocity as f32),
                NoteEventKind::Off => self.engine.note_off(key),
            }
            event_index += 1;
        }

        if self.ui_seen {
            self.meter_countdown = self.meter_countdown.saturating_sub(frames);
            if self.meter_countdown == 0 {
                self.meter_countdown = (self.sample_rate / 30.0) as usize;
                let voices = self.engine.active_voices().min(255) as u8;
                send_to_ui(&encode_meter(voices, self.engine.envelopes()));
            }
        }

        ProcessStatus::Continue
    }
}

#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioWebVocoder>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::band_centers;

    const SR: f32 = 48_000.0;

    fn engine_with(tweak: impl FnOnce(&mut VocoderParams)) -> VocoderEngine {
        let mut e = VocoderEngine::new(SR);
        let mut p = *e.params();
        tweak(&mut p);
        e.set_params(p);
        e
    }

    fn sine(freq: f32, len: usize, amp: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (core::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
            .collect()
    }

    fn run(e: &mut VocoderEngine, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0; input.len()];
        let mut r = vec![0.0; input.len()];
        e.process(input, input, &mut l, &mut r);
        (l, r)
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    #[test]
    fn param_defs_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 10);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((960..=969).contains(&def.id));
            assert!(seen.insert(def.id));
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }

    #[test]
    fn params_round_trip_through_the_id_surface() {
        let mut p = VocoderParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!(
                (param_value(&p, def.id) - def.max).abs() < 1e-6,
                "id {}",
                def.id
            );
        }
    }

    #[test]
    fn apply_param_clamps_to_declared_ranges() {
        let mut p = VocoderParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max + 1_000.0);
            assert!(param_value(&p, def.id) <= def.max + 1e-6, "id {}", def.id);
            apply_param(&mut p, def.id, def.min - 1_000.0);
            assert!(param_value(&p, def.id) >= def.min - 1e-6, "id {}", def.id);
        }
    }

    #[test]
    fn zero_mix_is_a_clean_passthrough() {
        let mut e = engine_with(|p| p.mix = 0.0);
        let input = sine(300.0, 512, 0.7);
        let (l, _) = run(&mut e, &input);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn free_run_produces_output_without_midi() {
        let mut e = engine_with(|_| {});
        let input = sine(400.0, 24_000, 0.5);
        let (l, _) = run(&mut e, &input);
        assert!(rms(&l[12_000..]) > 0.01, "rms {}", rms(&l[12_000..]));
    }

    #[test]
    fn free_run_off_and_no_notes_is_silent() {
        let mut e = engine_with(|p| p.free_run = false);
        let input = sine(400.0, 24_000, 0.5);
        let (l, _) = run(&mut e, &input);
        assert!(rms(&l[4_800..]) < 1e-3, "rms {}", rms(&l[4_800..]));
    }

    #[test]
    fn note_on_gates_output_and_note_off_releases_it() {
        let mut e = engine_with(|p| p.free_run = false);
        let input = sine(400.0, 12_000, 0.5);
        let (before, _) = run(&mut e, &input);
        assert!(rms(&before[6_000..]) < 1e-3);

        e.note_on(57, 0.8);
        let (held, _) = run(&mut e, &input);
        assert!(rms(&held[6_000..]) > 0.01, "rms {}", rms(&held[6_000..]));

        e.note_off(57);
        // Voice ramp (2 ms) + envelope release; well settled after 0.25 s.
        let (after, _) = run(&mut e, &input);
        assert!(rms(&after[6_000..]) < 1e-3, "rms {}", rms(&after[6_000..]));
    }

    #[test]
    fn sine_modulator_excites_the_matching_band() {
        let mut e = engine_with(|_| {});
        let input = sine(1_000.0, 12_000, 0.5);
        let _ = run(&mut e, &input);

        let (centers, _) = band_centers(16);
        let target = (0..16)
            .min_by(|&a, &b| {
                (centers[a] - 1_000.0)
                    .abs()
                    .partial_cmp(&(centers[b] - 1_000.0).abs())
                    .unwrap()
            })
            .unwrap();
        let env = e.envelopes();
        let peak_band = (0..16)
            .max_by(|&a, &b| env[a].partial_cmp(&env[b]).unwrap())
            .unwrap();
        assert!(
            (peak_band as i32 - target as i32).abs() <= 1,
            "peak {peak_band} target {target}"
        );
        for (k, &v) in env.iter().enumerate() {
            if (k as i32 - target as i32).abs() >= 3 {
                assert!(
                    v < env[peak_band] * 0.5,
                    "band {k} = {v} vs peak {}",
                    env[peak_band]
                );
            }
        }
    }

    #[test]
    fn formant_shift_moves_the_dominant_band_up() {
        // Goertzel magnitude at a band center.
        fn goertzel(signal: &[f32], freq: f32) -> f32 {
            let w = core::f32::consts::TAU * freq / SR;
            let coeff = 2.0 * w.cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &x in signal {
                let s0 = x + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            (s1 * s1 + s2 * s2 - coeff * s1 * s2).sqrt() / signal.len() as f32
        }

        let (centers, _) = band_centers(16);
        let mod_band = 6usize;
        let input = sine(centers[mod_band], 48_000, 0.5);

        let mut plain = engine_with(|p| p.noise = 0.0);
        let (out_plain, _) = run(&mut plain, &input);
        let mut shifted = engine_with(|p| {
            p.noise = 0.0;
            p.shift = 4.0;
        });
        let (out_shifted, _) = run(&mut shifted, &input);

        let up_band = mod_band + 4;
        let tail_plain = &out_plain[24_000..];
        let tail_shifted = &out_shifted[24_000..];
        let plain_up = goertzel(tail_plain, centers[up_band]);
        let shifted_up = goertzel(tail_shifted, centers[up_band]);
        assert!(
            shifted_up > plain_up * 2.0,
            "shifted {shifted_up} vs plain {plain_up}"
        );
    }

    #[test]
    fn noise_blend_still_tracks_the_modulator() {
        let mut e = engine_with(|p| p.noise = 1.0);
        let voiced = sine(400.0, 24_000, 0.5);
        let (l, _) = run(&mut e, &voiced);
        assert!(rms(&l[12_000..]) > 0.005, "rms {}", rms(&l[12_000..]));

        // Modulator falls silent: the envelopes must gate the noise off.
        let silence_in = vec![0.0f32; 48_000];
        let (l2, _) = run(&mut e, &silence_in);
        assert!(rms(&l2[24_000..]) < 1e-3, "rms {}", rms(&l2[24_000..]));
    }

    #[test]
    fn output_gain_jump_is_smoothed() {
        let mut e = engine_with(|_| {});
        let n = 9_600;
        let input = sine(400.0, n, 0.5);
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
        let settle = 2_000;
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
}
