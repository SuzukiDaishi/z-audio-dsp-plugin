use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{
    Compressor, CompressorParams, DetectorMode, Effect, ParamId,
    ProcessContext as DspProcessContext,
};

mod editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DetectorModeParam {
    Peak,
    Rms,
}

impl From<DetectorModeParam> for DetectorMode {
    fn from(value: DetectorModeParam) -> Self {
        match value {
            DetectorModeParam::Peak => DetectorMode::Peak,
            DetectorModeParam::Rms => DetectorMode::Rms,
        }
    }
}

#[derive(Params)]
pub struct ZAudioCompressorParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "compressor_input_gain"]
    input_gain: FloatParam,
    #[id = "compressor_threshold"]
    threshold: FloatParam,
    #[id = "compressor_ratio"]
    ratio: FloatParam,
    #[id = "compressor_knee"]
    knee: FloatParam,
    #[id = "compressor_attack"]
    attack: FloatParam,
    #[id = "compressor_release"]
    release: FloatParam,
    #[id = "compressor_makeup_gain"]
    makeup_gain: FloatParam,
    #[id = "compressor_mix"]
    mix: FloatParam,
    #[id = "compressor_detector_mode"]
    detector: EnumParam<DetectorModeParam>,
    #[id = "compressor_stereo_link"]
    stereo_link: FloatParam,
}

impl Default for ZAudioCompressorParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(760, 480),
            input_gain: float_param(ParamId::CompressorInputGain, "Input Gain", " dB"),
            threshold: float_param(ParamId::CompressorThreshold, "Threshold", " dB"),
            ratio: float_param(ParamId::CompressorRatio, "Ratio", ":1"),
            knee: float_param(ParamId::CompressorKnee, "Knee", " dB"),
            attack: float_param(ParamId::CompressorAttack, "Attack", " ms"),
            release: float_param(ParamId::CompressorRelease, "Release", " ms"),
            makeup_gain: float_param(ParamId::CompressorMakeupGain, "Makeup", " dB"),
            mix: float_param(ParamId::CompressorMix, "Mix", ""),
            detector: EnumParam::new("Detector", DetectorModeParam::Peak),
            stereo_link: float_param(ParamId::CompressorStereoLink, "Stereo Link", ""),
        }
    }
}

pub struct MeterState {
    input_peak_db: AtomicU32,
    output_peak_db: AtomicU32,
    gain_reduction_db: AtomicU32,
}

impl Default for MeterState {
    fn default() -> Self {
        Self {
            input_peak_db: AtomicU32::new((-90.0_f32).to_bits()),
            output_peak_db: AtomicU32::new((-90.0_f32).to_bits()),
            gain_reduction_db: AtomicU32::new(0.0_f32.to_bits()),
        }
    }
}

impl MeterState {
    pub fn input_peak_db(&self) -> f32 {
        f32::from_bits(self.input_peak_db.load(Ordering::Relaxed))
    }

    pub fn output_peak_db(&self) -> f32 {
        f32::from_bits(self.output_peak_db.load(Ordering::Relaxed))
    }

    pub fn gain_reduction_db(&self) -> f32 {
        f32::from_bits(self.gain_reduction_db.load(Ordering::Relaxed))
    }

    fn store(&self, input_peak_db: f32, output_peak_db: f32, gain_reduction_db: f32) {
        self.input_peak_db
            .store(input_peak_db.to_bits(), Ordering::Relaxed);
        self.output_peak_db
            .store(output_peak_db.to_bits(), Ordering::Relaxed);
        self.gain_reduction_db
            .store(gain_reduction_db.to_bits(), Ordering::Relaxed);
    }
}

pub struct ZAudioCompressor {
    params: Arc<ZAudioCompressorParams>,
    meters: Arc<MeterState>,
    compressor: Compressor,
    sample_rate: f32,
    max_block_size: usize,
    mono_right: Vec<f32>,
}

impl Default for ZAudioCompressor {
    fn default() -> Self {
        let mut compressor = Compressor::default();
        compressor.prepare(48_000.0, 512);
        Self {
            params: Arc::new(ZAudioCompressorParams::default()),
            meters: Arc::new(MeterState::default()),
            compressor,
            sample_rate: 48_000.0,
            max_block_size: 512,
            mono_right: vec![0.0; 512],
        }
    }
}

impl ZAudioCompressor {
    fn sync_params(&mut self) {
        self.compressor.set_params(CompressorParams {
            input_gain_db: self.params.input_gain.value(),
            threshold_db: self.params.threshold.value(),
            ratio: self.params.ratio.value(),
            knee_db: self.params.knee.value(),
            attack_ms: self.params.attack.value(),
            release_ms: self.params.release.value(),
            makeup_gain_db: self.params.makeup_gain.value(),
            mix: self.params.mix.value(),
            detector_mode: self.params.detector.value().into(),
            stereo_link: self.params.stereo_link.value(),
        });
    }
}

impl Plugin for ZAudioCompressor {
    const NAME: &'static str = "Z Audio Compressor";
    const VENDOR: &'static str = "zukky";
    const URL: &'static str = "https://github.com/SuzukiDaishi/z-audio-dsp";
    const EMAIL: &'static str = "zukky.rikugame@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_compressor_editor(self.params.clone(), self.meters.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.max_block_size = buffer_config.max_buffer_size as usize;
        if self.mono_right.len() < self.max_block_size {
            self.mono_right.resize(self.max_block_size, 0.0);
        }
        self.compressor
            .prepare(self.sample_rate, self.max_block_size);
        self.sync_params();
        true
    }

    fn reset(&mut self) {
        self.compressor.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.sync_params();
        let channels = buffer.as_slice();
        let frames = channels.first().map_or(0, |channel| channel.len());
        let input_peak_db = peak_db(channels);
        let ctx = DspProcessContext::new(self.sample_rate, frames, 120.0, &[]);
        match channels.len() {
            0 => {}
            1 => {
                let right = &mut self.mono_right[..frames];
                right.copy_from_slice(channels[0]);
                self.compressor.process_stereo(&ctx, channels[0], right);
            }
            _ => {
                let (left, rest) = channels.split_at_mut(1);
                self.compressor.process_stereo(&ctx, left[0], rest[0]);
            }
        }
        let output_peak_db = peak_db(channels);
        self.meters.store(
            input_peak_db,
            output_peak_db,
            (input_peak_db - output_peak_db).max(0.0).min(36.0),
        );
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioCompressor {
    const CLAP_ID: &'static str = "dev.zaudio.compressor";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Feed-forward compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for ZAudioCompressor {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioCompressor";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
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

fn peak_db(channels: &[&mut [f32]]) -> f32 {
    let peak = channels
        .iter()
        .flat_map(|channel| channel.iter())
        .fold(0.0_f32, |peak, sample| peak.max((*sample).abs()));
    if peak <= 1.0e-9 {
        -90.0
    } else {
        (20.0 * peak.log10()).clamp(-90.0, 24.0)
    }
}

nih_export_clap!(ZAudioCompressor);
nih_export_vst3!(ZAudioCompressor);
