//! nih-plug parameter struct for the Z Audio Simple Synth.
//!
//! Every field is built from [`ParamId::metadata()`] in `z-audio-dsp`, which remains the single
//! source of truth for each parameter's range, default, and unit. `#[id]` strings mirror
//! [`ParamMetadata::name`] so the on-disk parameter IDs match the DSP's stable identifiers.
//! `ParamId::MaxPolyphony` is intentionally not included: `SimpleSynth`'s polyphony is fixed at
//! construction time (see [`crate::engine::MAX_POLYPHONY`]).

use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{
    ButterworthKind, EnvelopeCurve, GeneratorKind, LfoTarget, LfoWaveform, ParamId, ParamUnit,
};

use crate::enums::{
    ButterworthKindParam, EnvelopeCurveParam, GeneratorKindParam, LfoTargetParam, LfoWaveformParam,
};

/// Build a [`FloatParam`] from a continuous [`ParamId`]'s metadata.
fn float_param(id: ParamId, name: &'static str) -> FloatParam {
    let m = id.metadata();
    let param = FloatParam::new(
        name,
        m.default,
        FloatRange::Linear {
            min: m.min,
            max: m.max,
        },
    );
    match m.unit {
        ParamUnit::Hertz => param
            .with_unit(" Hz")
            .with_smoother(SmoothingStyle::Logarithmic(20.0)),
        ParamUnit::Seconds => param
            .with_unit(" s")
            .with_smoother(SmoothingStyle::Linear(10.0)),
        ParamUnit::Linear
            if matches!(
                id,
                ParamId::EqLowGainDb | ParamId::EqMidGainDb | ParamId::EqHighGainDb
            ) =>
        {
            param
                .with_unit(" dB")
                .with_smoother(SmoothingStyle::Linear(10.0))
        }
        _ => param.with_smoother(SmoothingStyle::Linear(10.0)),
    }
}

/// Build a [`BoolParam`] from a boolean [`ParamId`]'s metadata.
fn bool_param(id: ParamId, name: &'static str) -> BoolParam {
    BoolParam::new(name, id.metadata().default >= 0.5)
}

#[derive(Params)]
pub struct ZAudioSimpleSynthParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[nested(group = "Master")]
    pub master: MasterParams,

    #[nested(group = "Generator")]
    pub generator: GeneratorGroup,

    #[nested(group = "Envelope")]
    pub envelope: EnvelopeGroup,

    #[nested(group = "LFO")]
    pub lfo: LfoGroup,

    #[nested(group = "EQ Low")]
    pub eq_low: EqLowBandParams,

    #[nested(group = "EQ Mid")]
    pub eq_mid: EqMidBandParams,

    #[nested(group = "EQ High")]
    pub eq_high: EqHighBandParams,
}

impl Default for ZAudioSimpleSynthParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(760, 520),
            master: MasterParams::default(),
            generator: GeneratorGroup::default(),
            envelope: EnvelopeGroup::default(),
            lfo: LfoGroup::default(),
            eq_low: EqLowBandParams::default(),
            eq_mid: EqMidBandParams::default(),
            eq_high: EqHighBandParams::default(),
        }
    }
}

#[derive(Params)]
pub struct MasterParams {
    #[id = "master_gain"]
    pub master_gain: FloatParam,

    #[id = "generator_kind"]
    pub generator_kind: EnumParam<GeneratorKindParam>,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self {
            master_gain: float_param(ParamId::MasterGain, "Master Gain"),
            generator_kind: EnumParam::new(
                "Generator Kind",
                GeneratorKindParam::from(GeneratorKind::from_param_value(
                    ParamId::GeneratorKind.metadata().default,
                )),
            ),
        }
    }
}

#[derive(Params)]
pub struct GeneratorGroup {
    #[id = "generator_gain"]
    pub gain: FloatParam,

    #[id = "generator_pulse_width"]
    pub pulse_width: FloatParam,

    #[id = "generator_phase_offset"]
    pub phase_offset: FloatParam,

    #[id = "generator_pan"]
    pub pan: FloatParam,
}

impl Default for GeneratorGroup {
    fn default() -> Self {
        Self {
            gain: float_param(ParamId::GeneratorGain, "Generator Gain"),
            pulse_width: float_param(ParamId::GeneratorPulseWidth, "Pulse Width"),
            phase_offset: float_param(ParamId::GeneratorPhaseOffset, "Phase Offset"),
            pan: float_param(ParamId::GeneratorPan, "Pan"),
        }
    }
}

#[derive(Params)]
pub struct EnvelopeGroup {
    #[id = "env_attack"]
    pub attack: FloatParam,

    #[id = "env_decay"]
    pub decay: FloatParam,

    #[id = "env_sustain"]
    pub sustain: FloatParam,

    #[id = "env_release"]
    pub release: FloatParam,

    #[id = "env_curve"]
    pub curve: EnumParam<EnvelopeCurveParam>,
}

impl Default for EnvelopeGroup {
    fn default() -> Self {
        Self {
            attack: float_param(ParamId::EnvAttack, "Attack"),
            decay: float_param(ParamId::EnvDecay, "Decay"),
            sustain: float_param(ParamId::EnvSustain, "Sustain"),
            release: float_param(ParamId::EnvRelease, "Release"),
            curve: EnumParam::new(
                "Envelope Curve",
                EnvelopeCurveParam::from(EnvelopeCurve::from_param_value(
                    ParamId::EnvCurve.metadata().default,
                )),
            ),
        }
    }
}

#[derive(Params)]
pub struct LfoGroup {
    #[id = "lfo_enabled"]
    pub enabled: BoolParam,

    #[id = "lfo_waveform"]
    pub waveform: EnumParam<LfoWaveformParam>,

    #[id = "lfo_rate_hz"]
    pub rate_hz: FloatParam,

    #[id = "lfo_amount"]
    pub amount: FloatParam,

    #[id = "lfo_target"]
    pub target: EnumParam<LfoTargetParam>,

    #[id = "lfo_retrigger"]
    pub retrigger: BoolParam,
}

impl Default for LfoGroup {
    fn default() -> Self {
        Self {
            enabled: bool_param(ParamId::LfoEnabled, "LFO Enabled"),
            waveform: EnumParam::new(
                "LFO Waveform",
                LfoWaveformParam::from(LfoWaveform::from_param_value(
                    ParamId::LfoWaveform.metadata().default,
                )),
            ),
            rate_hz: float_param(ParamId::LfoRateHz, "LFO Rate"),
            amount: float_param(ParamId::LfoAmount, "LFO Amount"),
            target: EnumParam::new(
                "LFO Target",
                LfoTargetParam::from(LfoTarget::from_param_value(
                    ParamId::LfoTarget.metadata().default,
                )),
            ),
            retrigger: bool_param(ParamId::LfoRetrigger, "LFO Retrigger"),
        }
    }
}

#[derive(Params)]
pub struct EqLowBandParams {
    #[id = "eq_low_enabled"]
    pub enabled: BoolParam,

    #[id = "eq_low_freq"]
    pub freq: FloatParam,

    #[id = "eq_low_type"]
    pub kind: EnumParam<ButterworthKindParam>,

    #[id = "eq_low_gain_db"]
    pub gain_db: FloatParam,

    #[id = "eq_low_q"]
    pub q: FloatParam,
}

impl Default for EqLowBandParams {
    fn default() -> Self {
        Self {
            enabled: bool_param(ParamId::EqLowEnabled, "EQ Low Enabled"),
            freq: float_param(ParamId::EqLowFreq, "EQ Low Frequency"),
            kind: EnumParam::new(
                "EQ Low Type",
                ButterworthKindParam::from(ButterworthKind::from_param_value(
                    ParamId::EqLowType.metadata().default,
                )),
            ),
            gain_db: float_param(ParamId::EqLowGainDb, "EQ Low Gain"),
            q: float_param(ParamId::EqLowQ, "EQ Low Q"),
        }
    }
}

#[derive(Params)]
pub struct EqMidBandParams {
    #[id = "eq_mid_enabled"]
    pub enabled: BoolParam,

    #[id = "eq_mid_freq"]
    pub freq: FloatParam,

    #[id = "eq_mid_type"]
    pub kind: EnumParam<ButterworthKindParam>,

    #[id = "eq_mid_gain_db"]
    pub gain_db: FloatParam,

    #[id = "eq_mid_q"]
    pub q: FloatParam,
}

impl Default for EqMidBandParams {
    fn default() -> Self {
        Self {
            enabled: bool_param(ParamId::EqMidEnabled, "EQ Mid Enabled"),
            freq: float_param(ParamId::EqMidFreq, "EQ Mid Frequency"),
            kind: EnumParam::new(
                "EQ Mid Type",
                ButterworthKindParam::from(ButterworthKind::from_param_value(
                    ParamId::EqMidType.metadata().default,
                )),
            ),
            gain_db: float_param(ParamId::EqMidGainDb, "EQ Mid Gain"),
            q: float_param(ParamId::EqMidQ, "EQ Mid Q"),
        }
    }
}

#[derive(Params)]
pub struct EqHighBandParams {
    #[id = "eq_high_enabled"]
    pub enabled: BoolParam,

    #[id = "eq_high_freq"]
    pub freq: FloatParam,

    #[id = "eq_high_type"]
    pub kind: EnumParam<ButterworthKindParam>,

    #[id = "eq_high_gain_db"]
    pub gain_db: FloatParam,

    #[id = "eq_high_q"]
    pub q: FloatParam,
}

impl Default for EqHighBandParams {
    fn default() -> Self {
        Self {
            enabled: bool_param(ParamId::EqHighEnabled, "EQ High Enabled"),
            freq: float_param(ParamId::EqHighFreq, "EQ High Frequency"),
            kind: EnumParam::new(
                "EQ High Type",
                ButterworthKindParam::from(ButterworthKind::from_param_value(
                    ParamId::EqHighType.metadata().default,
                )),
            ),
            gain_db: float_param(ParamId::EqHighGainDb, "EQ High Gain"),
            q: float_param(ParamId::EqHighQ, "EQ High Q"),
        }
    }
}
