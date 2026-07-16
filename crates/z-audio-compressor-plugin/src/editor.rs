use std::sync::Arc;
use std::time::Duration;

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Color32, Pos2, RichText, Shape, Stroke},
    widgets,
};

use crate::{DetectorModeParam, MeterState, ZAudioCompressorParams};

pub fn create_compressor_editor(
    params: Arc<ZAudioCompressorParams>,
    meters: Arc<MeterState>,
) -> Option<Box<dyn Editor>> {
    let editor_state = params.editor_state.clone();
    create_egui_editor(
        editor_state,
        (),
        |_, _| {},
        move |ctx, setter, _| {
            ctx.request_repaint_after(Duration::from_millis(33));
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                header(ui, "Z Audio Compressor");

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.columns(2, |columns| {
                            section(&mut columns[0], "Transfer", |ui| {
                                draw_transfer_curve(ui, &params);
                                meter(ui, "Gain Reduction", meters.gain_reduction_db(), 36.0);
                                meter(ui, "Input", meters.input_peak_db(), 48.0);
                                meter(ui, "Output", meters.output_peak_db(), 48.0);
                            });

                            section(&mut columns[1], "Compression", |ui| {
                                slider(ui, "Threshold", &params.threshold, setter);
                                slider(ui, "Knee", &params.knee, setter);
                                slider(ui, "Ratio", &params.ratio, setter);
                                enum_combo::<DetectorModeParam>(
                                    ui,
                                    "Detector",
                                    &params.detector,
                                    setter,
                                );
                                slider(ui, "SC HPF", &params.sc_hpf, setter);
                                slider(ui, "Lookahead", &params.lookahead, setter);
                            });
                        });

                        ui.columns(2, |columns| {
                            section(&mut columns[0], "Timing", |ui| {
                                slider(ui, "Attack", &params.attack, setter);
                                slider(ui, "Release", &params.release, setter);
                                toggle(ui, "Auto Release", &params.auto_release, setter);
                                slider(ui, "Stereo Link", &params.stereo_link, setter);
                            });
                            section(&mut columns[1], "Level", |ui| {
                                slider(ui, "Input Gain", &params.input_gain, setter);
                                slider(ui, "Makeup", &params.makeup_gain, setter);
                                toggle(ui, "Auto Makeup", &params.auto_makeup, setter);
                                slider(ui, "Warmth", &params.warmth, setter);
                                slider(ui, "Mix", &params.mix, setter);
                            });
                        });
                    });
            });
        },
    )
}

fn header(ui: &mut egui::Ui, title: &str) {
    ui.heading(RichText::new(title).color(Color32::from_rgb(235, 240, 244)));
    ui.separator();
}

fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(310.0);
                add_contents(ui);
            });
        });
}

fn slider<P: Param>(ui: &mut egui::Ui, label: &str, param: &P, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(widgets::ParamSlider::for_param(param, setter).with_width(220.0));
    });
}

fn toggle(ui: &mut egui::Ui, label: &str, param: &BoolParam, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut on = param.value();
        if ui.checkbox(&mut on, "").changed() {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, on);
            setter.end_set_parameter(param);
        }
    });
}

fn enum_combo<T>(ui: &mut egui::Ui, label: &str, param: &EnumParam<T>, setter: &ParamSetter)
where
    T: Enum + PartialEq + Copy + 'static,
{
    ui.horizontal(|ui| {
        ui.label(label);
        let selected_index = param.value().to_index();
        let selected = T::variants()
            .get(selected_index)
            .copied()
            .unwrap_or_default();
        egui::ComboBox::from_id_salt(param.name())
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (index, name) in T::variants().iter().enumerate() {
                    if ui
                        .selectable_label(index == selected_index, *name)
                        .clicked()
                    {
                        setter.begin_set_parameter(param);
                        setter.set_parameter(param, T::from_index(index));
                        setter.end_set_parameter(param);
                    }
                }
            });
    });
}

fn meter(ui: &mut egui::Ui, label: &str, db: f32, range: f32) {
    let level = ((db + range) / range).clamp(0.0, 1.0);
    let color = if label == "Gain Reduction" {
        Color32::from_rgb(235, 110, 88)
    } else {
        Color32::from_rgb(98, 184, 158)
    };
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::ProgressBar::new(level)
                .desired_width(190.0)
                .fill(color)
                .text(format!("{db:.1} dB")),
        );
    });
}

fn draw_transfer_curve(ui: &mut egui::Ui, params: &ZAudioCompressorParams) {
    let size = egui::vec2(ui.available_width().max(260.0), 170.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(6),
        Color32::from_rgb(23, 28, 34),
    );

    for i in 0..=4 {
        let t = i as f32 / 4.0;
        let x = egui::lerp(rect.left()..=rect.right(), t);
        let y = egui::lerp(rect.bottom()..=rect.top(), t);
        let grid = Stroke::new(1.0, Color32::from_rgb(46, 55, 64));
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            grid,
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            grid,
        );
    }

    let threshold = params.threshold.value();
    let knee = params.knee.value();
    let ratio = params.ratio.value();
    let makeup = params.makeup_gain.value();
    let to_screen = |input_db: f32, output_db: f32| {
        let x = ((input_db + 60.0) / 72.0).clamp(0.0, 1.0);
        let y = ((output_db + 60.0) / 72.0).clamp(0.0, 1.0);
        Pos2::new(
            egui::lerp(rect.left()..=rect.right(), x),
            egui::lerp(rect.bottom()..=rect.top(), y),
        )
    };

    let unity = vec![to_screen(-60.0, -60.0), to_screen(12.0, 12.0)];
    painter.add(Shape::line(
        unity,
        Stroke::new(1.0, Color32::from_rgb(86, 96, 108)),
    ));

    let points = (0..=96)
        .map(|i| {
            let input = -60.0 + 72.0 * i as f32 / 96.0;
            let output = compressor_output_db(input, threshold, knee, ratio) + makeup;
            to_screen(input, output)
        })
        .collect::<Vec<_>>();
    painter.add(Shape::line(
        points,
        Stroke::new(2.5, Color32::from_rgb(235, 180, 96)),
    ));
}

fn compressor_output_db(input_db: f32, threshold_db: f32, knee_db: f32, ratio: f32) -> f32 {
    let ratio = ratio.max(1.0);
    let over = input_db - threshold_db;
    let gain_reduction = if knee_db > 0.0 && over.abs() < knee_db * 0.5 {
        let x = over + knee_db * 0.5;
        (1.0 / ratio - 1.0) * x * x / (2.0 * knee_db)
    } else if over > 0.0 {
        (1.0 / ratio - 1.0) * over
    } else {
        0.0
    };
    input_db + gain_reduction
}
