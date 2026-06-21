use std::sync::Arc;

use nih_plug::prelude::*;
use z_audio_dsp::{EventKind, ParamId, TimedEvent};
use z_audio_synth::{FormulaPiano, FormulaPianoConfig};

const MAX_POLYPHONY: usize = 32;
const PIANO_AUTOMATABLE_IDS: [ParamId; 12] = [
    ParamId::PianoTone,
    ParamId::PianoBrightness,
    ParamId::PianoHammerHardness,
    ParamId::PianoHammerNoise,
    ParamId::PianoInharmonicity,
    ParamId::PianoDecay,
    ParamId::PianoRelease,
    ParamId::PianoBodyAmount,
    ParamId::PianoStereoWidth,
    ParamId::PianoSympatheticAmount,
    ParamId::PianoPedalResonance,
    ParamId::PianoMasterGain,
];

#[derive(Params)]
pub struct ZAudioPianoParams {
    #[id = "piano_tone"]
    tone: FloatParam,
    #[id = "piano_brightness"]
    brightness: FloatParam,
    #[id = "piano_hammer_hardness"]
    hammer_hardness: FloatParam,
    #[id = "piano_hammer_noise"]
    hammer_noise: FloatParam,
    #[id = "piano_inharmonicity"]
    inharmonicity: FloatParam,
    #[id = "piano_decay"]
    decay: FloatParam,
    #[id = "piano_release"]
    release: FloatParam,
    #[id = "piano_body_amount"]
    body_amount: FloatParam,
    #[id = "piano_stereo_width"]
    stereo_width: FloatParam,
    #[id = "piano_sympathetic_amount"]
    sympathetic_amount: FloatParam,
    #[id = "piano_pedal_resonance"]
    pedal_resonance: FloatParam,
    #[id = "piano_master_gain"]
    master_gain: FloatParam,
}

impl Default for ZAudioPianoParams {
    fn default() -> Self {
        Self {
            tone: float_param(ParamId::PianoTone, "Tone", ""),
            brightness: float_param(ParamId::PianoBrightness, "Brightness", ""),
            hammer_hardness: float_param(ParamId::PianoHammerHardness, "Hammer", ""),
            hammer_noise: float_param(ParamId::PianoHammerNoise, "Hammer Noise", ""),
            inharmonicity: float_param(ParamId::PianoInharmonicity, "Inharmonicity", ""),
            decay: float_param(ParamId::PianoDecay, "Decay", " s"),
            release: float_param(ParamId::PianoRelease, "Release", " s"),
            body_amount: float_param(ParamId::PianoBodyAmount, "Body", ""),
            stereo_width: float_param(ParamId::PianoStereoWidth, "Width", ""),
            sympathetic_amount: float_param(ParamId::PianoSympatheticAmount, "Sympathetic", ""),
            pedal_resonance: float_param(ParamId::PianoPedalResonance, "Pedal", ""),
            master_gain: float_param(ParamId::PianoMasterGain, "Master Gain", " dB"),
        }
    }
}

pub struct ZAudioFormulaPiano {
    params: Arc<ZAudioPianoParams>,
    piano: Option<FormulaPiano>,
    note_events: Vec<TimedEvent>,
    events: Vec<TimedEvent>,
    last_values: [f32; PIANO_AUTOMATABLE_IDS.len()],
    sample_rate: f32,
    max_block_size: usize,
}

impl Default for ZAudioFormulaPiano {
    fn default() -> Self {
        Self {
            params: Arc::new(ZAudioPianoParams::default()),
            piano: None,
            note_events: Vec::with_capacity(64),
            events: Vec::with_capacity(128),
            last_values: [f32::NAN; PIANO_AUTOMATABLE_IDS.len()],
            sample_rate: 48_000.0,
            max_block_size: 512,
        }
    }
}

impl ZAudioFormulaPiano {
    fn reinit(&mut self) {
        let mut piano = FormulaPiano::new(FormulaPianoConfig {
            sample_rate: self.sample_rate,
            max_block_size: self.max_block_size,
            max_polyphony: MAX_POLYPHONY,
        });
        for (i, id) in PIANO_AUTOMATABLE_IDS.iter().enumerate() {
            let value = current_param_value(&self.params, *id);
            piano.set_param(*id, value);
            self.last_values[i] = value;
        }
        self.piano = Some(piano);
    }
}

impl Plugin for ZAudioFormulaPiano {
    const NAME: &'static str = "Z Audio Formula Piano";
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
        self.note_events = Vec::with_capacity(self.max_block_size.max(64));
        self.events = Vec::with_capacity(self.max_block_size.max(64) + PIANO_AUTOMATABLE_IDS.len());
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

        let Some(piano) = self.piano.as_mut() else {
            return ProcessStatus::Normal;
        };
        self.events.clear();
        for (i, id) in PIANO_AUTOMATABLE_IDS.iter().enumerate() {
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
        piano.process_with_context(&ctx, left_slice[0], right_slice[0]);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioFormulaPiano {
    const CLAP_ID: &'static str = "dev.zaudio.formula-piano";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Modal formula piano instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioFormulaPiano {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioFormulaPno";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Piano,
        Vst3SubCategory::Stereo,
    ];
}

fn current_param_value(params: &ZAudioPianoParams, id: ParamId) -> f32 {
    match id {
        ParamId::PianoTone => params.tone.value(),
        ParamId::PianoBrightness => params.brightness.value(),
        ParamId::PianoHammerHardness => params.hammer_hardness.value(),
        ParamId::PianoHammerNoise => params.hammer_noise.value(),
        ParamId::PianoInharmonicity => params.inharmonicity.value(),
        ParamId::PianoDecay => params.decay.value(),
        ParamId::PianoRelease => params.release.value(),
        ParamId::PianoBodyAmount => params.body_amount.value(),
        ParamId::PianoStereoWidth => params.stereo_width.value(),
        ParamId::PianoSympatheticAmount => params.sympathetic_amount.value(),
        ParamId::PianoPedalResonance => params.pedal_resonance.value(),
        ParamId::PianoMasterGain => params.master_gain.value(),
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

nih_export_clap!(ZAudioFormulaPiano);
nih_export_vst3!(ZAudioFormulaPiano);
