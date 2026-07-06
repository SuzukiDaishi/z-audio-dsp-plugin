//! Z Audio Sampler — a Logic-style multi-zone WCLAP sampler instrument.
//!
//! The UI (see `ui/`) loads an audio file with the browser's File API,
//! decodes it with `decodeAudioData`, optionally auto-slices it, and
//! streams the PCM plus a zone table to this plugin over the
//! `clap.webview/3` binary message channel (see [`protocol`]). The engine
//! ([`engine::ZoneSampler`]) plays whatever zone table it was given:
//! one chromatic zone (Classic / One-Shot mode) or many key-mapped slices
//! (Slice mode).
//!
//! Until a file is loaded, an embedded preview bank (mono piano, see
//! `cargo xtask prepare-sampler-bank`) is mapped as a single chromatic
//! zone so the instrument makes sound out of the box.
//!
//! The [`engine`] and [`protocol`] modules are `pub` because this crate
//! doubles as the engine/protocol library for the native VST3/CLAP build
//! (`crates/z-audio-sampler-plugin`), which links it as an rlib.

pub mod engine;
pub mod protocol;

use std::sync::OnceLock;

use engine::{classic_zone, parse_dev_bank, ZoneSampler};
use protocol::{encode_status, parse_ui_message, UiMessage};
use wclap_plugin::{
    init_plugin, send_to_ui, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx,
    ProcessStatus, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED,
};

const DEV_BANK_BYTES: &[u8] = include_bytes!("../../../assets/sampler/piano-dev.bank");

/// Embedded startup preview bank, shared by the WebCLAP and native builds.
pub fn dev_bank() -> Option<(engine::SourceSample, u8)> {
    engine::parse_dev_bank(DEV_BANK_BYTES)
}

// Parameter IDs — a fresh surface for the multi-zone sampler (the old
// single-sample sampler used 200-213).
const P_MASTER_GAIN: u32 = 300;
const P_ATTACK: u32 = 301;
const P_DECAY: u32 = 302;
const P_SUSTAIN: u32 = 303;
const P_RELEASE: u32 = 304;
const P_TUNE: u32 = 305;
const P_TRANSPOSE: u32 = 306;
const P_VELOCITY: u32 = 307;
const P_WIDTH: u32 = 308;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.sampler\0",
    name: b"Z Audio Sampler\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.2.0\0",
    description: b"Multi-zone sampler: load a file in the UI, auto-slice, map to keys\0",
    features: &[b"instrument\0", b"sampler\0", b"stereo\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

fn param_defs() -> Vec<ParamDef> {
    fn def(
        id: u32,
        name: &'static [u8],
        min: f64,
        max: f64,
        default: f64,
        stepped: bool,
    ) -> ParamDef {
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
    vec![
        def(P_MASTER_GAIN, b"Master Gain\0", -48.0, 12.0, 0.0, false),
        def(P_ATTACK, b"Attack\0", 0.001, 5.0, 0.002, false),
        def(P_DECAY, b"Decay\0", 0.0, 5.0, 0.0, false),
        def(P_SUSTAIN, b"Sustain\0", 0.0, 1.0, 1.0, false),
        def(P_RELEASE, b"Release\0", 0.01, 10.0, 0.25, false),
        def(P_TUNE, b"Tune\0", -100.0, 100.0, 0.0, false),
        def(P_TRANSPOSE, b"Transpose\0", -24.0, 24.0, 0.0, true),
        def(P_VELOCITY, b"Velocity Sens\0", 0.0, 1.0, 1.0, false),
        def(P_WIDTH, b"Stereo Width\0", 0.0, 1.0, 1.0, false),
    ]
}

struct ZAudioWebSampler {
    sampler: ZoneSampler,
    note_events: Vec<wclap_plugin::NoteEvent>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl ZAudioWebSampler {
    fn load_dev_bank(sampler: &mut ZoneSampler) {
        if let Some((source, root)) = parse_dev_bank(DEV_BANK_BYTES) {
            let frames = source.frames() as u32;
            sampler.set_source(source, vec![classic_zone(root, frames)]);
        }
    }

    fn push_status(&self) {
        let (frames, rate, channels) = self.sampler.source_info();
        let status = encode_status(
            self.sampler.has_sample(),
            channels,
            self.sampler.zone_count() as u16,
            frames,
            rate,
        );
        send_to_ui(&status);
    }

    fn param(&self, id: u32) -> f64 {
        let p = self.sampler.params();
        (match id {
            P_MASTER_GAIN => p.master_gain_db,
            P_ATTACK => p.attack_s,
            P_DECAY => p.decay_s,
            P_SUSTAIN => p.sustain,
            P_RELEASE => p.release_s,
            P_TUNE => p.tune_cents,
            P_TRANSPOSE => p.transpose_semitones,
            P_VELOCITY => p.velocity_amount,
            P_WIDTH => p.stereo_width,
            _ => 0.0,
        }) as f64
    }

    fn apply_param(&mut self, id: u32, value: f64) {
        let mut p = *self.sampler.params();
        let v = value as f32;
        match id {
            P_MASTER_GAIN => p.master_gain_db = v.clamp(-48.0, 12.0),
            P_ATTACK => p.attack_s = v.clamp(0.001, 5.0),
            P_DECAY => p.decay_s = v.clamp(0.0, 5.0),
            P_SUSTAIN => p.sustain = v.clamp(0.0, 1.0),
            P_RELEASE => p.release_s = v.clamp(0.01, 10.0),
            P_TUNE => p.tune_cents = v.clamp(-100.0, 100.0),
            P_TRANSPOSE => p.transpose_semitones = v.clamp(-24.0, 24.0).round(),
            P_VELOCITY => p.velocity_amount = v.clamp(0.0, 1.0),
            P_WIDTH => p.stereo_width = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.sampler.set_params(p);
    }
}

impl Plugin for ZAudioWebSampler {
    fn new() -> Self {
        let mut sampler = ZoneSampler::new(48_000.0);
        Self::load_dev_bank(&mut sampler);
        Self {
            sampler,
            note_events: Vec::with_capacity(128),
            left: vec![0.0; 128],
            right: vec![0.0; 128],
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sampler.set_sample_rate(sample_rate as f32);
        let frames = (max_frames as usize).max(1);
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        reserve(&mut self.note_events, frames.max(64));
    }

    fn reset(&mut self) {
        self.sampler.reset_voices();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(param_defs)
    }

    fn get_param(&self, id: u32) -> f64 {
        self.param(id)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        self.apply_param(id, value);
    }

    fn on_ui_message(&mut self, bytes: &[u8]) -> bool {
        if bytes == b"\x65ready" {
            // UI (re)opened: after the scaffold's automatic params snapshot,
            // tell it whether a sample/zone table is already loaded.
            self.push_status();
            return true;
        }
        let Some(msg) = parse_ui_message(bytes) else {
            return false;
        };
        match msg {
            UiMessage::BeginSample {
                sample_rate,
                channels,
                frames,
            } => self.sampler.begin_upload(sample_rate, channels, frames),
            UiMessage::SampleChunk {
                float_offset,
                pcm_bytes,
            } => self.sampler.upload_chunk(float_offset, pcm_bytes),
            UiMessage::CommitZones(zones) => {
                self.sampler.commit_zones(zones);
                self.push_status();
            }
            UiMessage::NotePreview { on, key, velocity } => {
                if on {
                    self.sampler.note_on(key, velocity as f32 / 127.0);
                } else {
                    self.sampler.note_off(key);
                }
            }
            UiMessage::ClearSample => {
                self.sampler.clear();
                self.push_status();
            }
        }
        true
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        if frames == 0 || frames > self.left.len() || frames > self.right.len() {
            silence(ctx);
            return ProcessStatus::Continue;
        }

        ctx.collect_note_events(&mut self.note_events);

        // Render in segments split at note-event offsets so triggers land
        // sample-accurately within the block.
        let left = &mut self.left[..frames];
        let right = &mut self.right[..frames];
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
                        NoteEventKind::On => self.sampler.note_on(key, ev.velocity as f32),
                        NoteEventKind::Off => self.sampler.note_off(key),
                    }
                    event_index += 1;
                    continue;
                }
                end = at;
                break;
            }
            if end > start {
                self.sampler
                    .render(&mut left[start..end], &mut right[start..end]);
            }
            start = end;
        }
        while event_index < self.note_events.len() {
            let ev = &self.note_events[event_index];
            let key = ev.key.clamp(0, 127) as u8;
            match ev.kind {
                NoteEventKind::On => self.sampler.note_on(key, ev.velocity as f32),
                NoteEventKind::Off => self.sampler.note_off(key),
            }
            event_index += 1;
        }

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

// Only exported from the wasm cdylib; the native VST3/CLAP plugin links
// this crate as an rlib and must not re-export a WASI entry point.
#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioWebSampler>(&PLUGIN_DEF);
}
