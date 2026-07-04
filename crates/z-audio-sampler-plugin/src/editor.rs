use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets};
use z_audio_dsp::SampleBuffer;
use z_audio_synth::SamplerBank;

use crate::decode::decode_audio_file;
use crate::state::{BankUpdate, LoadStatus};
use crate::ZAudioSamplerParams;

/// Default root note assumed for a freshly loaded sample; the user can
/// retune it with the Root Note param afterward.
const DEFAULT_ROOT_NOTE: u8 = 60;

pub fn create_sampler_editor(
    params: Arc<ZAudioSamplerParams>,
    pending_bank: Arc<Mutex<Option<BankUpdate>>>,
    status: Arc<Mutex<LoadStatus>>,
) -> Option<Box<dyn Editor>> {
    let editor_state = params.editor_state.clone();
    create_egui_editor(
        editor_state,
        (),
        |_, _| {},
        move |ctx, setter, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
                ui.heading("Z Audio Sampler");
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Load Sample...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac", "aif", "aiff"])
                            .pick_file()
                        {
                            load_sample(&path, &params, &pending_bank, &status);
                        }
                    }
                    if ui.button("Clear").clicked() {
                        *params.sample_path.lock().unwrap() = None;
                        *status.lock().unwrap() = LoadStatus::Empty;
                        *pending_bank.lock().unwrap() = Some(BankUpdate::Cleared);
                    }
                });
                ui.label(status.lock().unwrap().label());
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        slider(ui, "Master Gain", &params.master_gain, setter);
                        slider(ui, "Root Note", &params.root_note, setter);
                        slider(ui, "Tune", &params.tune, setter);
                        slider(ui, "Offset", &params.offset, setter);
                        slider(ui, "Velocity Curve", &params.velocity_curve, setter);
                        slider(ui, "Release Time", &params.release_time, setter);
                        slider(ui, "Stereo Width", &params.stereo_width, setter);

                        ui.separator();
                        ui.label("Loop");
                        slider(ui, "Loop Mode", &params.loop_mode, setter);
                        slider(ui, "Loop Start", &params.loop_start, setter);
                        slider(ui, "Loop End", &params.loop_end, setter);
                        slider(ui, "Loop Crossfade", &params.loop_xfade, setter);

                        ui.separator();
                        ui.label("Unison");
                        slider(ui, "Voices", &params.unison_voices, setter);
                        slider(ui, "Detune", &params.unison_detune, setter);
                        slider(ui, "Spread", &params.unison_spread, setter);
                    });
            });
        },
    )
}

fn slider(ui: &mut egui::Ui, label: &str, param: &FloatParam, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(widgets::ParamSlider::for_param(param, setter).with_width(260.0));
    });
}

fn load_sample(
    path: &PathBuf,
    params: &Arc<ZAudioSamplerParams>,
    pending_bank: &Arc<Mutex<Option<BankUpdate>>>,
    status: &Arc<Mutex<LoadStatus>>,
) {
    match decode_audio_file(path) {
        Ok((sample_rate, channels, pcm)) => {
            let bank = SamplerBank {
                sample: SampleBuffer::new(sample_rate, channels, pcm),
                default_root_note: DEFAULT_ROOT_NOTE,
            };
            *params.sample_path.lock().unwrap() = Some(path.display().to_string());
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            *status.lock().unwrap() = LoadStatus::Loaded { file_name };
            *pending_bank.lock().unwrap() = Some(BankUpdate::Loaded(Arc::new(bank)));
        }
        Err(message) => {
            *status.lock().unwrap() = LoadStatus::Error { message };
        }
    }
}
