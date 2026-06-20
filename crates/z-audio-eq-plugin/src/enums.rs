use nih_plug::prelude::Enum;
use z_audio_dsp::ButterworthKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ButterworthKindParam {
    LowPass,
    BandPass,
    HighPass,
}

impl From<ButterworthKindParam> for ButterworthKind {
    fn from(v: ButterworthKindParam) -> Self {
        ButterworthKind::from_param_value(v.to_index() as f32)
    }
}

impl From<ButterworthKind> for ButterworthKindParam {
    fn from(v: ButterworthKind) -> Self {
        let idx = v
            .to_param_value()
            .round()
            .clamp(0.0, (ButterworthKind::VARIANT_COUNT - 1) as f32) as usize;
        ButterworthKindParam::from_index(idx)
    }
}
