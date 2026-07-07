//! Reduced egui editor for platforms without the wry webview (Linux):
//! native WAV/FLAC file loading plus every granular parameter. The full
//! waveform/seek-bar UI is exclusive to the WebCLAP build and the
//! Windows/macOS native builds, which embed the web UI.

use std::path::Path;
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets, EguiState};
use z_audio_webclap_granular::engine::SourceSample;
use z_audio_webclap_granular::protocol::MAX_SAMPLE_FLOATS;

use crate::decode::decode_audio_file;
use crate::shared::GranularShared;
use crate::ZAudioGranularParams;

pub fn create_granular_editor(
    params: Arc<ZAudioGranularParams>,
    shared: Arc<GranularShared>,
) -> Option<Box<dyn Editor>> {
    // Editor-local and not persisted, so host state blobs stay identical
    // to the webview platforms (params only).
    let egui_state = EguiState::from_size(480, 720);
    let message: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    create_egui_editor(
        egui_state,
        (),
        |_, _| {},
        move |ctx, setter, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
                ui.heading("Z Audio Granular");
                ui.label("Load a file, then drive Position (the seek bar) — grains bloom around it. The full waveform UI lives in the WebCLAP / Windows / macOS builds.");
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
                            "{:.0} Hz · {} ch · {:.2} s",
                            status.sample_rate, status.channels, secs
                        )
                    }
                    (None, false) => "No sample loaded".to_string(),
                };
                ui.label(line);
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        section(ui, "Playback");
                        slider(ui, "Position", &params.position, setter);
                        slider(ui, "Level", &params.level, setter);
                        slider(ui, "Pitch", &params.pitch, setter);
                        slider(ui, "Fine", &params.fine, setter);
                        slider(ui, "Root Note", &params.root_note, setter);

                        section(ui, "Grain");
                        slider(ui, "Grain Length", &params.grain_length, setter);
                        slider(ui, "Length Keytrack", &params.length_keytrack, setter);
                        slider(ui, "Grain Attack", &params.grain_attack, setter);
                        slider(ui, "Grain Decay", &params.grain_decay, setter);
                        slider(ui, "Attack Curve", &params.attack_curve, setter);
                        slider(ui, "Decay Curve", &params.decay_curve, setter);
                        slider(ui, "Align Phases", &params.align_phases, setter);
                        slider(ui, "Warm Start", &params.warm_start, setter);

                        section(ui, "Spawn (0 Free · 1 Sync · 2 Density)");
                        slider(ui, "Spawn Mode", &params.spawn_mode, setter);
                        slider(ui, "Rate", &params.rate, setter);
                        slider(ui, "Sync Rate", &params.sync_rate, setter);
                        slider(ui, "Density", &params.density, setter);

                        section(ui, "Random");
                        slider(ui, "Position Spray", &params.random_position, setter);
                        slider(ui, "Timing", &params.random_timing, setter);
                        slider(ui, "Pitch", &params.random_pitch, setter);
                        slider(ui, "Level", &params.random_level, setter);
                        slider(ui, "Pan", &params.random_pan, setter);
                        slider(ui, "Reverse", &params.random_reverse, setter);

                        section(ui, "Chord (type 0 = off)");
                        slider(ui, "Chord Type", &params.chord_type, setter);
                        slider(ui, "Chord Range", &params.chord_range, setter);
                        slider(ui, "Chord Pattern", &params.chord_pattern, setter);

                        section(ui, "Amp Envelope");
                        slider(ui, "Attack", &params.amp_attack, setter);
                        slider(ui, "Decay", &params.amp_decay, setter);
                        slider(ui, "Sustain", &params.amp_sustain, setter);
                        slider(ui, "Release", &params.amp_release, setter);
                    });
            });
        },
    )
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).strong());
}

fn slider(ui: &mut egui::Ui, label: &str, param: &FloatParam, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(widgets::ParamSlider::for_param(param, setter).with_width(240.0));
    });
}

/// Decodes and queues `path` as the new grain source. Returns the
/// status/error line to show in the editor.
fn load_sample(path: &Path, shared: &Arc<GranularShared>) -> String {
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
            if source.frames() < 2 {
                return "File too short".to_string();
            }
            shared.queue_commit(source);
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
