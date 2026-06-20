//! Z Audio Simple Synth, packaged as a real WCLAP instrument plugin.
//!
//! This is a `wclap-plugin` (see `crates/wclap-plugin`) front end around
//! `z-audio-synth::SimpleSynth` — the same DSP core used by the native
//! VST3/CLAP build in `crates/z-audio-plugin`. It exports the CLAP ABI
//! directly (`clap_entry`, factory, plugin vtable, audio-ports/note-ports/
//! params/state extensions) as a `wasm32-unknown-unknown` cdylib, loadable
//! by any WCLAP host (e.g. wclap.plinken.org) — no JS-glue wrapper needed.
//!
//! See `docs/z-audio-dsp_plugin_plan/04_webclap_plan.md` for how this
//! supersedes the earlier `wasm-bindgen` + `AudioWorklet` Stage-1 MVP.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{EventKind, ParamId, ParamUnit, ProcessContext, TimedEvent};
use z_audio_synth::{SimpleSynth, SimpleSynthConfig};

/// Fixed polyphony, matching `z-audio-plugin`'s `engine::MAX_POLYPHONY`.
const MAX_POLYPHONY: usize = 16;

fn is_visible_synth_param(id: ParamId) -> bool {
    matches!(
        id,
        ParamId::MasterGain
            | ParamId::GeneratorKind
            | ParamId::GeneratorGain
            | ParamId::GeneratorPulseWidth
            | ParamId::EnvAttack
            | ParamId::EnvDecay
            | ParamId::EnvSustain
            | ParamId::EnvRelease
            | ParamId::EnvCurve
            | ParamId::LfoWaveform
            | ParamId::LfoRateHz
            | ParamId::LfoAmount
            | ParamId::LfoTarget
    )
}

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.simple-synth\0",
    name: b"Z Audio Simple Synth\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"A simple subtractive synthesizer built on z-audio-dsp / z-audio-synth\0",
    features: &[b"instrument\0", b"synthesizer\0", b"stereo\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

/// Builds the static `ParamDef` table from `ParamId::metadata()` — the same
/// Polyphony is fixed, and the synth's internal EQ params are hidden because
/// EQ is shipped as a separate plugin.
fn build_params() -> Vec<ParamDef> {
    ParamId::ALL
        .iter()
        .copied()
        .filter(|id| is_visible_synth_param(*id))
        .map(|id| {
            let m = id.metadata();
            let mut name_bytes = m.name.as_bytes().to_vec();
            name_bytes.push(0);
            // Leaked once per param at first access — `ParamDef::name` must be
            // `&'static [u8]`, and `ParamId::metadata().name` isn't NUL-terminated.
            let name: &'static [u8] = Box::leak(name_bytes.into_boxed_slice());
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
        })
        .collect()
}

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioSimpleSynth {
    synth: SimpleSynth,
    sample_rate: f32,
    events: Vec<TimedEvent>,
    note_events: Vec<wclap_plugin::NoteEvent>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Plugin for ZAudioSimpleSynth {
    fn new() -> Self {
        Self {
            synth: SimpleSynth::new(SimpleSynthConfig {
                sample_rate: 48_000.0,
                max_block_size: 128,
                max_polyphony: MAX_POLYPHONY,
            }),
            sample_rate: 48_000.0,
            events: Vec::with_capacity(160),
            note_events: Vec::with_capacity(128),
            left: vec![0.0; 128],
            right: vec![0.0; 128],
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.synth = SimpleSynth::new(SimpleSynthConfig {
            sample_rate: self.sample_rate,
            max_block_size: (max_frames as usize).max(1),
            max_polyphony: MAX_POLYPHONY,
        });
        let frames = (max_frames as usize).max(1);
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        let wanted_event_capacity = frames.max(64).saturating_add(32);
        if self.events.capacity() < wanted_event_capacity {
            self.events
                .reserve_exact(wanted_event_capacity - self.events.capacity());
        }
        if self.note_events.capacity() < frames.max(64) {
            self.note_events
                .reserve_exact(frames.max(64) - self.note_events.capacity());
        }
    }

    fn reset(&mut self) {
        let cfg = SimpleSynthConfig {
            sample_rate: self.synth.sample_rate(),
            max_block_size: self.synth.max_block_size(),
            max_polyphony: MAX_POLYPHONY,
        };
        self.synth = SimpleSynth::new(cfg);
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(build_params)
    }

    fn get_param(&self, id: u32) -> f64 {
        match ParamId::ALL.iter().copied().find(|p| *p as u32 == id) {
            Some(param_id) => self.synth.param_value(param_id) as f64,
            None => 0.0,
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if let Some(param_id) = ParamId::ALL.iter().copied().find(|p| *p as u32 == id) {
            if self.events.len() < self.events.capacity() {
                self.events.push(TimedEvent {
                    sample_offset: 0,
                    kind: EventKind::Param {
                        id: param_id,
                        value: value as f32,
                    },
                });
            }
        }
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        if frames > self.left.len() || frames > self.right.len() {
            silence(ctx);
            return ProcessStatus::Continue;
        }

        self.left[..frames].fill(0.0);
        self.right[..frames].fill(0.0);

        ctx.collect_note_events(&mut self.note_events);
        for note in &self.note_events {
            let sample_offset = (note.time as usize).min(frames.saturating_sub(1));
            let key = note.key.clamp(0, 127) as u8;
            let kind = match note.kind {
                NoteEventKind::On => EventKind::NoteOn {
                    note: key,
                    velocity: note.velocity as f32,
                },
                NoteEventKind::Off => EventKind::NoteOff {
                    note: key,
                    velocity: note.velocity as f32,
                },
            };
            if self.events.len() < self.events.capacity() {
                self.events.push(TimedEvent {
                    sample_offset,
                    kind,
                });
            }
        }
        self.events.sort_by_key(|e| e.sample_offset);

        let process_ctx =
            ProcessContext::new(self.synth.sample_rate(), frames, 120.0, &self.events);
        self.synth.process_with_context(
            &process_ctx,
            &mut self.left[..frames],
            &mut self.right[..frames],
        );
        self.events.clear();

        let wrote_l = match ctx.output_mut(0, 0) {
            Some(out_l) => {
                out_l[..frames].copy_from_slice(&self.left[..frames]);
                true
            }
            None => false,
        };
        let wrote_r = match ctx.output_mut(0, 1) {
            Some(out_r) => {
                out_r[..frames].copy_from_slice(&self.right[..frames]);
                true
            }
            None => false,
        };
        if !wrote_l && !wrote_r {
            silence(ctx);
        }

        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioSimpleSynth>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_synth_params_match_webclap_ui() {
        let visible: Vec<_> = ParamId::ALL
            .iter()
            .copied()
            .filter(|id| is_visible_synth_param(*id))
            .collect();

        assert_eq!(
            visible,
            vec![
                ParamId::MasterGain,
                ParamId::GeneratorKind,
                ParamId::GeneratorGain,
                ParamId::GeneratorPulseWidth,
                ParamId::EnvAttack,
                ParamId::EnvDecay,
                ParamId::EnvSustain,
                ParamId::EnvRelease,
                ParamId::EnvCurve,
                ParamId::LfoWaveform,
                ParamId::LfoRateHz,
                ParamId::LfoAmount,
                ParamId::LfoTarget,
            ]
        );
    }
}
