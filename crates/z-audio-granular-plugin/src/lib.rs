//! Native VST3/CLAP build of Z Audio Granular — the Phase Plant-style
//! granular synthesizer. The engine (`GranularEngine`), the `ZGRN` UI
//! protocol, and (on Windows/macOS) the web UI itself are shared with the
//! WebCLAP build in `crates/z-audio-webclap-granular`; this crate is the
//! `nih_plug` adapter.
//!
//! Editor strategy:
//! - Windows/macOS: the WebCLAP `ui/` bundle in a wry webview. Params ride
//!   the shared JSON param sync; sample uploads / keyboard previews /
//!   grain-activity polls ride the `{"type":"bin"}` envelope (see
//!   `crates/z-audio-webview-editor`) into [`shared::GranularShared`].
//! - elsewhere: a reduced egui editor (file load + all parameters).
//!
//! Like the WebCLAP build, only parameters persist in host projects; the
//! sample PCM does not, so files must be reloaded after reopening a
//! project. An embedded piano preview bank is the grain source until a
//! file is loaded.

use std::sync::Arc;

use nih_plug::prelude::*;
use z_audio_webclap_granular::engine::{GranularEngine, GranularParams};

#[cfg(not(any(windows, target_os = "macos")))]
mod decode;
#[cfg(not(any(windows, target_os = "macos")))]
mod editor;
mod shared;

use shared::{GranularShared, GranularUpdate, NotePreview};

/// One note event with its intra-block sample offset.
#[derive(Clone, Copy)]
struct TimedNote {
    at: usize,
    on: bool,
    key: u8,
    velocity: f32,
}

/// Mirrors the WebCLAP param surface (web ids 400-429) one-to-one; the
/// editor mapping in [`ZAudioGranular::editor`] pairs them up, and the
/// `params_mirror_the_webclap_surface` test pins ranges and defaults
/// against `z_audio_webclap_granular::params::param_defs()`.
#[derive(Params)]
pub struct ZAudioGranularParams {
    #[id = "level"]
    pub level: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "fine"]
    pub fine: FloatParam,
    #[id = "position"]
    pub position: FloatParam,
    #[id = "grainlen"]
    pub grain_length: FloatParam,
    #[id = "lenkey"]
    pub length_keytrack: FloatParam,
    #[id = "gatk"]
    pub grain_attack: FloatParam,
    #[id = "gdec"]
    pub grain_decay: FloatParam,
    #[id = "acurve"]
    pub attack_curve: FloatParam,
    #[id = "dcurve"]
    pub decay_curve: FloatParam,
    #[id = "spawnmode"]
    pub spawn_mode: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "syncrate"]
    pub sync_rate: FloatParam,
    #[id = "density"]
    pub density: FloatParam,
    #[id = "root"]
    pub root_note: FloatParam,
    #[id = "align"]
    pub align_phases: FloatParam,
    #[id = "warm"]
    pub warm_start: FloatParam,
    #[id = "rndpos"]
    pub random_position: FloatParam,
    #[id = "rndtime"]
    pub random_timing: FloatParam,
    #[id = "rndpitch"]
    pub random_pitch: FloatParam,
    #[id = "rndlevel"]
    pub random_level: FloatParam,
    #[id = "rndpan"]
    pub random_pan: FloatParam,
    #[id = "rndrev"]
    pub random_reverse: FloatParam,
    #[id = "chordtype"]
    pub chord_type: FloatParam,
    #[id = "chordrange"]
    pub chord_range: FloatParam,
    #[id = "chordpat"]
    pub chord_pattern: FloatParam,
    #[id = "aatk"]
    pub amp_attack: FloatParam,
    #[id = "adec"]
    pub amp_decay: FloatParam,
    #[id = "asus"]
    pub amp_sustain: FloatParam,
    #[id = "arel"]
    pub amp_release: FloatParam,
}

impl Default for ZAudioGranularParams {
    fn default() -> Self {
        // Ranges and defaults match the WebCLAP build's `param_defs()`
        // (pinned by the mirror test below). No smoothers: grain spawn
        // granularity self-smooths, matching the repo's other instruments.
        let linear = |name: &str, unit: &'static str, min: f32, max: f32, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min, max }).with_unit(unit)
        };
        let stepped = |name: &str, unit: &'static str, min: f32, max: f32, default: f32| {
            linear(name, unit, min, max, default).with_step_size(1.0)
        };
        Self {
            level: linear("Level", "", 0.0, 2.0, 1.0),
            pitch: stepped("Pitch", " st", -48.0, 48.0, 0.0),
            fine: linear("Fine", " ct", -100.0, 100.0, 0.0),
            position: linear("Position", "", 0.0, 1.0, 0.0),
            grain_length: linear("Grain Length", " ms", 2.0, 1000.0, 100.0),
            length_keytrack: stepped("Length Keytrack", "", 0.0, 1.0, 0.0),
            grain_attack: linear("Grain Attack", "", 0.0, 1.0, 0.5),
            grain_decay: linear("Grain Decay", "", 0.0, 1.0, 0.5),
            attack_curve: linear("Attack Curve", "", -1.0, 1.0, 0.0),
            decay_curve: linear("Decay Curve", "", -1.0, 1.0, 0.0),
            spawn_mode: stepped("Spawn Mode", "", 0.0, 2.0, 0.0),
            rate: linear("Rate", " Hz", 0.1, 400.0, 25.0),
            sync_rate: stepped("Sync Rate", "", 0.0, 8.0, 4.0),
            density: linear("Density", "", 0.5, 64.0, 8.0),
            root_note: stepped("Root Note", "", 0.0, 127.0, 60.0),
            align_phases: stepped("Align Phases", "", 0.0, 1.0, 0.0),
            warm_start: stepped("Warm Start", "", 0.0, 1.0, 0.0),
            random_position: linear("Random Position", " ms", 0.0, 2000.0, 0.0),
            random_timing: linear("Random Timing", "", 0.0, 1.0, 0.0),
            random_pitch: linear("Random Pitch", " st", 0.0, 24.0, 0.0),
            random_level: linear("Random Level", "", 0.0, 1.0, 0.0),
            random_pan: linear("Random Pan", "", 0.0, 1.0, 0.0),
            random_reverse: linear("Random Reverse", "", 0.0, 1.0, 0.0),
            chord_type: stepped("Chord Type", "", 0.0, 9.0, 0.0),
            chord_range: stepped("Chord Range", " oct", 1.0, 4.0, 1.0),
            chord_pattern: stepped("Chord Pattern", "", 0.0, 3.0, 0.0),
            amp_attack: linear("Amp Attack", " s", 0.001, 5.0, 0.002),
            amp_decay: linear("Amp Decay", " s", 0.0, 5.0, 0.0),
            amp_sustain: linear("Amp Sustain", "", 0.0, 1.0, 1.0),
            amp_release: linear("Amp Release", " s", 0.01, 10.0, 0.25),
        }
    }
}

impl ZAudioGranularParams {
    /// The native params paired with their WebCLAP ids, in id order —
    /// drives both the editor mapping and the mirror test.
    pub fn web_id_pairs(&self) -> Vec<(u32, &FloatParam)> {
        use z_audio_webclap_granular::params::*;
        vec![
            (P_LEVEL, &self.level),
            (P_PITCH, &self.pitch),
            (P_FINE, &self.fine),
            (P_POSITION, &self.position),
            (P_GRAIN_LENGTH, &self.grain_length),
            (P_LENGTH_KEYTRACK, &self.length_keytrack),
            (P_GRAIN_ATTACK, &self.grain_attack),
            (P_GRAIN_DECAY, &self.grain_decay),
            (P_ATTACK_CURVE, &self.attack_curve),
            (P_DECAY_CURVE, &self.decay_curve),
            (P_SPAWN_MODE, &self.spawn_mode),
            (P_RATE, &self.rate),
            (P_SYNC_RATE, &self.sync_rate),
            (P_DENSITY, &self.density),
            (P_ROOT_NOTE, &self.root_note),
            (P_ALIGN_PHASES, &self.align_phases),
            (P_WARM_START, &self.warm_start),
            (P_RANDOM_POSITION, &self.random_position),
            (P_RANDOM_TIMING, &self.random_timing),
            (P_RANDOM_PITCH, &self.random_pitch),
            (P_RANDOM_LEVEL, &self.random_level),
            (P_RANDOM_PAN, &self.random_pan),
            (P_RANDOM_REVERSE, &self.random_reverse),
            (P_CHORD_TYPE, &self.chord_type),
            (P_CHORD_RANGE, &self.chord_range),
            (P_CHORD_PATTERN, &self.chord_pattern),
            (P_AMP_ATTACK, &self.amp_attack),
            (P_AMP_DECAY, &self.amp_decay),
            (P_AMP_SUSTAIN, &self.amp_sustain),
            (P_AMP_RELEASE, &self.amp_release),
        ]
    }
}

pub struct ZAudioGranular {
    params: Arc<ZAudioGranularParams>,
    engine: GranularEngine,
    shared: Arc<GranularShared>,
    notes: Vec<TimedNote>,
    previews: Vec<NotePreview>,
}

impl Default for ZAudioGranular {
    fn default() -> Self {
        let mut engine = GranularEngine::new(48_000.0);
        if let Some((source, root)) = z_audio_webclap_granular::dev_bank() {
            let mut p = *engine.params();
            p.root_note = root;
            engine.set_params(p);
            engine.set_source(source);
        }
        let shared = Arc::new(GranularShared::mirroring(&engine));
        Self {
            params: Arc::new(ZAudioGranularParams::default()),
            engine,
            shared,
            notes: Vec::with_capacity(128),
            previews: Vec::with_capacity(64),
        }
    }
}

impl ZAudioGranular {
    fn engine_params(&self) -> GranularParams {
        let p = &self.params;
        GranularParams {
            level: p.level.value(),
            pitch_semitones: p.pitch.value().round(),
            fine_cents: p.fine.value(),
            position: p.position.value(),
            grain_length_ms: p.grain_length.value(),
            length_keytrack: p.length_keytrack.value() >= 0.5,
            grain_attack: p.grain_attack.value(),
            grain_decay: p.grain_decay.value(),
            attack_curve: p.attack_curve.value(),
            decay_curve: p.decay_curve.value(),
            spawn_mode: p.spawn_mode.value().round().clamp(0.0, 2.0) as u8,
            rate_hz: p.rate.value(),
            sync_index: p.sync_rate.value().round().clamp(0.0, 8.0) as u8,
            density: p.density.value(),
            root_note: p.root_note.value().round().clamp(0.0, 127.0) as u8,
            align_phases: p.align_phases.value() >= 0.5,
            warm_start: p.warm_start.value() >= 0.5,
            random_position_ms: p.random_position.value(),
            random_timing: p.random_timing.value(),
            random_pitch: p.random_pitch.value(),
            random_level: p.random_level.value(),
            random_pan: p.random_pan.value(),
            random_reverse: p.random_reverse.value(),
            chord_type: p.chord_type.value().round().clamp(0.0, 9.0) as u8,
            chord_range: p.chord_range.value().round().clamp(1.0, 4.0) as u8,
            chord_pattern: p.chord_pattern.value().round().clamp(0.0, 3.0) as u8,
            amp_attack_s: p.amp_attack.value(),
            amp_decay_s: p.amp_decay.value(),
            amp_sustain: p.amp_sustain.value(),
            amp_release_s: p.amp_release.value(),
        }
    }

    fn trigger(&mut self, note: TimedNote) {
        if note.on {
            self.engine
                .note_on(note.key & 0x7f, note.velocity.clamp(0.0, 1.0));
        } else {
            self.engine.note_off(note.key & 0x7f);
        }
    }
}

impl Plugin for ZAudioGranular {
    const NAME: &'static str = "Z Audio Granular";
    const VENDOR: &'static str = "zukky";
    const URL: &'static str = "https://github.com/SuzukiDaishi/z-audio-dsp";
    const EMAIL: &'static str = "zukky.rikugame@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    // Windows/macOS open the WebCLAP UI in a wry webview; ZGRN packets
    // ride the bin envelope into the shared bridge.
    #[cfg(any(windows, target_os = "macos"))]
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        use z_audio_webview_editor::{create_webview_editor_with_messages, inline_ui_html, map};
        static HTML: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let html = HTML.get_or_init(|| {
            inline_ui_html(
                include_str!("../../z-audio-webclap-granular/ui/index.html"),
                include_str!("../../z-audio-webclap-granular/ui/styles.css"),
                // No extra JS module — the granular UI is one main.js.
                "",
                include_str!("../../z-audio-webclap-granular/ui/main.js"),
            )
        });
        let shared = self.shared.clone();
        let mappings = self
            .params
            .web_id_pairs()
            .into_iter()
            .map(|(id, param)| map(id, param.as_ptr()))
            .collect();
        create_webview_editor_with_messages(
            html.as_str(),
            (860, 780),
            mappings,
            Some(Arc::new(
                move |bytes: &[u8], reply: &mut dyn FnMut(&[u8])| {
                    shared.on_ui_message(bytes, reply)
                },
            )),
        )
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_granular_editor(self.params.clone(), self.shared.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine.set_sample_rate(buffer_config.sample_rate);
        let capacity = (buffer_config.max_buffer_size as usize).max(64);
        self.engine.reserve_block(capacity);
        if self.notes.capacity() < capacity {
            self.notes.reserve_exact(capacity - self.notes.capacity());
        }
        true
    }

    fn reset(&mut self) {
        // Drop voices, keep the loaded sample (same as WebCLAP).
        self.engine.reset_voices();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Editor-prepared updates: file loads are rare, and installing the
        // already-decoded PCM is one pointer move.
        match self.shared.take_update() {
            Some(GranularUpdate::Commit(source)) => self.engine.set_source(source),
            Some(GranularUpdate::Clear) => self.engine.clear(),
            None => {}
        }

        // No-op inside the engine unless a value actually changed.
        self.engine.set_params(self.engine_params());
        self.engine
            .set_tempo(context.transport().tempo.unwrap_or(120.0));

        // Editor keyboard previews trigger at block start; sample-accurate
        // placement only matters for host-sequenced notes below.
        let mut previews = std::mem::take(&mut self.previews);
        previews.clear();
        self.shared.drain_notes(&mut previews);
        for preview in &previews {
            self.trigger(TimedNote {
                at: 0,
                on: preview.on,
                key: preview.key,
                velocity: preview.velocity as f32 / 127.0,
            });
        }
        self.previews = previews;

        self.notes.clear();
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    timing,
                    note,
                    velocity,
                    ..
                } => self.notes.push(TimedNote {
                    at: timing as usize,
                    on: true,
                    key: note,
                    velocity,
                }),
                NoteEvent::NoteOff { timing, note, .. } => self.notes.push(TimedNote {
                    at: timing as usize,
                    on: false,
                    key: note,
                    velocity: 0.0,
                }),
                _ => {}
            }
        }

        let output = buffer.as_slice();
        if output.len() < 2 {
            return ProcessStatus::Normal;
        }
        let (left_channel, rest) = output.split_at_mut(1);
        let frames = left_channel[0].len();

        // Render in segments split at note-event offsets so triggers land
        // sample-accurately within the block (same as the WebCLAP build).
        let mut start = 0usize;
        let mut event_index = 0usize;
        while start < frames {
            let mut end = frames;
            while event_index < self.notes.len() {
                let note = self.notes[event_index];
                let at = note.at.min(frames);
                if at <= start {
                    self.trigger(note);
                    event_index += 1;
                    continue;
                }
                end = at;
                break;
            }
            if end > start {
                self.engine
                    .render(&mut left_channel[0][start..end], &mut rest[0][start..end]);
            }
            start = end;
        }
        while event_index < self.notes.len() {
            let note = self.notes[event_index];
            self.trigger(note);
            event_index += 1;
        }

        // Publish grain positions for the editor's activity display
        // (non-blocking; the UI polls them with OP_POLL_ACTIVITY).
        let mut positions = [0.0f32; z_audio_webclap_granular::protocol::MAX_ACTIVITY_GRAINS];
        let count = self.engine.grain_positions(&mut positions);
        self.shared.store_activity(&positions[..count]);

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioGranular {
    const CLAP_ID: &'static str = "dev.zaudio.granular";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Granular synth: load a file, automate the play position, grains bloom around it");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Granular,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioGranular {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioGranular01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(ZAudioGranular);
nih_export_vst3!(ZAudioGranular);

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the 1:1 id contract: every native param's range and default
    /// must equal its WebCLAP `ParamDef`, in the same order.
    #[test]
    fn params_mirror_the_webclap_surface() {
        let params = ZAudioGranularParams::default();
        let pairs = params.web_id_pairs();
        let defs = z_audio_webclap_granular::params::param_defs();
        assert_eq!(pairs.len(), defs.len());
        for ((web_id, param), def) in pairs.iter().zip(defs.iter()) {
            assert_eq!(*web_id, def.id, "pair order must match the def order");
            let min = param.preview_plain(0.0);
            let max = param.preview_plain(1.0);
            let default = param.default_plain_value();
            assert!(
                (min as f64 - def.min).abs() < 1.0e-6,
                "param {} min: native {min} vs web {}",
                def.id,
                def.min
            );
            assert!(
                (max as f64 - def.max).abs() < 1.0e-6,
                "param {} max: native {max} vs web {}",
                def.id,
                def.max
            );
            assert!(
                (default as f64 - def.default).abs() < 1.0e-6,
                "param {} default: native {default} vs web {}",
                def.id,
                def.default
            );
        }
    }

    /// End-to-end audio path as the native adapter drives it: a UI file
    /// load flows through the shared bridge into the engine, a note makes
    /// sound, and note-off decays to a freed voice.
    #[test]
    fn shared_bridge_upload_note_renders_audio_and_decays() {
        use z_audio_webclap_granular::protocol::{encode_begin, encode_chunk, encode_commit};

        let mut engine = GranularEngine::new(48_000.0);
        engine.reserve_block(512);
        let shared = GranularShared::mirroring(&engine);

        // "UI" uploads one second of a 220 Hz tone in protocol packets.
        let pcm: Vec<f32> = (0..48_000)
            .map(|i| (core::f32::consts::TAU * 220.0 * i as f32 / 48_000.0).sin())
            .collect();
        let mut no_reply = |_: &[u8]| {};
        shared.on_ui_message(&encode_begin(48_000.0, 1, 48_000), &mut no_reply);
        for (index, chunk) in pcm.chunks(4_096).enumerate() {
            shared.on_ui_message(&encode_chunk((index * 4_096) as u32, chunk), &mut no_reply);
        }
        shared.on_ui_message(&encode_commit(), &mut no_reply);

        // Audio thread applies the update (as `process` does each block).
        let Some(GranularUpdate::Commit(source)) = shared.take_update() else {
            panic!("expected the uploaded source");
        };
        engine.set_source(source);

        let mut p = *engine.params();
        p.position = 0.5;
        p.spawn_mode = 2; // density
        p.density = 8.0;
        // Decorrelate the grains: evenly spaced grains over a pure tone
        // can sum to silence (anti-phase pairs).
        p.random_position_ms = 300.0;
        p.warm_start = true;
        p.amp_release_s = 0.05;
        engine.set_params(p);

        engine.note_on(60, 1.0);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        let mut energy = 0.0f32;
        for _ in 0..20 {
            engine.render(&mut l, &mut r);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
            energy += l.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(energy > 0.1, "note should be audible, energy {energy}");

        engine.note_off(60);
        for _ in 0..200 {
            engine.render(&mut l, &mut r);
        }
        assert_eq!(engine.active_voice_count(), 0, "release frees the voice");
        engine.render(&mut l, &mut r);
        assert!(l.iter().chain(r.iter()).all(|v| *v == 0.0));
    }

    /// The webview editor inlines the granular UI (with an empty module
    /// slot) into one self-contained page; make sure that composition
    /// holds for the real files, on every platform.
    #[test]
    fn granular_ui_inlines_into_one_self_contained_page() {
        let html = z_audio_webview_editor::inline_ui_html(
            include_str!("../../z-audio-webclap-granular/ui/index.html"),
            include_str!("../../z-audio-webclap-granular/ui/styles.css"),
            "",
            include_str!("../../z-audio-webclap-granular/ui/main.js"),
        );
        // No module plumbing may survive the inlining.
        assert!(!html.contains("\nimport "));
        assert!(!html.contains("src=\"./main.js\""));
        assert!(!html.contains("href=\"./styles.css\""));
        // The native transport branch made it into the page.
        assert!(html.contains("window.sendToPlugin"));
    }
}
