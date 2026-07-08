//! Z Audio Granular — a Phase Plant-style granular synthesizer WCLAP
//! instrument.
//!
//! The UI (see `ui/`) loads an audio file with the browser's File API,
//! decodes it with `decodeAudioData`, and streams the PCM to this plugin
//! over the `clap.webview/3` binary message channel (see [`protocol`]).
//! MIDI notes trigger grain streams around the automatable **Position**
//! parameter (the seek bar); see [`engine::GranularEngine`] for the whole
//! granular model (spawn modes, randomization, chords, warm start …).
//!
//! Like the sampler, only parameters persist in host projects; the sample
//! PCM does not, so files must be reloaded after reopening a project.
//! Until a file is loaded, an embedded preview bank (mono piano, see
//! `cargo xtask prepare-sampler-bank`) is the grain source so the
//! instrument makes sound out of the box.
//!
//! The [`engine`], [`params`] and [`protocol`] modules are `pub` because
//! this crate doubles as the engine/protocol library for the native
//! VST3/CLAP build (`crates/z-audio-granular-plugin`), which links it as
//! an rlib.
//!
//! Tempo: the WebCLAP scaffold exposes no host transport, so Sync spawn
//! mode runs at the engine's 120 BPM default here; the native build feeds
//! the real host tempo per block.

pub mod engine;
pub mod params;
pub mod protocol;

use std::sync::OnceLock;

use engine::{GranularEngine, GranularParams, SourceSample};
use params::*;
use protocol::{encode_activity, encode_status, parse_ui_message, UiMessage};
use wclap_plugin::{
    init_plugin, send_to_ui, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx,
    ProcessStatus,
};

const DEV_BANK_BYTES: &[u8] = include_bytes!("../../../assets/sampler/piano-dev.bank");

/// Embedded startup preview sample, shared by the WebCLAP and native
/// builds. Returns the source plus its root note.
pub fn dev_bank() -> Option<(SourceSample, u8)> {
    parse_dev_bank(DEV_BANK_BYTES)
}

/// Same `ZSMPLBNK` blob format the sampler embeds (magic · u32 version ·
/// f32 rate · u8 channels · u8 root · u32 frames · i16le PCM).
fn parse_dev_bank(bytes: &[u8]) -> Option<(SourceSample, u8)> {
    let magic = bytes.get(..8)?;
    if magic != b"ZSMPLBNK" {
        return None;
    }
    let version = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    if version != 1 {
        return None;
    }
    let sample_rate = f32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?);
    let channels = *bytes.get(16)?;
    let root = *bytes.get(17)?;
    let frames = u32::from_le_bytes(bytes.get(18..22)?.try_into().ok()?) as usize;
    let total = frames.checked_mul(channels.max(1) as usize)?;
    let pcm_bytes = bytes.get(22..22 + total * 2)?;
    let mut data = Vec::with_capacity(total);
    for i in 0..total {
        let v = i16::from_le_bytes([pcm_bytes[i * 2], pcm_bytes[i * 2 + 1]]);
        data.push(v as f32 / i16::MAX as f32);
    }
    Some((
        SourceSample {
            sample_rate,
            channels: channels.max(1),
            data,
        },
        root.min(127),
    ))
}

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.granular\0",
    name: b"Z Audio Granular\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description:
        b"Granular synth: load a file, automate the play position, grains bloom around it\0",
    features: &[
        b"instrument\0",
        b"synthesizer\0",
        b"granular\0",
        b"stereo\0",
    ],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

/// Reads one engine param by web id (shared by `get_param` and tests).
pub fn param_value(p: &GranularParams, id: u32) -> f64 {
    (match id {
        P_LEVEL => p.level,
        P_PITCH => p.pitch_semitones,
        P_FINE => p.fine_cents,
        P_POSITION => p.position,
        P_GRAIN_LENGTH => p.grain_length_ms,
        P_LENGTH_KEYTRACK => p.length_keytrack as u8 as f32,
        P_GRAIN_ATTACK => p.grain_attack,
        P_GRAIN_DECAY => p.grain_decay,
        P_ATTACK_CURVE => p.attack_curve,
        P_DECAY_CURVE => p.decay_curve,
        P_SPAWN_MODE => p.spawn_mode as f32,
        P_RATE => p.rate_hz,
        P_SYNC_RATE => p.sync_index as f32,
        P_DENSITY => p.density,
        P_ROOT_NOTE => p.root_note as f32,
        P_ALIGN_PHASES => p.align_phases as u8 as f32,
        P_WARM_START => p.warm_start as u8 as f32,
        P_RANDOM_POSITION => p.random_position_ms,
        P_RANDOM_TIMING => p.random_timing,
        P_RANDOM_PITCH => p.random_pitch,
        P_RANDOM_LEVEL => p.random_level,
        P_RANDOM_PAN => p.random_pan,
        P_RANDOM_REVERSE => p.random_reverse,
        P_CHORD_TYPE => p.chord_type as f32,
        P_CHORD_RANGE => p.chord_range as f32,
        P_CHORD_PATTERN => p.chord_pattern as f32,
        P_AMP_ATTACK => p.amp_attack_s,
        P_AMP_DECAY => p.amp_decay_s,
        P_AMP_SUSTAIN => p.amp_sustain,
        P_AMP_RELEASE => p.amp_release_s,
        _ => 0.0,
    }) as f64
}

/// Writes one engine param by web id, clamped to its declared range
/// (shared by `set_param` and tests).
pub fn apply_param(p: &mut GranularParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_LEVEL => p.level = v.clamp(0.0, 2.0),
        P_PITCH => p.pitch_semitones = v.clamp(-48.0, 48.0).round(),
        P_FINE => p.fine_cents = v.clamp(-100.0, 100.0),
        P_POSITION => p.position = v.clamp(0.0, 1.0),
        P_GRAIN_LENGTH => p.grain_length_ms = v.clamp(2.0, 1000.0),
        P_LENGTH_KEYTRACK => p.length_keytrack = v >= 0.5,
        P_GRAIN_ATTACK => p.grain_attack = v.clamp(0.0, 1.0),
        P_GRAIN_DECAY => p.grain_decay = v.clamp(0.0, 1.0),
        P_ATTACK_CURVE => p.attack_curve = v.clamp(-1.0, 1.0),
        P_DECAY_CURVE => p.decay_curve = v.clamp(-1.0, 1.0),
        P_SPAWN_MODE => p.spawn_mode = v.clamp(0.0, 2.0).round() as u8,
        P_RATE => p.rate_hz = v.clamp(0.1, 400.0),
        P_SYNC_RATE => p.sync_index = v.clamp(0.0, 8.0).round() as u8,
        P_DENSITY => p.density = v.clamp(0.5, 64.0),
        P_ROOT_NOTE => p.root_note = v.clamp(0.0, 127.0).round() as u8,
        P_ALIGN_PHASES => p.align_phases = v >= 0.5,
        P_WARM_START => p.warm_start = v >= 0.5,
        P_RANDOM_POSITION => p.random_position_ms = v.clamp(0.0, 2000.0),
        P_RANDOM_TIMING => p.random_timing = v.clamp(0.0, 1.0),
        P_RANDOM_PITCH => p.random_pitch = v.clamp(0.0, 24.0),
        P_RANDOM_LEVEL => p.random_level = v.clamp(0.0, 1.0),
        P_RANDOM_PAN => p.random_pan = v.clamp(0.0, 1.0),
        P_RANDOM_REVERSE => p.random_reverse = v.clamp(0.0, 1.0),
        P_CHORD_TYPE => p.chord_type = v.clamp(0.0, 9.0).round() as u8,
        P_CHORD_RANGE => p.chord_range = v.clamp(1.0, 4.0).round() as u8,
        P_CHORD_PATTERN => p.chord_pattern = v.clamp(0.0, 3.0).round() as u8,
        P_AMP_ATTACK => p.amp_attack_s = v.clamp(0.001, 5.0),
        P_AMP_DECAY => p.amp_decay_s = v.clamp(0.0, 5.0),
        P_AMP_SUSTAIN => p.amp_sustain = v.clamp(0.0, 1.0),
        P_AMP_RELEASE => p.amp_release_s = v.clamp(0.01, 10.0),
        _ => {}
    }
}

struct ZAudioWebGranular {
    engine: GranularEngine,
    note_events: Vec<wclap_plugin::NoteEvent>,
    left: Vec<f32>,
    right: Vec<f32>,
    /// UI has said `ready` at least once, so activity pushes have a peer.
    ui_seen: bool,
    /// Output samples until the next grain-activity push (~30 Hz).
    activity_countdown: usize,
    /// Skip redundant empty Activity packets.
    last_activity_empty: bool,
    sample_rate: f32,
}

impl ZAudioWebGranular {
    fn push_status(&self) {
        let (frames, rate, channels) = self.engine.source_info();
        send_to_ui(&encode_status(
            self.engine.has_sample(),
            channels,
            frames,
            rate,
        ));
    }

    fn push_activity(&mut self) {
        let mut positions = [0.0f32; protocol::MAX_ACTIVITY_GRAINS];
        let n = self.engine.grain_positions(&mut positions);
        if n == 0 && self.last_activity_empty {
            return;
        }
        self.last_activity_empty = n == 0;
        send_to_ui(&encode_activity(&positions[..n]));
    }
}

impl Plugin for ZAudioWebGranular {
    fn new() -> Self {
        let mut engine = GranularEngine::new(48_000.0);
        if let Some((source, root)) = dev_bank() {
            let mut p = *engine.params();
            p.root_note = root;
            engine.set_params(p);
            engine.set_source(source);
        }
        Self {
            engine,
            note_events: Vec::with_capacity(128),
            left: vec![0.0; 128],
            right: vec![0.0; 128],
            ui_seen: false,
            activity_countdown: 0,
            last_activity_empty: true,
            sample_rate: 48_000.0,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.engine.set_sample_rate(sample_rate as f32);
        let frames = (max_frames as usize).max(1);
        self.engine.reserve_block(frames);
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        reserve(&mut self.note_events, frames.max(64));
    }

    fn reset(&mut self) {
        self.engine.reset_voices();
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
            // UI (re)opened: after the scaffold's automatic params snapshot,
            // tell it whether a sample is already loaded.
            self.ui_seen = true;
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
            } => self.engine.begin_upload(sample_rate, channels, frames),
            UiMessage::SampleChunk {
                float_offset,
                pcm_bytes,
            } => self.engine.upload_chunk(float_offset, pcm_bytes),
            UiMessage::CommitSample => {
                self.engine.commit_sample();
                self.push_status();
            }
            UiMessage::NotePreview { on, key, velocity } => {
                if on {
                    self.engine.note_on(key, velocity as f32 / 127.0);
                } else {
                    self.engine.note_off(key);
                }
            }
            UiMessage::ClearSample => {
                self.engine.clear();
                self.push_status();
            }
            UiMessage::PollActivity => self.push_activity(),
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
                self.engine
                    .render(&mut left[start..end], &mut right[start..end]);
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

        // Push grain activity to the UI at ~30 Hz (WebCLAP can push;
        // the native webview instead polls with OP_POLL_ACTIVITY).
        if self.ui_seen {
            self.activity_countdown = self.activity_countdown.saturating_sub(frames);
            if self.activity_countdown == 0 {
                self.activity_countdown = (self.sample_rate / 30.0) as usize;
                self.push_activity();
            }
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
    init_plugin::<ZAudioWebGranular>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_bank_parses() {
        let (source, root) = dev_bank().expect("embedded bank parses");
        assert!(source.frames() > 1_000);
        assert!(root <= 127);
        assert!(source.sample_rate > 8_000.0);
    }

    #[test]
    fn dev_bank_note_renders_audible_output_at_default_params() {
        // End-to-end: default engine (Position 0.0) + embedded bank, one
        // note on, the first 200 ms of grains must carry real signal.
        let (source, _) = dev_bank().expect("embedded bank parses");
        let mut e = engine::GranularEngine::new(48_000.0);
        e.set_source(source);
        e.note_on(60, 0.9);
        let total = 9_600;
        let mut left = vec![0.0f32; total];
        let mut right = vec![0.0f32; total];
        for (l, r) in left.chunks_mut(128).zip(right.chunks_mut(128)) {
            e.render(l, r);
        }
        let rms = (left.iter().map(|s| s * s).sum::<f32>() / total as f32).sqrt();
        assert!(rms > 0.005, "granular dev bank is inaudible (rms {rms})");
    }

    #[test]
    fn dev_bank_is_audible_at_the_default_position() {
        // Regression: the embedded bank used to lead with 1.5 s of digital
        // silence, so grains spawned at the default Position (0.0) read
        // nothing but zeros and the plugin appeared silent.
        let (source, _) = dev_bank().expect("embedded bank parses");
        let peak = source.data.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "dev bank is too quiet (peak {peak})");
        let early = ((source.sample_rate * 0.25) as usize * source.channels.max(1) as usize)
            .min(source.data.len());
        let early_peak = source.data[..early]
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            early_peak > 0.05,
            "dev bank leads with silence (first 250 ms peak {early_peak})"
        );
    }

    #[test]
    fn param_defaults_round_trip_through_the_id_surface() {
        let p = GranularParams::default();
        for def in param_defs() {
            let got = param_value(&p, def.id);
            assert!(
                (got - def.default).abs() < 1.0e-9,
                "param {} default mismatch: engine {} vs surface {}",
                def.id,
                got,
                def.default
            );
        }
    }

    #[test]
    fn apply_param_clamps_to_declared_ranges() {
        let mut p = GranularParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max + 1_000.0);
            assert!(
                param_value(&p, def.id) <= def.max + 1.0e-9,
                "param {} must clamp to max",
                def.id
            );
            apply_param(&mut p, def.id, def.min - 1_000.0);
            assert!(
                param_value(&p, def.id) >= def.min - 1.0e-9,
                "param {} must clamp to min",
                def.id
            );
            // Restore the default so cross-field asserts stay meaningful.
            apply_param(&mut p, def.id, def.default);
        }
        assert_eq!(p, GranularParams::default());
    }
}
