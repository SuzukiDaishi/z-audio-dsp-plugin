use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{Effect, Limiter, LimiterParams, ParamId, ProcessContext as DspProcessContext};

mod editor;

#[derive(Params)]
pub struct ZAudioLimiterParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "limiter_input_gain"]
    input_gain: FloatParam,
    #[id = "limiter_threshold"]
    threshold: FloatParam,
    #[id = "limiter_ceiling"]
    ceiling: FloatParam,
    #[id = "limiter_release"]
    release: FloatParam,
    #[id = "limiter_lookahead"]
    lookahead: FloatParam,
    #[id = "limiter_stereo_link"]
    stereo_link: FloatParam,
    #[id = "limiter_true_peak"]
    true_peak: BoolParam,
    #[id = "limiter_output_gain"]
    output_gain: FloatParam,
}

impl Default for ZAudioLimiterParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(720, 430),
            input_gain: float_param(ParamId::LimiterInputGain, "Input Gain", " dB"),
            threshold: float_param(ParamId::LimiterThreshold, "Threshold", " dB"),
            ceiling: float_param(ParamId::LimiterCeiling, "Ceiling", " dB"),
            release: float_param(ParamId::LimiterRelease, "Release", " ms"),
            lookahead: float_param(ParamId::LimiterLookahead, "Lookahead", " ms"),
            stereo_link: float_param(ParamId::LimiterStereoLink, "Stereo Link", ""),
            true_peak: BoolParam::new("True Peak", false),
            output_gain: float_param(ParamId::LimiterOutputGain, "Output Gain", " dB"),
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

pub struct ZAudioLimiter {
    params: Arc<ZAudioLimiterParams>,
    meters: Arc<MeterState>,
    limiter: Limiter,
    sample_rate: f32,
    max_block_size: usize,
    mono_right: Vec<f32>,
}

impl Default for ZAudioLimiter {
    fn default() -> Self {
        let mut limiter = Limiter::default();
        limiter.prepare(48_000.0, 512);
        Self {
            params: Arc::new(ZAudioLimiterParams::default()),
            meters: Arc::new(MeterState::default()),
            limiter,
            sample_rate: 48_000.0,
            max_block_size: 512,
            mono_right: vec![0.0; 512],
        }
    }
}

impl ZAudioLimiter {
    fn sync_params(&mut self) {
        self.limiter.set_params(LimiterParams {
            input_gain_db: self.params.input_gain.value(),
            ceiling_db: self.params.ceiling.value(),
            threshold_db: self.params.threshold.value(),
            release_ms: self.params.release.value(),
            lookahead_ms: self.params.lookahead.value(),
            stereo_link: self.params.stereo_link.value(),
            true_peak: self.params.true_peak.value(),
            output_gain_db: self.params.output_gain.value(),
        });
    }
}

impl Plugin for ZAudioLimiter {
    const NAME: &'static str = "Z Audio Limiter";
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
        // Windows/macOS: reuse the WebCLAP UI inside a wry webview (see
        // crates/z-audio-webview-editor). Linux hosts can't embed a
        // webview in the plugin window, so they keep the egui editor.
        #[cfg(any(windows, target_os = "macos"))]
        {
            let p = &self.params;
            return z_audio_webview_editor::webview_editor_from_ui!(
                "../../z-audio-webclap-limiter/ui",
                (880, 560),
                vec![
                    z_audio_webview_editor::map(120, p.input_gain.as_ptr()),
                    z_audio_webview_editor::map(121, p.threshold.as_ptr()),
                    z_audio_webview_editor::map(122, p.ceiling.as_ptr()),
                    z_audio_webview_editor::map(123, p.release.as_ptr()),
                    z_audio_webview_editor::map(124, p.lookahead.as_ptr()),
                    z_audio_webview_editor::map(125, p.stereo_link.as_ptr()),
                    z_audio_webview_editor::map(126, p.true_peak.as_ptr()),
                    z_audio_webview_editor::map(127, p.output_gain.as_ptr())
                ]
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        editor::create_limiter_editor(self.params.clone(), self.meters.clone())
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
        self.limiter.prepare(self.sample_rate, self.max_block_size);
        self.sync_params();
        true
    }

    fn reset(&mut self) {
        self.limiter.reset();
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
                self.limiter.process_stereo(&ctx, channels[0], right);
            }
            _ => {
                let (left, rest) = channels.split_at_mut(1);
                self.limiter.process_stereo(&ctx, left[0], rest[0]);
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

impl ClapPlugin for ZAudioLimiter {
    const CLAP_ID: &'static str = "dev.zaudio.limiter";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lookahead peak limiter");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for ZAudioLimiter {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioLimiter000";
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

nih_export_clap!(ZAudioLimiter);
nih_export_vst3!(ZAudioLimiter);
