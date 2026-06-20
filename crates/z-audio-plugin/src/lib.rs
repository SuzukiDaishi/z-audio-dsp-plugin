use std::sync::Arc;

use nih_plug::prelude::*;
use z_audio_dsp::{EventKind, TimedEvent};

pub mod editor;
pub mod engine;
pub mod enums;
pub mod params;

use engine::SynthEngine;
use params::ZAudioSimpleSynthParams;

/// `z-audio-synth::SimpleSynth` exposed as a VST3/CLAP instrument via nih-plug.
pub struct ZAudioSimpleSynth {
    params: Arc<ZAudioSimpleSynthParams>,
    engine: SynthEngine,
    note_events: Vec<TimedEvent>,
    sample_rate: f32,
    max_block_size: usize,
}

impl Default for ZAudioSimpleSynth {
    fn default() -> Self {
        Self {
            params: Arc::new(ZAudioSimpleSynthParams::default()),
            engine: SynthEngine::new(),
            note_events: Vec::with_capacity(64),
            sample_rate: 48_000.0,
            max_block_size: 512,
        }
    }
}

impl Plugin for ZAudioSimpleSynth {
    const NAME: &'static str = "Z Audio Simple Synth";
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

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_synth_editor(self.params.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.max_block_size = buffer_config.max_buffer_size as usize;
        let wanted_event_capacity = self.max_block_size.max(64);
        if self.note_events.capacity() < wanted_event_capacity {
            self.note_events = Vec::with_capacity(wanted_event_capacity);
        }
        self.engine
            .reinit(self.sample_rate, self.max_block_size, &self.params);
        true
    }

    fn reset(&mut self) {
        self.engine
            .reinit(self.sample_rate, self.max_block_size, &self.params);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let tempo_bpm = context.transport().tempo.unwrap_or(120.0) as f32;

        self.note_events.clear();
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    timing,
                    note,
                    velocity,
                    ..
                } => {
                    if self.note_events.len() < self.note_events.capacity() {
                        self.note_events.push(TimedEvent {
                            sample_offset: timing as usize,
                            kind: EventKind::NoteOn { note, velocity },
                        });
                    }
                }
                NoteEvent::NoteOff {
                    timing,
                    note,
                    velocity,
                    ..
                } => {
                    if self.note_events.len() < self.note_events.capacity() {
                        self.note_events.push(TimedEvent {
                            sample_offset: timing as usize,
                            kind: EventKind::NoteOff { note, velocity },
                        });
                    }
                }
                _ => {}
            }
        }

        let output = buffer.as_slice();
        let (left_slice, right_slice) = output.split_at_mut(1);
        let left = &mut left_slice[0];
        let right = &mut right_slice[0];

        self.engine.process_block(
            &self.params,
            self.note_events.iter().copied(),
            self.sample_rate,
            tempo_bpm,
            left,
            right,
        );

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioSimpleSynth {
    const CLAP_ID: &'static str = "dev.zaudio.simple-synth";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A simple subtractive synthesizer built on z-audio-dsp / z-audio-synth");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioSimpleSynth {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioSmplSynth1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(ZAudioSimpleSynth);
nih_export_vst3!(ZAudioSimpleSynth);
