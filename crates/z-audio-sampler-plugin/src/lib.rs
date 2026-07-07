//! Native VST3/CLAP build of Z Audio Sampler — the Logic-style multi-zone
//! sampler. The engine (`ZoneSampler`), the `ZSMP` UI protocol, and (on
//! Windows/macOS) the web UI itself are shared with the WebCLAP build in
//! `crates/z-audio-webclap-sampler`; this crate is the `nih_plug` adapter.
//!
//! Editor strategy:
//! - Windows/macOS: the WebCLAP `ui/` bundle in a wry webview. Params ride
//!   the shared JSON param sync; sample uploads / zone tables / keyboard
//!   previews ride the `{"type":"bin"}` envelope (see
//!   `crates/z-audio-webview-editor`) into [`shared::SamplerShared`].
//! - elsewhere: a reduced egui editor (file load as one Classic zone).
//!
//! Like the WebCLAP build, only parameters persist in host projects; the
//! sample PCM does not, so files must be reloaded after reopening a
//! project. An embedded piano preview bank plays until a file is loaded.

use std::sync::Arc;

use nih_plug::prelude::*;
use z_audio_webclap_sampler::engine::{classic_zone, GlobalParams, ZoneSampler};

#[cfg(not(any(windows, target_os = "macos")))]
mod decode;
#[cfg(not(any(windows, target_os = "macos")))]
mod editor;
mod shared;

use shared::{NotePreview, SamplerShared, SamplerUpdate};

/// One note event with its intra-block sample offset.
#[derive(Clone, Copy)]
struct TimedNote {
    at: usize,
    on: bool,
    key: u8,
    velocity: f32,
}

/// Mirrors the WebCLAP param surface (web ids 300-308) one-to-one; the
/// editor mapping in [`ZAudioSampler::editor`] pairs them up.
#[derive(Params)]
pub struct ZAudioSamplerParams {
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "transpose"]
    pub transpose: FloatParam,
    #[id = "velocity"]
    pub velocity: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
}

impl Default for ZAudioSamplerParams {
    fn default() -> Self {
        // Ranges and defaults match the WebCLAP build's `param_defs()`.
        // No smoothers: the engine bakes params into trigger regions and
        // rebuilds them only when a value actually changes.
        let linear = |name: &str, unit: &'static str, min: f32, max: f32, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min, max }).with_unit(unit)
        };
        Self {
            gain: linear("Master Gain", " dB", -48.0, 12.0, 0.0),
            attack: linear("Attack", " s", 0.001, 5.0, 0.002),
            decay: linear("Decay", " s", 0.0, 5.0, 0.0),
            sustain: linear("Sustain", "", 0.0, 1.0, 1.0),
            release: linear("Release", " s", 0.01, 10.0, 0.25),
            tune: linear("Tune", " ct", -100.0, 100.0, 0.0),
            transpose: linear("Transpose", " st", -24.0, 24.0, 0.0).with_step_size(1.0),
            velocity: linear("Velocity Sens", "", 0.0, 1.0, 1.0),
            width: linear("Stereo Width", "", 0.0, 1.0, 1.0),
        }
    }
}

pub struct ZAudioSampler {
    params: Arc<ZAudioSamplerParams>,
    sampler: ZoneSampler,
    shared: Arc<SamplerShared>,
    notes: Vec<TimedNote>,
    previews: Vec<NotePreview>,
}

impl Default for ZAudioSampler {
    fn default() -> Self {
        let mut sampler = ZoneSampler::new(48_000.0);
        if let Some((source, root)) = z_audio_webclap_sampler::dev_bank() {
            let frames = source.frames() as u32;
            sampler.set_source(source, vec![classic_zone(root, frames)]);
        }
        let shared = Arc::new(SamplerShared::mirroring(&sampler));
        Self {
            params: Arc::new(ZAudioSamplerParams::default()),
            sampler,
            shared,
            notes: Vec::with_capacity(128),
            previews: Vec::with_capacity(64),
        }
    }
}

impl ZAudioSampler {
    fn global_params(&self) -> GlobalParams {
        GlobalParams {
            master_gain_db: self.params.gain.value(),
            attack_s: self.params.attack.value(),
            decay_s: self.params.decay.value(),
            sustain: self.params.sustain.value(),
            release_s: self.params.release.value(),
            tune_cents: self.params.tune.value(),
            transpose_semitones: self.params.transpose.value().round(),
            velocity_amount: self.params.velocity.value(),
            stereo_width: self.params.width.value(),
        }
    }

    fn trigger(&mut self, note: TimedNote) {
        if note.on {
            self.sampler
                .note_on(note.key & 0x7f, note.velocity.clamp(0.0, 1.0));
        } else {
            self.sampler.note_off(note.key & 0x7f);
        }
    }
}

impl Plugin for ZAudioSampler {
    const NAME: &'static str = "Z Audio Sampler";
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

    // Windows/macOS open the WebCLAP UI in a wry webview; ZSMP packets
    // ride the bin envelope into the shared bridge.
    #[cfg(any(windows, target_os = "macos"))]
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        use z_audio_webview_editor::{create_webview_editor_with_messages, inline_ui_html, map};
        static HTML: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let html = HTML.get_or_init(|| {
            inline_ui_html(
                include_str!("../../z-audio-webclap-sampler/ui/index.html"),
                include_str!("../../z-audio-webclap-sampler/ui/styles.css"),
                // The third slot inlines `onsets.js` the same way `zui.js`
                // is inlined for the other plugins (export-stripped module
                // prepended to an import-stripped main.js).
                include_str!("../../z-audio-webclap-sampler/ui/onsets.js"),
                include_str!("../../z-audio-webclap-sampler/ui/main.js"),
            )
        });
        let shared = self.shared.clone();
        let p = &self.params;
        create_webview_editor_with_messages(
            html.as_str(),
            (880, 720),
            vec![
                map(300, p.gain.as_ptr()),
                map(301, p.attack.as_ptr()),
                map(302, p.decay.as_ptr()),
                map(303, p.sustain.as_ptr()),
                map(304, p.release.as_ptr()),
                map(305, p.tune.as_ptr()),
                map(306, p.transpose.as_ptr()),
                map(307, p.velocity.as_ptr()),
                map(308, p.width.as_ptr()),
            ],
            Some(Arc::new(
                move |bytes: &[u8], reply: &mut dyn FnMut(&[u8])| {
                    shared.on_ui_message(bytes, reply)
                },
            )),
        )
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_sampler_editor(self.params.clone(), self.shared.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sampler.set_sample_rate(buffer_config.sample_rate);
        let capacity = (buffer_config.max_buffer_size as usize).max(64);
        if self.notes.capacity() < capacity {
            self.notes.reserve_exact(capacity - self.notes.capacity());
        }
        true
    }

    fn reset(&mut self) {
        // Drop voices, keep the loaded sample and zones (same as WebCLAP).
        self.sampler.reset_voices();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Editor-prepared updates. `set_source`/`set_zones` re-cut zone PCM
        // on this thread; that allocation is rare (user file loads / marker
        // drags) and matches what the WebCLAP build does in its
        // single-threaded `on_ui_message`.
        match self.shared.take_update() {
            Some(SamplerUpdate::Commit {
                source: Some(source),
                zones,
            }) => self.sampler.set_source(source, zones),
            Some(SamplerUpdate::Commit {
                source: None,
                zones,
            }) => self.sampler.set_zones(zones),
            Some(SamplerUpdate::Clear) => self.sampler.clear(),
            None => {}
        }

        // No-op inside the engine unless a value actually changed.
        self.sampler.set_params(self.global_params());

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
                self.sampler
                    .render(&mut left_channel[0][start..end], &mut rest[0][start..end]);
            }
            start = end;
        }
        while event_index < self.notes.len() {
            let note = self.notes[event_index];
            self.trigger(note);
            event_index += 1;
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioSampler {
    const CLAP_ID: &'static str = "dev.zaudio.sampler";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Multi-zone sampler: load a file in the UI, auto-slice, map to keys");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioSampler {
    // Fresh class id: the multi-zone rewrite shares nothing (params, state)
    // with the old never-shipped single-sample plugin (`ZAudioSamplerGen`).
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioSamplerMZ1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Sampler,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(ZAudioSampler);
nih_export_vst3!(ZAudioSampler);

#[cfg(test)]
mod tests {
    /// The webview editor inlines the sampler UI (with `onsets.js` in the
    /// module slot) into one self-contained page; make sure that
    /// composition holds for the real files, on every platform.
    #[test]
    fn sampler_ui_inlines_into_one_self_contained_page() {
        let html = z_audio_webview_editor::inline_ui_html(
            include_str!("../../z-audio-webclap-sampler/ui/index.html"),
            include_str!("../../z-audio-webclap-sampler/ui/styles.css"),
            include_str!("../../z-audio-webclap-sampler/ui/onsets.js"),
            include_str!("../../z-audio-webclap-sampler/ui/main.js"),
        );
        // No module plumbing may survive the inlining.
        assert!(!html.contains("\nimport "));
        assert!(!html.contains("\nexport "));
        assert!(!html.contains("src=\"./main.js\""));
        assert!(!html.contains("href=\"./styles.css\""));
        // onsets.js and the native transport branch made it into the page.
        assert!(html.contains("function computeOnsetCurve"));
        assert!(html.contains("window.sendToPlugin"));
    }
}
