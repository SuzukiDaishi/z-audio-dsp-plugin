use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
    PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};
use z_audio_dsp::{EventKind, ParamId, ParamUnit, ProcessContext, TimedEvent};
use z_audio_synth::{FormulaDrumKit, FormulaDrumKitConfig};

const MAX_POLYPHONY: usize = 64;
const PARAM_IDS: [ParamId; 12] = [
    ParamId::DrumKickLevel,
    ParamId::DrumSnareLevel,
    ParamId::DrumTomLevel,
    ParamId::DrumHatLevel,
    ParamId::DrumCymbalLevel,
    ParamId::DrumTuning,
    ParamId::DrumDecay,
    ParamId::DrumTone,
    ParamId::DrumSnareWire,
    ParamId::DrumRoomAmount,
    ParamId::DrumStereoWidth,
    ParamId::DrumMasterGain,
];

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.formula-drums\0",
    name: b"Z Audio Formula Drum Set\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Modal formula drum set instrument built on z-audio-dsp\0",
    features: &[b"instrument\0", b"drum\0", b"stereo\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: None,
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

struct ZAudioWebDrums {
    drums: FormulaDrumKit,
    sample_rate: f32,
    events: Vec<TimedEvent>,
    note_events: Vec<wclap_plugin::NoteEvent>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Plugin for ZAudioWebDrums {
    fn new() -> Self {
        Self {
            drums: FormulaDrumKit::new(FormulaDrumKitConfig {
                sample_rate: 48_000.0,
                max_block_size: 128,
                max_polyphony: MAX_POLYPHONY,
            }),
            sample_rate: 48_000.0,
            events: Vec::with_capacity(192),
            note_events: Vec::with_capacity(128),
            left: vec![0.0; 128],
            right: vec![0.0; 128],
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        let frames = (max_frames as usize).max(1);
        self.drums = FormulaDrumKit::new(FormulaDrumKitConfig {
            sample_rate: self.sample_rate,
            max_block_size: frames,
            max_polyphony: MAX_POLYPHONY,
        });
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        reserve(&mut self.events, frames.max(96) + PARAM_IDS.len());
        reserve(&mut self.note_events, frames.max(96));
    }

    fn reset(&mut self) {
        self.drums = FormulaDrumKit::new(FormulaDrumKitConfig {
            sample_rate: self.sample_rate,
            max_block_size: self.left.len(),
            max_polyphony: MAX_POLYPHONY,
        });
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(|| PARAM_IDS.iter().copied().map(param_def).collect())
    }

    fn get_param(&self, id: u32) -> f64 {
        id_to_param(id)
            .map(|param| self.drums.param_value(param) as f64)
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
        self.drums.process_with_context(
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
    init_plugin::<ZAudioWebDrums>(&PLUGIN_DEF);
}
