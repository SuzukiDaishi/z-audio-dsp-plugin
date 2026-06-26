//! Z Audio VCSL Piano, packaged as a real WCLAP instrument plugin.
//!
//! Embeds a small "dev bank" (a handful of notes, mono, truncated, see
//! `cargo xtask prepare-vcsl-piano` in the root repo) generated offline from
//! VCSL Keys' "Grand Piano, K" SFZ/FLAC patch, so the wasm module stays
//! small. The full bank is reserved for the native VST3/CLAP plugin, which
//! loads it from disk; see `docs/VCSLサンプラーピアノ実装計画.md`.

use std::sync::{Arc, OnceLock};

use wclap_plugin::{
    init_plugin, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{EventKind, ParamId, ParamUnit, ProcessContext, TimedEvent};
use z_audio_synth::{VcslPiano, VcslPianoConfig, VcslSampleBank};

const MAX_POLYPHONY: usize = 16;
const PARAM_IDS: [ParamId; 6] = [
    ParamId::VcslMasterGain,
    ParamId::VcslTone,
    ParamId::VcslVelocityCurve,
    ParamId::VcslReleaseLevel,
    ParamId::VcslReleaseTime,
    ParamId::VcslStereoWidth,
];

const DEV_BANK_BYTES: &[u8] = include_bytes!("../../../assets/vcsl-piano/grand-piano-k-dev.bank");

fn embedded_bank() -> Arc<VcslSampleBank> {
    static BANK: OnceLock<Arc<VcslSampleBank>> = OnceLock::new();
    BANK.get_or_init(|| {
        Arc::new(
            z_audio_synth::load_bank_bytes(DEV_BANK_BYTES)
                .expect("embedded VCSL dev bank should be valid"),
        )
    })
    .clone()
}

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.vcsl-piano\0",
    name: b"Z Audio VCSL Piano\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"VCSL Keys sampler piano (preview bank) built on z-audio-dsp\0",
    features: &[b"instrument\0", b"piano\0", b"sampler\0", b"stereo\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioWebVcslPiano {
    piano: VcslPiano,
    sample_rate: f32,
    events: Vec<TimedEvent>,
    note_events: Vec<wclap_plugin::NoteEvent>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Plugin for ZAudioWebVcslPiano {
    fn new() -> Self {
        let mut piano = VcslPiano::new(VcslPianoConfig {
            sample_rate: 48_000.0,
            max_block_size: 128,
            max_polyphony: MAX_POLYPHONY,
        });
        piano.load_bank(embedded_bank());
        Self {
            piano,
            sample_rate: 48_000.0,
            events: Vec::with_capacity(160),
            note_events: Vec::with_capacity(128),
            left: vec![0.0; 128],
            right: vec![0.0; 128],
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        let frames = (max_frames as usize).max(1);
        let mut piano = VcslPiano::new(VcslPianoConfig {
            sample_rate: self.sample_rate,
            max_block_size: frames,
            max_polyphony: MAX_POLYPHONY,
        });
        piano.load_bank(embedded_bank());
        for id in PARAM_IDS {
            piano.set_param(id, self.piano.param_value(id));
        }
        self.piano = piano;
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        reserve(&mut self.events, frames.max(64) + PARAM_IDS.len());
        reserve(&mut self.note_events, frames.max(64));
    }

    fn reset(&mut self) {
        let mut piano = VcslPiano::new(VcslPianoConfig {
            sample_rate: self.sample_rate,
            max_block_size: self.left.len(),
            max_polyphony: MAX_POLYPHONY,
        });
        piano.load_bank(embedded_bank());
        for id in PARAM_IDS {
            piano.set_param(id, self.piano.param_value(id));
        }
        self.piano = piano;
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(|| PARAM_IDS.iter().copied().map(param_def).collect())
    }

    fn get_param(&self, id: u32) -> f64 {
        id_to_param(id)
            .map(|param| self.piano.param_value(param) as f64)
            .unwrap_or(0.0)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if let Some(param_id) = id_to_param(id) {
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
        if frames == 0 || frames > self.left.len() || frames > self.right.len() {
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
        let process_ctx = ProcessContext::new(self.sample_rate, frames, 120.0, &self.events);
        self.piano.process_with_context(
            &process_ctx,
            &mut self.left[..frames],
            &mut self.right[..frames],
        );
        self.events.clear();

        let wrote_l = match ctx.output_mut(0, 0) {
            Some(out) => {
                out[..frames].copy_from_slice(&self.left[..frames]);
                true
            }
            None => false,
        };
        let wrote_r = match ctx.output_mut(0, 1) {
            Some(out) => {
                out[..frames].copy_from_slice(&self.right[..frames]);
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

fn reserve<T>(vec: &mut Vec<T>, wanted: usize) {
    if vec.capacity() < wanted {
        vec.reserve_exact(wanted - vec.capacity());
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
    init_plugin::<ZAudioWebVcslPiano>(&PLUGIN_DEF);
}
