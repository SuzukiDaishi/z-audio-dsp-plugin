use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Color32, RichText},
    widgets,
};

use crate::enums::ButterworthKindParam;
use crate::params::ZAudioSimpleEqParams;

pub fn create_eq_editor(params: Arc<ZAudioSimpleEqParams>) -> Option<Box<dyn Editor>> {
    let editor_state = params.editor_state.clone();
    create_egui_editor(
        editor_state,
        (),
        |_, _| {},
        move |ctx, setter, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
                ui.heading(
                    RichText::new("Z Audio Simple EQ").color(Color32::from_rgb(230, 236, 242)),
                );
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        band_section(
                            ui,
                            "Low",
                            &params.low.enabled,
                            &params.low.freq,
                            &params.low.kind,
                            &params.low.gain_db,
                            &params.low.q,
                            setter,
                        );
                        band_section(
                            ui,
                            "Mid",
                            &params.mid.enabled,
                            &params.mid.freq,
                            &params.mid.kind,
                            &params.mid.gain_db,
                            &params.mid.q,
                            setter,
                        );
                        band_section(
                            ui,
                            "High",
                            &params.high.enabled,
                            &params.high.freq,
                            &params.high.kind,
                            &params.high.gain_db,
                            &params.high.q,
                            setter,
                        );
                    });
            });
        },
    )
}

fn band_section(
    ui: &mut egui::Ui,
    title: &str,
    enabled: &BoolParam,
    freq: &FloatParam,
    kind: &EnumParam<ButterworthKindParam>,
    gain_db: &FloatParam,
    q: &FloatParam,
    setter: &ParamSetter,
) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                bool_checkbox(ui, "Enabled", enabled, setter);
                slider(ui, "Frequency", freq, setter);
                enum_combo::<ButterworthKindParam>(ui, "Type", kind, setter);
                slider(ui, "Gain", gain_db, setter);
                slider(ui, "Q", q, setter);
            });
        });
}

fn slider<P: Param>(ui: &mut egui::Ui, label: &str, param: &P, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(widgets::ParamSlider::for_param(param, setter).with_width(260.0));
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
