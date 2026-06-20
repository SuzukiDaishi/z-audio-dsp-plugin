use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use z_audio_dsp::{ButterworthKind, ParamId, ParamUnit};

use crate::enums::ButterworthKindParam;

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

fn bool_param(id: ParamId, name: &'static str) -> BoolParam {
    BoolParam::new(name, id.metadata().default >= 0.5)
}

#[derive(Params)]
pub struct ZAudioSimpleEqParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[nested(group = "EQ Low")]
    pub low: EqLowBandParams,

    #[nested(group = "EQ Mid")]
    pub mid: EqMidBandParams,

    #[nested(group = "EQ High")]
    pub high: EqHighBandParams,
}

impl Default for ZAudioSimpleEqParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(520, 360),
            low: EqLowBandParams::default(),
            mid: EqMidBandParams::default(),
            high: EqHighBandParams::default(),
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
            enabled: bool_param(ParamId::EqLowEnabled, "Low Enabled"),
            freq: float_param(ParamId::EqLowFreq, "Low Frequency"),
            kind: EnumParam::new(
                "Low Type",
                ButterworthKindParam::from(ButterworthKind::from_param_value(
                    ParamId::EqLowType.metadata().default,
                )),
            ),
            gain_db: float_param(ParamId::EqLowGainDb, "Low Gain"),
            q: float_param(ParamId::EqLowQ, "Low Q"),
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
            enabled: bool_param(ParamId::EqMidEnabled, "Mid Enabled"),
            freq: float_param(ParamId::EqMidFreq, "Mid Frequency"),
            kind: EnumParam::new(
                "Mid Type",
                ButterworthKindParam::from(ButterworthKind::from_param_value(
                    ParamId::EqMidType.metadata().default,
                )),
            ),
            gain_db: float_param(ParamId::EqMidGainDb, "Mid Gain"),
            q: float_param(ParamId::EqMidQ, "Mid Q"),
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
            enabled: bool_param(ParamId::EqHighEnabled, "High Enabled"),
            freq: float_param(ParamId::EqHighFreq, "High Frequency"),
            kind: EnumParam::new(
                "High Type",
                ButterworthKindParam::from(ButterworthKind::from_param_value(
                    ParamId::EqHighType.metadata().default,
                )),
            ),
            gain_db: float_param(ParamId::EqHighGainDb, "High Gain"),
            q: float_param(ParamId::EqHighQ, "High Q"),
        }
    }
}
