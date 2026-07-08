use std::sync::Arc;

use nih_plug::prelude::*;
use z_audio_dsp::{
    ButterworthKind, Effect, ProcessContext as DspProcessContext, ThreeBandButterworthEq,
};

pub mod editor;
pub mod enums;
pub mod params;

use params::ZAudioSimpleEqParams;

pub struct ZAudioSimpleEq {
    params: Arc<ZAudioSimpleEqParams>,
    eq: ThreeBandButterworthEq,
    sample_rate: f32,
    max_block_size: usize,
    mono_right: Vec<f32>,
}

impl Default for ZAudioSimpleEq {
    fn default() -> Self {
        let mut eq = ThreeBandButterworthEq::new();
        eq.prepare(48_000.0, 512);
        Self {
            params: Arc::new(ZAudioSimpleEqParams::default()),
            eq,
            sample_rate: 48_000.0,
            max_block_size: 512,
            mono_right: vec![0.0; 512],
        }
    }
}

impl ZAudioSimpleEq {
    fn sync_params(&mut self) {
        self.eq.low.enabled = self.params.low.enabled.value();
        self.eq.low.frequency_hz = self.params.low.freq.value();
        self.eq.low.kind = ButterworthKind::from(self.params.low.kind.value());
        self.eq.low.gain_db = self.params.low.gain_db.value();
        self.eq.low.q = self.params.low.q.value();

        self.eq.mid.enabled = self.params.mid.enabled.value();
        self.eq.mid.frequency_hz = self.params.mid.freq.value();
        self.eq.mid.kind = ButterworthKind::from(self.params.mid.kind.value());
        self.eq.mid.gain_db = self.params.mid.gain_db.value();
        self.eq.mid.q = self.params.mid.q.value();

        self.eq.high.enabled = self.params.high.enabled.value();
        self.eq.high.frequency_hz = self.params.high.freq.value();
        self.eq.high.kind = ButterworthKind::from(self.params.high.kind.value());
        self.eq.high.gain_db = self.params.high.gain_db.value();
        self.eq.high.q = self.params.high.q.value();
    }
}

impl Plugin for ZAudioSimpleEq {
    const NAME: &'static str = "Z Audio Simple EQ";
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
        // Windows/macOS: a wry webview editor (see
        // crates/z-audio-webview-editor). Linux hosts can't embed a
        // webview in the plugin window, so they keep the egui editor.
        // The UI is this crate's own snapshot of the original 3-band
        // WebCLAP EQ page — the live webclap-eq UI has since grown into
        // the 8-band pro EQ with a different parameter surface.
        #[cfg(any(windows, target_os = "macos"))]
        {
            let p = &self.params;
            return z_audio_webview_editor::webview_editor_from_ui!(
                "../ui",
                (960, 540),
                vec![
                    z_audio_webview_editor::map(40, p.low.enabled.as_ptr()),
                    z_audio_webview_editor::map(41, p.low.freq.as_ptr()),
                    z_audio_webview_editor::map(42, p.low.kind.as_ptr()),
                    z_audio_webview_editor::map(49, p.low.gain_db.as_ptr()),
                    z_audio_webview_editor::map(50, p.low.q.as_ptr()),
                    z_audio_webview_editor::map(43, p.mid.enabled.as_ptr()),
                    z_audio_webview_editor::map(44, p.mid.freq.as_ptr()),
                    z_audio_webview_editor::map(45, p.mid.kind.as_ptr()),
                    z_audio_webview_editor::map(51, p.mid.gain_db.as_ptr()),
                    z_audio_webview_editor::map(52, p.mid.q.as_ptr()),
                    z_audio_webview_editor::map(46, p.high.enabled.as_ptr()),
                    z_audio_webview_editor::map(47, p.high.freq.as_ptr()),
                    z_audio_webview_editor::map(48, p.high.kind.as_ptr()),
                    z_audio_webview_editor::map(53, p.high.gain_db.as_ptr()),
                    z_audio_webview_editor::map(54, p.high.q.as_ptr())
                ]
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        editor::create_eq_editor(self.params.clone())
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
        self.eq.prepare(self.sample_rate, self.max_block_size);
        self.sync_params();
        true
    }

    fn reset(&mut self) {
        self.eq.reset();
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
        if frames == 0 {
            return ProcessStatus::Normal;
        }

        let events = [];
        let ctx = DspProcessContext::new(self.sample_rate, frames, 120.0, &events);
        match channels.len() {
            1 => {
                if frames > self.mono_right.len() {
                    return ProcessStatus::Normal;
                }
                let right = &mut self.mono_right[..frames];
                right.copy_from_slice(channels[0]);
                self.eq.process_stereo(&ctx, channels[0], right);
            }
            _ => {
                let (left, rest) = channels.split_at_mut(1);
                self.eq.process_stereo(&ctx, left[0], rest[0]);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ZAudioSimpleEq {
    const CLAP_ID: &'static str = "dev.zaudio.simple-eq";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A simple 3-band Butterworth EQ built on z-audio-dsp");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Equalizer,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for ZAudioSimpleEq {
    const VST3_CLASS_ID: [u8; 16] = *b"ZAudioSimpleEQ01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Eq,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(ZAudioSimpleEq);
nih_export_vst3!(ZAudioSimpleEq);
