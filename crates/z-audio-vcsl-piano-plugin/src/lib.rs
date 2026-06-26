use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use nih_plug::prelude::*;
use z_audio_dsp::{EventKind, ParamId, TimedEvent};
use z_audio_synth::{VcslPiano, VcslPianoConfig, VcslSampleBank};

const MAX_POLYPHONY: usize = 32;
const PIANO_AUTOMATABLE_IDS: [ParamId; 6] = [
    ParamId::VcslMasterGain,
    ParamId::VcslTone,
    ParamId::VcslVelocityCurve,
    ParamId::VcslReleaseLevel,
    ParamId::VcslReleaseTime,
    ParamId::VcslStereoWidth,
];

/// The sample bank is read once per process (it can be hundreds of MB) and
/// shared by every plugin instance via `Arc`.
fn shared_bank() -> Option<Arc<VcslSampleBank>> {
    static BANK: OnceLock<Option<Arc<VcslSampleBank>>> = OnceLock::new();
    BANK.get_or_init(load_bank_from_disk).clone()
}

fn load_bank_from_disk() -> Option<Arc<VcslSampleBank>> {
    let path = locate_bank_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let bank = z_audio_synth::load_bank_bytes(&bytes).ok()?;
    Some(Arc::new(bank))
}

/// Looks for the VCSL piano bank produced by `cargo xtask prepare-vcsl-piano`.
/// Checks `Z_AUDIO_VCSL_PIANO_BANK` first, then a path relative to the
/// current working directory; if neither resolves, the plugin stays silent
/// instead of failing to load.
fn locate_bank_path() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("Z_AUDIO_VCSL_PIANO_BANK") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    let default = PathBuf::from("assets/vcsl-piano/grand-piano-k.bank");
    if default.is_file() {
        return Some(default);
    }
    None
}

#[derive(Params)]
pub struct ZAudioVcslPianoParams {
    #[id = "vcsl_master_gain"]
    master_gain: FloatParam,
    #[id = "vcsl_tone"]
    tone: FloatParam,
    #[id = "vcsl_velocity_curve"]
    velocity_curve: FloatParam,
    #[id = "vcsl_release_level"]
    release_level: FloatParam,
    #[id = "vcsl_release_time"]
    release_time: FloatParam,
    #[id = "vcsl_stereo_width"]
    stereo_width: FloatParam,
}

impl Default for ZAudioVcslPianoParams {
    fn default() -> Self {
        Self {
            master_gain: float_param(ParamId::VcslMasterGain, "Master Gain", " dB"),
            tone: float_param(ParamId::VcslTone, "Tone", ""),
            velocity_curve: float_param(ParamId::VcslVelocityCurve, "Velocity Curve", ""),
            release_level: float_param(ParamId::VcslReleaseLevel, "Release Level", " dB"),
            release_time: float_param(ParamId::VcslReleaseTime, "Release Time", " s"),
            stereo_width: float_param(ParamId::VcslStereoWidth, "Stereo Width", ""),
        }
    }
}

pub struct ZAudioVcslPiano {
    params: Arc<ZAudioVcslPianoParams>,
    piano: Option<VcslPiano>,
    note_events: Vec<TimedEvent>,
    events: Vec<TimedEvent>,
    last_values: [f32; PIANO_AUTOMATABLE_IDS.len()],
    sample_rate: f32,
    max_block_size: usize,
}

impl Default for ZAudioVcslPiano {
    fn default() -> Self {
        Self {
            params: Arc::new(ZAudioVcslPianoParams::default()),
            piano: None,
            note_events: Vec::with_capacity(64),
            events: Vec::with_capacity(128),
            last_values: [f32::NAN; PIANO_AUTOMATABLE_IDS.len()],
            sample_rate: 48_000.0,
            max_block_size: 512,
        }
    }
}

impl ZAudioVcslPiano {
    fn reinit(&mut self) {
        let mut piano = VcslPiano::new(VcslPianoConfig {
            sample_rate: self.sample_rate,
            max_block_size: self.max_block_size,
            max_polyphony: MAX_POLYPHONY,
        });
        if let Some(bank) = shared_bank() {
            piano.load_bank(bank);
        }
        for (i, id) in PIANO_AUTOMATABLE_IDS.iter().enumerate() {
            let value = current_param_value(&self.params, *id);
            piano.set_param(*id, value);
            self.last_values[i] = value;
        }
        self.piano = Some(piano);
    }
}

impl Plugin for ZAudioVcslPiano {
    const NAME: &'static str = "Z Audio VCSL Piano";
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

impl ClapPlugin for ZAudioVcslPiano {
    const CLAP_ID: &'static str = "dev.zaudio.vcsl-piano";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("VCSL Keys sampler piano instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioVcslPiano {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioVCSLPiano1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Piano,
        Vst3SubCategory::Stereo,
    ];
}

fn current_param_value(params: &ZAudioVcslPianoParams, id: ParamId) -> f32 {
    match id {
        ParamId::VcslMasterGain => params.master_gain.value(),
        ParamId::VcslTone => params.tone.value(),
        ParamId::VcslVelocityCurve => params.velocity_curve.value(),
        ParamId::VcslReleaseLevel => params.release_level.value(),
        ParamId::VcslReleaseTime => params.release_time.value(),
        ParamId::VcslStereoWidth => params.stereo_width.value(),
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

nih_export_clap!(ZAudioVcslPiano);
nih_export_vst3!(ZAudioVcslPiano);
