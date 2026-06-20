use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Color32, RichText},
    widgets,
};

use crate::enums::{LfoTargetParam, LfoWaveformParam};
use crate::params::{GeneratorGroup, LfoGroup, ZAudioSimpleSynthParams};

pub fn create_synth_editor(params: Arc<ZAudioSimpleSynthParams>) -> Option<Box<dyn Editor>> {
    let editor_state = params.editor_state.clone();
    create_egui_editor(
        editor_state,
        (),
        |_, _| {},
        move |ctx, setter, _| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
                header(ui, "Z Audio Simple Synth");

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        section(ui, "Master", |ui| {
                            slider(ui, "Master Gain", &params.master.master_gain, setter);
                            enum_combo(ui, "Generator", &params.master.generator_kind, setter);
                        });

                        section(ui, "Generator", |ui| {
                            generator_controls(ui, &params.generator, setter)
                        });
                        section(ui, "Envelope", |ui| {
                            slider(ui, "Attack", &params.envelope.attack, setter);
                            slider(ui, "Decay", &params.envelope.decay, setter);
                            slider(ui, "Sustain", &params.envelope.sustain, setter);
                            slider(ui, "Release", &params.envelope.release, setter);
                            enum_combo(ui, "Curve", &params.envelope.curve, setter);
                        });
                        section(ui, "LFO", |ui| lfo_controls(ui, &params.lfo, setter));
                    });
            });
        },
    )
}

fn header(ui: &mut egui::Ui, title: &str) {
    ui.heading(RichText::new(title).color(Color32::from_rgb(230, 236, 242)));
    ui.separator();
}

fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                add_contents(ui);
            });
        });
}

pub fn slider<P: Param>(ui: &mut egui::Ui, label: &str, param: &P, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        ui.set_min_width(430.0);
        ui.label(label);
        ui.add(widgets::ParamSlider::for_param(param, setter).with_width(260.0));
    });
}

pub fn bool_checkbox(ui: &mut egui::Ui, label: &str, param: &BoolParam, setter: &ParamSetter) {
    let mut value = param.value();
    if ui.checkbox(&mut value, label).changed() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
}

pub fn enum_combo<T>(ui: &mut egui::Ui, label: &str, param: &EnumParam<T>, setter: &ParamSetter)
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

fn generator_controls(ui: &mut egui::Ui, params: &GeneratorGroup, setter: &ParamSetter) {
    slider(ui, "Gain", &params.gain, setter);
    slider(ui, "Pulse Width", &params.pulse_width, setter);
    slider(ui, "Phase Offset", &params.phase_offset, setter);
    slider(ui, "Pan", &params.pan, setter);
}

fn lfo_controls(ui: &mut egui::Ui, params: &LfoGroup, setter: &ParamSetter) {
    bool_checkbox(ui, "Enabled", &params.enabled, setter);
    enum_combo::<LfoWaveformParam>(ui, "Waveform", &params.waveform, setter);
    slider(ui, "Rate", &params.rate_hz, setter);
    slider(ui, "Amount", &params.amount, setter);
    lfo_target_combo(ui, "Target", &params.target, setter);
    bool_checkbox(ui, "Retrigger", &params.retrigger, setter);
}

fn lfo_target_combo(
    ui: &mut egui::Ui,
    label: &str,
    param: &EnumParam<LfoTargetParam>,
    setter: &ParamSetter,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let selected_index = param.value().to_index().min(2);
        let options = ["None", "Gain", "Pitch Semitone"];
        egui::ComboBox::from_id_salt(param.name())
            .selected_text(options[selected_index])
            .show_ui(ui, |ui| {
                for (index, name) in options.iter().enumerate() {
                    if ui
                        .selectable_label(index == selected_index, *name)
                        .clicked()
                    {
                        setter.begin_set_parameter(param);
                        setter.set_parameter(param, LfoTargetParam::from_index(index));
                        setter.end_set_parameter(param);
                    }
                }
            });
    });
}
