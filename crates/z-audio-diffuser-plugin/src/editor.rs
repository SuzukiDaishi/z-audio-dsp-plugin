use std::sync::Arc;
use std::time::Duration;

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Color32, Pos2, RichText, Shape, Stroke},
    widgets,
};

use crate::{MeterState, ZAudioDiffuserParams};

pub fn create_diffuser_editor(
    params: Arc<ZAudioDiffuserParams>,
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
                header(ui, "Z Audio Diffuser");

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.columns(2, |columns| {
                            section(&mut columns[0], "Allpass Chain", |ui| {
                                draw_diffusion_chain(ui, &params);
                                meter(ui, "Input", meters.input_peak_db(), 48.0);
                                meter(ui, "Output", meters.output_peak_db(), 48.0);
                            });

                            section(&mut columns[1], "Diffusion", |ui| {
                                slider(ui, "Mix", &params.mix, setter);
                                slider(ui, "Diffusion", &params.diffusion, setter);
                                slider(ui, "Allpass Count", &params.allpass_count, setter);
                                slider(ui, "Size", &params.size, setter);
                                slider(ui, "Width", &params.width, setter);
                                slider(ui, "Output", &params.output_gain, setter);
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

fn meter(ui: &mut egui::Ui, label: &str, db: f32, range: f32) {
    let level = ((db + range) / range).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::ProgressBar::new(level)
                .desired_width(190.0)
                .fill(Color32::from_rgb(106, 204, 190))
                .text(format!("{db:.1} dB")),
        );
    });
}

fn draw_diffusion_chain(ui: &mut egui::Ui, params: &ZAudioDiffuserParams) {
    let size = egui::vec2(ui.available_width().max(260.0), 170.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(6),
        Color32::from_rgb(22, 28, 34),
    );

    let diffusion = params.diffusion.value().clamp(0.0, 1.0);
    let allpass_count = params.allpass_count.value().round().clamp(1.0, 100.0);
    let effective_order = diffusion * allpass_count;
    let order = effective_order.round().clamp(0.0, 100.0) as usize;
    let size_amount = params.size.value().clamp(0.0, 1.0);
    let width = params.width.value().clamp(0.0, 1.0);
    let y_mid = rect.center().y;
    let left = rect.left() + 22.0;
    let right = rect.right() - 22.0;
    let visible_stages = order.min(24);

    if visible_stages > 0 {
        let step = (right - left) / visible_stages as f32;
        for i in 0..visible_stages {
            let x0 = left + step * i as f32;
            let x1 = left + step * (i as f32 + 0.72 + size_amount * 0.18);
            let y = y_mid + if i % 2 == 0 { -18.0 } else { 18.0 };
            let color = Color32::from_rgb(
                92 + (diffusion * 80.0) as u8,
                178 + (width * 50.0) as u8,
                208,
            );
            painter.line_segment(
                [Pos2::new(x0, y_mid), Pos2::new(x1, y)],
                Stroke::new(1.4, Color32::from_rgb(78, 92, 106)),
            );
            painter.circle_filled(Pos2::new(x0, y_mid), 4.0, Color32::from_rgb(235, 220, 158));
            painter.circle_stroke(
                Pos2::new(x1, y),
                (14.0 - visible_stages as f32 * 0.22).max(6.0),
                Stroke::new(2.2 + diffusion * 2.0, color),
            );
            painter.circle_filled(Pos2::new(x1, y), 4.5, color);
        }
    }

    let mut points = Vec::with_capacity(96);
    for i in 0..96 {
        let t = i as f32 / 95.0;
        let x = egui::lerp(rect.left() + 14.0..=rect.right() - 14.0, t);
        let decay = (1.0 - t).powf(1.2 + (1.0 - diffusion) * 2.0);
        let y = y_mid
            + (t * (10.0 + effective_order * 0.65)).sin()
                * decay
                * (18.0 + width * 24.0)
                * diffusion;
        points.push(Pos2::new(x, y));
    }
    painter.add(Shape::line(
        points,
        Stroke::new(2.4, Color32::from_rgb(238, 185, 96)),
    ));
}
