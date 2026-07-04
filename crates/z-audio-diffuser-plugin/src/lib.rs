use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{Diffuser, DiffuserParams, Effect, ParamId, ProcessContext as DspProcessContext};

mod editor;

#[derive(Params)]
pub struct ZAudioDiffuserParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "diffuser_mix"]
    mix: FloatParam,
    #[id = "diffuser_diffusion"]
    diffusion: FloatParam,
    #[id = "diffuser_allpass_count"]
    allpass_count: FloatParam,
    #[id = "diffuser_size"]
    size: FloatParam,
    #[id = "diffuser_width"]
    width: FloatParam,
    #[id = "diffuser_output_gain"]
    output_gain: FloatParam,
}

impl Default for ZAudioDiffuserParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(720, 430),
            mix: float_param(ParamId::DiffuserMix, "Mix", ""),
            diffusion: float_param(ParamId::DiffuserDiffusion, "Diffusion", ""),
            allpass_count: immediate_float_param(
                ParamId::DiffuserAllpassCount,
                "Allpass Count",
                "",
            ),
            size: float_param(ParamId::DiffuserSize, "Size", ""),
            width: float_param(ParamId::DiffuserWidth, "Width", ""),
            output_gain: float_param(ParamId::DiffuserOutputGain, "Output Gain", " dB"),
        }
    }
}

pub struct MeterState {
    input_peak_db: AtomicU32,
    output_peak_db: AtomicU32,
}

impl Default for MeterState {
    fn default() -> Self {
        Self {
            input_peak_db: AtomicU32::new((-90.0_f32).to_bits()),
            output_peak_db: AtomicU32::new((-90.0_f32).to_bits()),
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

    fn store(&self, input_peak_db: f32, output_peak_db: f32) {
        self.input_peak_db
            .store(input_peak_db.to_bits(), Ordering::Relaxed);
        self.output_peak_db
            .store(output_peak_db.to_bits(), Ordering::Relaxed);
    }
}

pub struct ZAudioDiffuser {
    params: Arc<ZAudioDiffuserParams>,
    meters: Arc<MeterState>,
    diffuser: Diffuser,
    sample_rate: f32,
    max_block_size: usize,
    mono_right: Vec<f32>,
}

impl Default for ZAudioDiffuser {
    fn default() -> Self {
        let mut diffuser = Diffuser::default();
        diffuser.prepare(48_000.0, 512);
        Self {
            params: Arc::new(ZAudioDiffuserParams::default()),
            meters: Arc::new(MeterState::default()),
            diffuser,
            sample_rate: 48_000.0,
            max_block_size: 512,
            mono_right: vec![0.0; 512],
        }
    }
}

impl ZAudioDiffuser {
    fn sync_params(&mut self) {
        self.diffuser.set_params(DiffuserParams {
            mix: self.params.mix.value(),
            diffusion: self.params.diffusion.value(),
            allpass_count: self.params.allpass_count.value(),
            size: self.params.size.value(),
            width: self.params.width.value(),
            output_gain_db: self.params.output_gain.value(),
        });
    }
}

impl Plugin for ZAudioDiffuser {
    const NAME: &'static str = "Z Audio Diffuser";
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
        editor::create_diffuser_editor(self.params.clone(), self.meters.clone())
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
        self.diffuser.prepare(self.sample_rate, self.max_block_size);
        self.sync_params();
        true
    }

    fn reset(&mut self) {
        self.diffuser.reset();
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
                self.diffuser.process_stereo(&ctx, channels[0], right);
            }
            _ => {
                let (left, rest) = channels.split_at_mut(1);
                self.diffuser.process_stereo(&ctx, left[0], rest[0]);
            }
        }
        let output_peak_db = peak_db(channels);
        self.meters.store(input_peak_db, output_peak_db);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioDiffuser {
    const CLAP_ID: &'static str = "dev.zaudio.diffuser";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Schroeder allpass diffuser");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for ZAudioDiffuser {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioDiffuser00";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Stereo];
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

fn immediate_float_param(id: ParamId, name: &'static str, unit: &'static str) -> FloatParam {
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

nih_export_clap!(ZAudioDiffuser);
nih_export_vst3!(ZAudioDiffuser);
