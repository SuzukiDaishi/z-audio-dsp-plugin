use std::sync::Arc;
use std::time::Duration;

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Color32, Pos2, RichText, Shape, Stroke},
    widgets,
};

use crate::{MeterState, ZAudioLimiterParams};

pub fn create_limiter_editor(
    params: Arc<ZAudioLimiterParams>,
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
                header(ui, "Z Audio Limiter");

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.columns(2, |columns| {
                            section(&mut columns[0], "Ceiling", |ui| {
                                draw_limiter_curve(ui, &params);
                                meter(ui, "Gain Reduction", meters.gain_reduction_db(), 36.0);
                                meter(ui, "Input", meters.input_peak_db(), 48.0);
                                meter(ui, "Output", meters.output_peak_db(), 48.0);
                            });

                            section(&mut columns[1], "Level", |ui| {
                                slider(ui, "Input Gain", &params.input_gain, setter);
                                slider(ui, "Threshold", &params.threshold, setter);
                                slider(ui, "Ceiling", &params.ceiling, setter);
                                slider(ui, "Output Gain", &params.output_gain, setter);
                            });
                        });

                        section(ui, "Timing and Detection", |ui| {
                            slider(ui, "Lookahead", &params.lookahead, setter);
                            slider(ui, "Release", &params.release, setter);
                            slider(ui, "Stereo Link", &params.stereo_link, setter);
                            bool_checkbox(ui, "True Peak", &params.true_peak, setter);
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
                ui.set_min_width(300.0);
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

fn bool_checkbox(ui: &mut egui::Ui, label: &str, param: &BoolParam, setter: &ParamSetter) {
    let mut value = param.value();
    if ui.checkbox(&mut value, label).changed() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
}

fn meter(ui: &mut egui::Ui, label: &str, db: f32, range: f32) {
    let level = ((db + range) / range).clamp(0.0, 1.0);
    let color = if label == "Gain Reduction" {
        Color32::from_rgb(235, 100, 92)
    } else {
        Color32::from_rgb(92, 170, 218)
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

fn draw_limiter_curve(ui: &mut egui::Ui, params: &ZAudioLimiterParams) {
    let size = egui::vec2(ui.available_width().max(260.0), 170.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(6),
        Color32::from_rgb(22, 27, 34),
    );

    for i in 0..=4 {
        let t = i as f32 / 4.0;
        let x = egui::lerp(rect.left()..=rect.right(), t);
        let y = egui::lerp(rect.bottom()..=rect.top(), t);
        let grid = Stroke::new(1.0, Color32::from_rgb(45, 55, 66));
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
    let ceiling = params.ceiling.value();
    let input_gain = params.input_gain.value();
    let output_gain = params.output_gain.value();
    let to_screen = |input_db: f32, output_db: f32| {
        let x = ((input_db + 60.0) / 72.0).clamp(0.0, 1.0);
        let y = ((output_db + 60.0) / 72.0).clamp(0.0, 1.0);
        Pos2::new(
            egui::lerp(rect.left()..=rect.right(), x),
            egui::lerp(rect.bottom()..=rect.top(), y),
        )
    };

    let points = (0..=96)
        .map(|i| {
            let input = -60.0 + 72.0 * i as f32 / 96.0;
            let driven = input + input_gain;
            let limited = if driven > threshold {
                threshold + (driven - threshold) * 0.08
            } else {
                driven
            };
            to_screen(input, limited.min(ceiling) + output_gain)
        })
        .collect::<Vec<_>>();
    painter.add(Shape::line(
        points,
        Stroke::new(2.5, Color32::from_rgb(95, 190, 230)),
    ));

    let ceiling_line = vec![to_screen(-60.0, ceiling), to_screen(12.0, ceiling)];
    painter.add(Shape::line(
        ceiling_line,
        Stroke::new(1.5, Color32::from_rgb(235, 118, 105)),
    ));
}
