use std::sync::Arc;
use std::time::Duration;

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Color32, Pos2, RichText, Shape, Stroke},
    widgets,
};

use crate::{MeterState, ZAudioReverbParams};

pub fn create_reverb_editor(
    params: Arc<ZAudioReverbParams>,
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
                header(ui, "Z Audio Parametric Reverb");

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.columns(2, |columns| {
                            section(&mut columns[0], "Room and Tail", |ui| {
                                draw_room(ui, &params);
                                meter(ui, "Input", meters.input_peak_db(), 48.0);
                                meter(ui, "Output", meters.output_peak_db(), 48.0);
                                meter(ui, "Tail Delta", meters.tail_level_db(), 48.0);
                            });

                            section(&mut columns[1], "Space", |ui| {
                                slider(ui, "Mix", &params.mix, setter);
                                slider(ui, "Room Size", &params.room_size, setter);
                                slider(ui, "Decay", &params.decay, setter);
                                slider(ui, "Pre Delay", &params.pre_delay, setter);
                                slider(ui, "Early/Late", &params.early_late, setter);
                            });
                        });

                        ui.columns(2, |columns| {
                            section(&mut columns[0], "Texture", |ui| {
                                slider(ui, "Diffusion", &params.diffusion, setter);
                                slider(ui, "Damping", &params.damping, setter);
                                slider(ui, "Width", &params.width, setter);
                            });
                            section(&mut columns[1], "Tone", |ui| {
                                slider(ui, "Low Cut", &params.low_cut, setter);
                                slider(ui, "High Cut", &params.high_cut, setter);
                                slider(ui, "Output Gain", &params.output_gain, setter);
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
                ui.set_min_width(330.0);
                add_contents(ui);
            });
        });
}

fn slider<P: Param>(ui: &mut egui::Ui, label: &str, param: &P, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(widgets::ParamSlider::for_param(param, setter).with_width(230.0));
    });
}

fn meter(ui: &mut egui::Ui, label: &str, db: f32, range: f32) {
    let level = ((db + range) / range).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::ProgressBar::new(level)
                .desired_width(200.0)
                .fill(Color32::from_rgb(112, 180, 156))
                .text(format!("{db:.1} dB")),
        );
    });
}

fn draw_room(ui: &mut egui::Ui, params: &ZAudioReverbParams) {
    let size = egui::vec2(ui.available_width().max(300.0), 190.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(6),
        Color32::from_rgb(21, 27, 31),
    );

    let room = params.room_size.value().clamp(0.0, 1.0);
    let decay = params.decay.value();
    let diffusion = params.diffusion.value().clamp(0.0, 1.0);
    let damping = params.damping.value().clamp(0.0, 1.0);
    let early = 1.0 - params.early_late.value().clamp(0.0, 1.0);

    let center = rect.center();
    let room_w = egui::lerp(90.0..=rect.width() * 0.82, room);
    let room_h = egui::lerp(48.0..=rect.height() * 0.72, room);
    let left = center.x - room_w * 0.5;
    let right = center.x + room_w * 0.5;
    let top = center.y - room_h * 0.5;
    let bottom = center.y + room_h * 0.5;
    let wall = Stroke::new(2.0, Color32::from_rgb(102, 154, 178));
    painter.line_segment([Pos2::new(left, top), Pos2::new(right, top)], wall);
    painter.line_segment([Pos2::new(right, top), Pos2::new(right, bottom)], wall);
    painter.line_segment([Pos2::new(right, bottom), Pos2::new(left, bottom)], wall);
    painter.line_segment([Pos2::new(left, bottom), Pos2::new(left, top)], wall);

    for i in 0..8 {
        let t = i as f32 / 7.0;
        let spread = 0.18 + t * (0.68 + diffusion * 0.24);
        let y = egui::lerp(top..=bottom, t);
        let fade = ((1.0 - t) * (0.35 + decay / 8.0)).clamp(0.12, 1.0);
        let color = Color32::from_rgba_premultiplied(
            120,
            (170.0 - damping * 60.0) as u8,
            190,
            (fade * 150.0) as u8,
        );
        painter.line_segment(
            [
                Pos2::new(center.x - room_w * spread * 0.5, y),
                Pos2::new(center.x + room_w * spread * 0.5, y),
            ],
            Stroke::new(1.0 + diffusion * 1.6, color),
        );
    }

    let tail_points = (0..=80)
        .map(|i| {
            let t = i as f32 / 80.0;
            let x = egui::lerp(rect.left() + 16.0..=rect.right() - 16.0, t);
            let curve = (-(t * 5.0 / decay.max(0.2))).exp();
            let y = egui::lerp(rect.bottom() - 18.0..=rect.top() + 28.0, curve);
            Pos2::new(x, y)
        })
        .collect::<Vec<_>>();
    painter.add(Shape::line(
        tail_points,
        Stroke::new(2.5, Color32::from_rgb(230, 175, 100)),
    ));

    let early_x = egui::lerp(rect.left() + 16.0..=rect.right() - 16.0, early);
    painter.line_segment(
        [
            Pos2::new(early_x, rect.top() + 18.0),
            Pos2::new(early_x, rect.bottom() - 18.0),
        ],
        Stroke::new(1.5, Color32::from_rgb(235, 120, 110)),
    );
}
