use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{EventKind, ParamId, TimedEvent};
use z_audio_synth::{GenericSampler, GenericSamplerConfig, SamplerBank};

mod decode;
mod editor;
mod state;

use state::{BankUpdate, LoadStatus};

const MAX_POLYPHONY: usize = 16;
const SAMPLER_AUTOMATABLE_IDS: [ParamId; 14] = [
    ParamId::SamplerMasterGain,
    ParamId::SamplerRootNote,
    ParamId::SamplerTune,
    ParamId::SamplerOffset,
    ParamId::SamplerVelocityCurve,
    ParamId::SamplerReleaseTime,
    ParamId::SamplerStereoWidth,
    ParamId::SamplerLoopMode,
    ParamId::SamplerLoopStart,
    ParamId::SamplerLoopEnd,
    ParamId::SamplerLoopXfade,
    ParamId::SamplerUnisonVoices,
    ParamId::SamplerUnisonDetune,
    ParamId::SamplerUnisonSpread,
];

/// The bundled demo bank is read once per process and shared by every
/// plugin instance via `Arc`; it's only used as a fallback until the user
/// loads their own sample (or restores a previously persisted one).
fn shared_demo_bank() -> Option<Arc<SamplerBank>> {
    static BANK: OnceLock<Option<Arc<SamplerBank>>> = OnceLock::new();
    BANK.get_or_init(load_demo_bank_from_disk).clone()
}

fn load_demo_bank_from_disk() -> Option<Arc<SamplerBank>> {
    let path = locate_demo_bank_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let bank = z_audio_synth::load_sampler_bank_bytes(&bytes).ok()?;
    Some(Arc::new(bank))
}

/// Looks for the demo sampler bank produced by `cargo xtask
/// prepare-sampler-bank`. Checks `Z_AUDIO_SAMPLER_BANK` first, then a path
/// relative to the current working directory; if neither resolves, the
/// plugin stays silent instead of failing to load.
fn locate_demo_bank_path() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("Z_AUDIO_SAMPLER_BANK") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    let default = PathBuf::from("assets/sampler/piano.bank");
    if default.is_file() {
        return Some(default);
    }
    None
}

/// Decodes the user-loaded sample at `path` (persisted plugin state) into a
/// fresh [`SamplerBank`]. Only called from `initialize`/`reset` (non-
/// realtime setup callbacks), never from `process`.
fn load_user_bank_from_path(path: &str) -> Result<Arc<SamplerBank>, String> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(format!("file not found: '{}'", path.display()));
    }
    let (sample_rate, channels, pcm) = decode::decode_audio_file(&path)?;
    Ok(Arc::new(SamplerBank {
        sample: z_audio_dsp::SampleBuffer::new(sample_rate, channels, pcm),
        default_root_note: 60,
    }))
}

#[derive(Params)]
pub struct ZAudioSamplerParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,
    /// Absolute path to the user-loaded sample, persisted so the same file
    /// is reloaded when the DAW restores this plugin's state. `None` until
    /// the user picks a file via the editor.
    #[persist = "sample-path"]
    sample_path: Arc<Mutex<Option<String>>>,

    #[id = "sampler_master_gain"]
    master_gain: FloatParam,
    #[id = "sampler_root_note"]
    root_note: FloatParam,
    #[id = "sampler_tune"]
    tune: FloatParam,
    #[id = "sampler_offset"]
    offset: FloatParam,
    #[id = "sampler_velocity_curve"]
    velocity_curve: FloatParam,
    #[id = "sampler_release_time"]
    release_time: FloatParam,
    #[id = "sampler_stereo_width"]
    stereo_width: FloatParam,
    /// `0`=Off, `1`=Infinite, `2`=Sustain, `3`=PingPong, `4`=Reverse (see
    /// [`z_audio_dsp::LoopMode`]).
    #[id = "sampler_loop_mode"]
    loop_mode: FloatParam,
    #[id = "sampler_loop_start"]
    loop_start: FloatParam,
    #[id = "sampler_loop_end"]
    loop_end: FloatParam,
    #[id = "sampler_loop_xfade"]
    loop_xfade: FloatParam,
    #[id = "sampler_unison_voices"]
    unison_voices: FloatParam,
    #[id = "sampler_unison_detune"]
    unison_detune: FloatParam,
    #[id = "sampler_unison_spread"]
    unison_spread: FloatParam,
}

impl Default for ZAudioSamplerParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(420, 520),
            sample_path: Arc::new(Mutex::new(None)),
            master_gain: float_param(ParamId::SamplerMasterGain, "Master Gain", " dB"),
            root_note: float_param(ParamId::SamplerRootNote, "Root Note", ""),
            tune: float_param(ParamId::SamplerTune, "Tune", " cents"),
            offset: float_param(ParamId::SamplerOffset, "Offset", ""),
            velocity_curve: float_param(ParamId::SamplerVelocityCurve, "Velocity Curve", ""),
            release_time: float_param(ParamId::SamplerReleaseTime, "Release Time", " s"),
            stereo_width: float_param(ParamId::SamplerStereoWidth, "Stereo Width", ""),
            loop_mode: float_param(ParamId::SamplerLoopMode, "Loop Mode", ""),
            loop_start: float_param(ParamId::SamplerLoopStart, "Loop Start", ""),
            loop_end: float_param(ParamId::SamplerLoopEnd, "Loop End", ""),
            loop_xfade: float_param(ParamId::SamplerLoopXfade, "Loop Crossfade", " s"),
            unison_voices: float_param(ParamId::SamplerUnisonVoices, "Unison Voices", ""),
            unison_detune: float_param(ParamId::SamplerUnisonDetune, "Unison Detune", " cents"),
            unison_spread: float_param(ParamId::SamplerUnisonSpread, "Unison Spread", ""),
        }
    }
}

pub struct ZAudioSampler {
    params: Arc<ZAudioSamplerParams>,
    sampler: Option<GenericSampler>,
    /// Bank swap requested by the editor (e.g. "Load Sample..."), applied
    /// at the top of the next `process` call via a non-blocking `try_lock`.
    pending_bank: Arc<Mutex<Option<BankUpdate>>>,
    /// Editor-facing load status; not persisted (recomputed by `reinit`).
    status: Arc<Mutex<LoadStatus>>,
    note_events: Vec<TimedEvent>,
    events: Vec<TimedEvent>,
    last_values: [f32; SAMPLER_AUTOMATABLE_IDS.len()],
    sample_rate: f32,
    max_block_size: usize,
}

impl Default for ZAudioSampler {
    fn default() -> Self {
        Self {
            params: Arc::new(ZAudioSamplerParams::default()),
            sampler: None,
            pending_bank: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(LoadStatus::Empty)),
            note_events: Vec::with_capacity(64),
            events: Vec::with_capacity(128),
            last_values: [f32::NAN; SAMPLER_AUTOMATABLE_IDS.len()],
            sample_rate: 48_000.0,
            max_block_size: 512,
        }
    }
}

impl ZAudioSampler {
    /// Resolves which bank to (re)load on init/reset, preferring (in
    /// order): the bank already running in memory (so a sample-rate change
    /// doesn't drop a user-loaded sample), the persisted sample path (DAW
    /// state restore), then the bundled demo bank.
    fn resolve_bank(&self) -> Option<Arc<SamplerBank>> {
        if let Some(bank) = self.sampler.as_ref().and_then(|s| s.bank()) {
            return Some(bank);
        }
        let path = self.params.sample_path.lock().unwrap().clone();
        if let Some(path) = path {
            match load_user_bank_from_path(&path) {
                Ok(bank) => {
                    let file_name = PathBuf::from(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or(path.clone());
                    *self.status.lock().unwrap() = LoadStatus::Loaded { file_name };
                    return Some(bank);
                }
                Err(message) => {
                    *self.status.lock().unwrap() = LoadStatus::Missing { path: message };
                    return shared_demo_bank();
                }
            }
        }
        shared_demo_bank()
    }

    fn reinit(&mut self) {
        let bank = self.resolve_bank();
        let mut sampler = GenericSampler::new(GenericSamplerConfig {
            sample_rate: self.sample_rate,
            max_block_size: self.max_block_size,
            max_polyphony: MAX_POLYPHONY,
        });
        if let Some(bank) = bank {
            sampler.load_bank(bank);
        }
        for (i, id) in SAMPLER_AUTOMATABLE_IDS.iter().enumerate() {
            let value = current_param_value(&self.params, *id);
            sampler.set_param(*id, value);
            self.last_values[i] = value;
        }
        self.sampler = Some(sampler);
    }

    /// Picks up a bank swap requested by the editor, if any, without
    /// blocking (a held lock just means the editor is mid-update; we'll
    /// catch it on the next block).
    fn apply_pending_bank(&mut self) {
        let Ok(mut pending) = self.pending_bank.try_lock() else {
            return;
        };
        let Some(update) = pending.take() else {
            return;
        };
        drop(pending);
        let Some(sampler) = self.sampler.as_mut() else {
            return;
        };
        match update {
            BankUpdate::Loaded(bank) => sampler.load_bank(bank),
            BankUpdate::Cleared => {
                if let Some(bank) = shared_demo_bank() {
                    sampler.load_bank(bank);
                }
            }
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

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_sampler_editor(
            self.params.clone(),
            self.pending_bank.clone(),
            self.status.clone(),
        )
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
        self.events =
            Vec::with_capacity(self.max_block_size.max(64) + SAMPLER_AUTOMATABLE_IDS.len());
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
        self.apply_pending_bank();

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

        let Some(sampler) = self.sampler.as_mut() else {
            return ProcessStatus::Normal;
        };
        self.events.clear();
        for (i, id) in SAMPLER_AUTOMATABLE_IDS.iter().enumerate() {
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
        sampler.process_with_context(&ctx, left_slice[0], right_slice[0]);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioSampler {
    const CLAP_ID: &'static str = "dev.zaudio.sampler";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("General-purpose single-sample sampler instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioSampler {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioSamplerGen";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Sampler,
        Vst3SubCategory::Stereo,
    ];
}

fn current_param_value(params: &ZAudioSamplerParams, id: ParamId) -> f32 {
    match id {
        ParamId::SamplerMasterGain => params.master_gain.value(),
        ParamId::SamplerRootNote => params.root_note.value(),
        ParamId::SamplerTune => params.tune.value(),
        ParamId::SamplerOffset => params.offset.value(),
        ParamId::SamplerVelocityCurve => params.velocity_curve.value(),
        ParamId::SamplerReleaseTime => params.release_time.value(),
        ParamId::SamplerStereoWidth => params.stereo_width.value(),
        ParamId::SamplerLoopMode => params.loop_mode.value(),
        ParamId::SamplerLoopStart => params.loop_start.value(),
        ParamId::SamplerLoopEnd => params.loop_end.value(),
        ParamId::SamplerLoopXfade => params.loop_xfade.value(),
        ParamId::SamplerUnisonVoices => params.unison_voices.value(),
        ParamId::SamplerUnisonDetune => params.unison_detune.value(),
        ParamId::SamplerUnisonSpread => params.unison_spread.value(),
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

nih_export_clap!(ZAudioSampler);
nih_export_vst3!(ZAudioSampler);
