use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{
    Effect, ParamId, ParametricReverb, ParametricReverbParams, ProcessContext as DspProcessContext,
};

mod editor;

#[derive(Params)]
pub struct ZAudioReverbParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "reverb_mix"]
    mix: FloatParam,
    #[id = "reverb_room_size"]
    room_size: FloatParam,
    #[id = "reverb_decay"]
    decay: FloatParam,
    #[id = "reverb_pre_delay"]
    pre_delay: FloatParam,
    #[id = "reverb_diffusion"]
    diffusion: FloatParam,
    #[id = "reverb_damping"]
    damping: FloatParam,
    #[id = "reverb_low_cut"]
    low_cut: FloatParam,
    #[id = "reverb_high_cut"]
    high_cut: FloatParam,
    #[id = "reverb_width"]
    width: FloatParam,
    #[id = "reverb_early_late_mix"]
    early_late: FloatParam,
    #[id = "reverb_output_gain"]
    output_gain: FloatParam,
}

impl Default for ZAudioReverbParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(820, 500),
            mix: float_param(ParamId::ReverbMix, "Mix", ""),
            room_size: float_param(ParamId::ReverbRoomSize, "Room Size", ""),
            decay: float_param(ParamId::ReverbDecay, "Decay", " s"),
            pre_delay: float_param(ParamId::ReverbPreDelay, "Pre Delay", " ms"),
            diffusion: float_param(ParamId::ReverbDiffusion, "Diffusion", ""),
            damping: float_param(ParamId::ReverbDamping, "Damping", ""),
            low_cut: float_param(ParamId::ReverbLowCut, "Low Cut", " Hz"),
            high_cut: float_param(ParamId::ReverbHighCut, "High Cut", " Hz"),
            width: float_param(ParamId::ReverbWidth, "Width", ""),
            early_late: float_param(ParamId::ReverbEarlyLateMix, "Early/Late", ""),
            output_gain: float_param(ParamId::ReverbOutputGain, "Output Gain", " dB"),
        }
    }
}

pub struct MeterState {
    input_peak_db: AtomicU32,
    output_peak_db: AtomicU32,
    tail_level_db: AtomicU32,
}

impl Default for MeterState {
    fn default() -> Self {
        Self {
            input_peak_db: AtomicU32::new((-90.0_f32).to_bits()),
            output_peak_db: AtomicU32::new((-90.0_f32).to_bits()),
            tail_level_db: AtomicU32::new((-90.0_f32).to_bits()),
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

    pub fn tail_level_db(&self) -> f32 {
        f32::from_bits(self.tail_level_db.load(Ordering::Relaxed))
    }

    fn store(&self, input_peak_db: f32, output_peak_db: f32, tail_level_db: f32) {
        self.input_peak_db
            .store(input_peak_db.to_bits(), Ordering::Relaxed);
        self.output_peak_db
            .store(output_peak_db.to_bits(), Ordering::Relaxed);
        self.tail_level_db
            .store(tail_level_db.to_bits(), Ordering::Relaxed);
    }
}

pub struct ZAudioReverb {
    params: Arc<ZAudioReverbParams>,
    meters: Arc<MeterState>,
    reverb: ParametricReverb,
    sample_rate: f32,
    max_block_size: usize,
    mono_right: Vec<f32>,
}

impl Default for ZAudioReverb {
    fn default() -> Self {
        let mut reverb = ParametricReverb::default();
        reverb.prepare(48_000.0, 512);
        Self {
            params: Arc::new(ZAudioReverbParams::default()),
            meters: Arc::new(MeterState::default()),
            reverb,
            sample_rate: 48_000.0,
            max_block_size: 512,
            mono_right: vec![0.0; 512],
        }
    }
}

impl ZAudioReverb {
    fn sync_params(&mut self) {
        self.reverb.set_params(ParametricReverbParams {
            mix: self.params.mix.value(),
            room_size: self.params.room_size.value(),
            decay_time_sec: self.params.decay.value(),
            pre_delay_ms: self.params.pre_delay.value(),
            diffusion: self.params.diffusion.value(),
            damping: self.params.damping.value(),
            low_cut_hz: self.params.low_cut.value(),
            high_cut_hz: self.params.high_cut.value(),
            modulation_rate_hz: ParamId::ReverbModRate.metadata().default,
            modulation_depth: ParamId::ReverbModDepth.metadata().default,
            width: self.params.width.value(),
            early_late_mix: self.params.early_late.value(),
            output_gain_db: self.params.output_gain.value(),
        });
    }
}

impl Plugin for ZAudioReverb {
    const NAME: &'static str = "Z Audio Parametric Reverb";
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
                "../../z-audio-webclap-reverb/ui",
                (980, 650),
                vec![
                    z_audio_webview_editor::map(100, p.mix.as_ptr()),
                    z_audio_webview_editor::map(101, p.room_size.as_ptr()),
                    z_audio_webview_editor::map(102, p.decay.as_ptr()),
                    z_audio_webview_editor::map(103, p.pre_delay.as_ptr()),
                    z_audio_webview_editor::map(104, p.diffusion.as_ptr()),
                    z_audio_webview_editor::map(105, p.damping.as_ptr()),
                    z_audio_webview_editor::map(106, p.low_cut.as_ptr()),
                    z_audio_webview_editor::map(107, p.high_cut.as_ptr()),
                    z_audio_webview_editor::map(110, p.width.as_ptr()),
                    z_audio_webview_editor::map(111, p.early_late.as_ptr()),
                    z_audio_webview_editor::map(112, p.output_gain.as_ptr())
                ]
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        editor::create_reverb_editor(self.params.clone(), self.meters.clone())
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
        self.reverb.prepare(self.sample_rate, self.max_block_size);
        self.sync_params();
        true
    }

    fn reset(&mut self) {
        self.reverb.reset();
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
                self.reverb.process_stereo(&ctx, channels[0], right);
            }
            _ => {
                let (left, rest) = channels.split_at_mut(1);
                self.reverb.process_stereo(&ctx, left[0], rest[0]);
            }
        }
        let output_peak_db = peak_db(channels);
        let tail_level_db = (output_peak_db - input_peak_db).clamp(-90.0, 24.0);
        self.meters
            .store(input_peak_db, output_peak_db, tail_level_db);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioReverb {
    const CLAP_ID: &'static str = "dev.zaudio.parametric-reverb";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("FDN parametric reverb");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Reverb,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for ZAudioReverb {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioParaReverb";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Reverb];
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

nih_export_clap!(ZAudioReverb);
nih_export_vst3!(ZAudioReverb);
