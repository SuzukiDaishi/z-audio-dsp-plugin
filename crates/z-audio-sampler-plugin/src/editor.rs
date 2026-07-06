//! Reduced egui editor for platforms without the wry webview (Linux):
//! native WAV/FLAC file loading mapped as a single Classic zone, plus the
//! nine global parameters. The full multi-zone UI (trim/loop markers,
//! slicing) is exclusive to the WebCLAP build and the Windows/macOS
//! native builds, which embed the web UI.

use std::path::Path;
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets, EguiState};
use z_audio_webclap_sampler::engine::{classic_zone, SourceSample};
use z_audio_webclap_sampler::protocol::MAX_SAMPLE_FLOATS;

use crate::decode::decode_audio_file;
use crate::shared::SamplerShared;
use crate::ZAudioSamplerParams;

/// Root note assumed for a freshly loaded sample; retune from the host or
/// by reloading via a webview platform.
const DEFAULT_ROOT_NOTE: u8 = 60;

pub fn create_sampler_editor(
    params: Arc<ZAudioSamplerParams>,
    shared: Arc<SamplerShared>,
) -> Option<Box<dyn Editor>> {
    // Editor-local and not persisted, so host state blobs stay identical
    // to the webview platforms (params only).
    let egui_state = EguiState::from_size(440, 500);
    let message: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    create_egui_editor(
        egui_state,
        (),
        |_, _| {},
        move |ctx, setter, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
                ui.heading("Z Audio Sampler");
                ui.label("Loads a file as one chromatic Classic zone. Trim, loop and slice editing live in the WebCLAP / Windows / macOS UI.");
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Load Sample...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac"])
                            .pick_file()
                        {
                            let outcome = load_sample(&path, &shared);
                            *message.lock().unwrap() = Some(outcome);
                        }
                    }
                    if ui.button("Clear").clicked() {
                        shared.queue_clear();
                        *message.lock().unwrap() = None;
                    }
                });

                let status = shared.status();
                let line = match (&*message.lock().unwrap(), status.has_sample) {
                    (Some(text), _) => text.clone(),
                    (None, true) => {
                        let secs = if status.sample_rate > 0.0 {
                            status.frames as f32 / status.sample_rate
                        } else {
                            0.0
                        };
                        format!(
                            "{:.0} Hz · {} ch · {:.2} s · {} zone{}",
                            status.sample_rate,
                            status.channels,
                            secs,
                            status.zone_count,
                            if status.zone_count == 1 { "" } else { "s" }
                        )
                    }
                    (None, false) => "No sample loaded".to_string(),
                };
                ui.label(line);
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        slider(ui, "Master Gain", &params.gain, setter);
                        slider(ui, "Attack", &params.attack, setter);
                        slider(ui, "Decay", &params.decay, setter);
                        slider(ui, "Sustain", &params.sustain, setter);
                        slider(ui, "Release", &params.release, setter);
                        slider(ui, "Tune", &params.tune, setter);
                        slider(ui, "Transpose", &params.transpose, setter);
                        slider(ui, "Velocity Sens", &params.velocity, setter);
                        slider(ui, "Stereo Width", &params.width, setter);
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

/// Decodes and queues `path` as the new source with one Classic zone.
/// Returns the status/error line to show in the editor.
fn load_sample(path: &Path, shared: &Arc<SamplerShared>) -> String {
    match decode_audio_file(path) {
        Ok((sample_rate, channels, pcm)) => {
            let (channels, mut data) = at_most_stereo(channels, pcm);
            // Same 60-second cap the UI applies before uploading.
            let max_frames = MAX_SAMPLE_FLOATS / channels as usize;
            let truncated = data.len() > max_frames * channels as usize;
            data.truncate(max_frames * channels as usize);
            let source = SourceSample {
                sample_rate,
                channels,
                data,
            };
            let frames = source.frames() as u32;
            if frames < 2 {
                return "File too short to map".to_string();
            }
            shared.queue_commit(Some(source), vec![classic_zone(DEFAULT_ROOT_NOTE, frames)]);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            format!(
                "Loaded {name}{}",
                if truncated {
                    " (truncated to 60 s)"
                } else {
                    ""
                }
            )
        }
        Err(message) => format!("Load failed: {message}"),
    }
}

/// The engine plays mono or interleaved stereo; drop extra channels from
/// surround files (keeping the first two).
fn at_most_stereo(channels: u8, pcm: Vec<f32>) -> (u8, Vec<f32>) {
    let channels = channels.max(1);
    if channels <= 2 {
        return (channels, pcm);
    }
    let src_channels = channels as usize;
    let frames = pcm.len() / src_channels;
    let mut out = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        out.push(pcm[frame * src_channels]);
        out.push(pcm[frame * src_channels + 1]);
    }
    (2, out)
}
