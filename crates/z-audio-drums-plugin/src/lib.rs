use std::sync::Arc;

use nih_plug::prelude::*;
use z_audio_dsp::{EventKind, ParamId, TimedEvent};
use z_audio_synth::{FormulaDrumKit, FormulaDrumKitConfig};

const MAX_POLYPHONY: usize = 64;
const DRUM_AUTOMATABLE_IDS: [ParamId; 12] = [
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

#[derive(Params)]
pub struct ZAudioDrumParams {
    #[id = "drum_kick_level"]
    kick_level: FloatParam,
    #[id = "drum_snare_level"]
    snare_level: FloatParam,
    #[id = "drum_tom_level"]
    tom_level: FloatParam,
    #[id = "drum_hat_level"]
    hat_level: FloatParam,
    #[id = "drum_cymbal_level"]
    cymbal_level: FloatParam,
    #[id = "drum_tuning"]
    tuning: FloatParam,
    #[id = "drum_decay"]
    decay: FloatParam,
    #[id = "drum_tone"]
    tone: FloatParam,
    #[id = "drum_snare_wire"]
    snare_wire: FloatParam,
    #[id = "drum_room_amount"]
    room_amount: FloatParam,
    #[id = "drum_stereo_width"]
    stereo_width: FloatParam,
    #[id = "drum_master_gain"]
    master_gain: FloatParam,
}

impl Default for ZAudioDrumParams {
    fn default() -> Self {
        Self {
            kick_level: float_param(ParamId::DrumKickLevel, "Kick", ""),
            snare_level: float_param(ParamId::DrumSnareLevel, "Snare", ""),
            tom_level: float_param(ParamId::DrumTomLevel, "Toms", ""),
            hat_level: float_param(ParamId::DrumHatLevel, "Hats", ""),
            cymbal_level: float_param(ParamId::DrumCymbalLevel, "Cymbals", ""),
            tuning: float_param(ParamId::DrumTuning, "Tuning", " st"),
            decay: float_param(ParamId::DrumDecay, "Decay", ""),
            tone: float_param(ParamId::DrumTone, "Tone", ""),
            snare_wire: float_param(ParamId::DrumSnareWire, "Snare Wire", ""),
            room_amount: float_param(ParamId::DrumRoomAmount, "Room", ""),
            stereo_width: float_param(ParamId::DrumStereoWidth, "Width", ""),
            master_gain: float_param(ParamId::DrumMasterGain, "Master Gain", " dB"),
        }
    }
}

pub struct ZAudioFormulaDrumSet {
    params: Arc<ZAudioDrumParams>,
    drums: Option<FormulaDrumKit>,
    note_events: Vec<TimedEvent>,
    events: Vec<TimedEvent>,
    last_values: [f32; DRUM_AUTOMATABLE_IDS.len()],
    sample_rate: f32,
    max_block_size: usize,
}

impl Default for ZAudioFormulaDrumSet {
    fn default() -> Self {
        Self {
            params: Arc::new(ZAudioDrumParams::default()),
            drums: None,
            note_events: Vec::with_capacity(96),
            events: Vec::with_capacity(128),
            last_values: [f32::NAN; DRUM_AUTOMATABLE_IDS.len()],
            sample_rate: 48_000.0,
            max_block_size: 512,
        }
    }
}

impl ZAudioFormulaDrumSet {
    fn reinit(&mut self) {
        let mut drums = FormulaDrumKit::new(FormulaDrumKitConfig {
            sample_rate: self.sample_rate,
            max_block_size: self.max_block_size,
            max_polyphony: MAX_POLYPHONY,
        });
        for (i, id) in DRUM_AUTOMATABLE_IDS.iter().enumerate() {
            let value = current_param_value(&self.params, *id);
            drums.set_param(*id, value);
            self.last_values[i] = value;
        }
        self.drums = Some(drums);
    }
}

impl Plugin for ZAudioFormulaDrumSet {
    const NAME: &'static str = "Z Audio Formula Drum Set";
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

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.max_block_size = buffer_config.max_buffer_size as usize;
        self.note_events = Vec::with_capacity(self.max_block_size.max(96));
        self.events = Vec::with_capacity(self.max_block_size.max(96) + DRUM_AUTOMATABLE_IDS.len());
        self.reinit();
        true
    }

    fn reset(&mut self) {
        self.reinit();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
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

        let Some(drums) = self.drums.as_mut() else {
            return ProcessStatus::Normal;
        };
        self.events.clear();
        for (i, id) in DRUM_AUTOMATABLE_IDS.iter().enumerate() {
            let value = current_param_value(&self.params, *id);
            if value != self.last_values[i] {
                self.last_values[i] = value;
                self.events.push(TimedEvent {
                    sample_offset: 0,
                    kind: EventKind::Param { id: *id, value },
                });
            }
        }
        self.events.extend(self.note_events.iter().copied());
        self.events.sort_by_key(|event| event.sample_offset);

        let output = buffer.as_slice();
        let (left_slice, right_slice) = output.split_at_mut(1);
        let ctx = z_audio_dsp::ProcessContext::new(
            self.sample_rate,
            left_slice[0].len(),
            context.transport().tempo.unwrap_or(120.0) as f32,
            &self.events,
        );
        drums.process_with_context(&ctx, left_slice[0], right_slice[0]);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioFormulaDrumSet {
    const CLAP_ID: &'static str = "dev.zaudio.formula-drums";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Modal formula drum set instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioFormulaDrumSet {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioDrumSet001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

fn current_param_value(params: &ZAudioDrumParams, id: ParamId) -> f32 {
    match id {
        ParamId::DrumKickLevel => params.kick_level.value(),
        ParamId::DrumSnareLevel => params.snare_level.value(),
        ParamId::DrumTomLevel => params.tom_level.value(),
        ParamId::DrumHatLevel => params.hat_level.value(),
        ParamId::DrumCymbalLevel => params.cymbal_level.value(),
        ParamId::DrumTuning => params.tuning.value(),
        ParamId::DrumDecay => params.decay.value(),
        ParamId::DrumTone => params.tone.value(),
        ParamId::DrumSnareWire => params.snare_wire.value(),
        ParamId::DrumRoomAmount => params.room_amount.value(),
        ParamId::DrumStereoWidth => params.stereo_width.value(),
        ParamId::DrumMasterGain => params.master_gain.value(),
        _ => id.metadata().default,
    }
}

fn float_param(id: ParamId, name: &'static str, unit: &'static str) -> FloatParam {
    let m = id.metadata();
    FloatParam::new(
        name,
        m.default,
        FloatRange::Linear {
            min: m.min,
            max: m.max,
        },
    )
    .with_unit(unit)
    .with_smoother(SmoothingStyle::Linear(10.0))
}

nih_export_clap!(ZAudioFormulaDrumSet);
nih_export_vst3!(ZAudioFormulaDrumSet);
