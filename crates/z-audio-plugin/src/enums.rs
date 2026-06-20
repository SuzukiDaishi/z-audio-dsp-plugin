//! Local mirror enums that implement nih-plug's [`Enum`] trait for use with
//! [`nih_plug::params::EnumParam`].
//!
//! The DSP enums in `z-audio-dsp` cannot derive nih-plug's `Enum` directly, so each one gets a
//! local mirror here with an identical variant order. Because `to_param_value()`/
//! `from_param_value()` on the DSP side and `to_index()`/`from_index()` on the nih-plug side both
//! operate on the variants' declaration order, conversion is a simple index round-trip.

use nih_plug::prelude::Enum;
use z_audio_dsp::{ButterworthKind, EnvelopeCurve, GeneratorKind, LfoTarget, LfoWaveform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum GeneratorKindParam {
    Sine,
    Triangle,
    Saw,
    Pulse,
    Noise,
}

impl From<GeneratorKindParam> for GeneratorKind {
    fn from(v: GeneratorKindParam) -> Self {
        GeneratorKind::from_param_value(v.to_index() as f32)
    }
}

impl From<GeneratorKind> for GeneratorKindParam {
    fn from(v: GeneratorKind) -> Self {
        let idx = v
            .to_param_value()
            .round()
            .clamp(0.0, (GeneratorKind::VARIANT_COUNT - 1) as f32) as usize;
        GeneratorKindParam::from_index(idx)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum EnvelopeCurveParam {
    Linear,
    Exponential,
}

impl From<EnvelopeCurveParam> for EnvelopeCurve {
    fn from(v: EnvelopeCurveParam) -> Self {
        EnvelopeCurve::from_param_value(v.to_index() as f32)
    }
}

impl From<EnvelopeCurve> for EnvelopeCurveParam {
    fn from(v: EnvelopeCurve) -> Self {
        let idx = v
            .to_param_value()
            .round()
            .clamp(0.0, (EnvelopeCurve::VARIANT_COUNT - 1) as f32) as usize;
        EnvelopeCurveParam::from_index(idx)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum LfoWaveformParam {
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    RandomHold,
}

impl From<LfoWaveformParam> for LfoWaveform {
    fn from(v: LfoWaveformParam) -> Self {
        LfoWaveform::from_param_value(v.to_index() as f32)
    }
}

impl From<LfoWaveform> for LfoWaveformParam {
    fn from(v: LfoWaveform) -> Self {
        let idx = v
            .to_param_value()
            .round()
            .clamp(0.0, (LfoWaveform::VARIANT_COUNT - 1) as f32) as usize;
        LfoWaveformParam::from_index(idx)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum LfoTargetParam {
    None,
    Gain,
    PitchSemitone,
    EqLowFreq,
    EqMidFreq,
    EqHighFreq,
}

impl From<LfoTargetParam> for LfoTarget {
    fn from(v: LfoTargetParam) -> Self {
        LfoTarget::from_param_value(v.to_index() as f32)
    }
}

impl From<LfoTarget> for LfoTargetParam {
    fn from(v: LfoTarget) -> Self {
        let idx = v
            .to_param_value()
            .round()
            .clamp(0.0, (LfoTarget::VARIANT_COUNT - 1) as f32) as usize;
        LfoTargetParam::from_index(idx)
    }
}

/// Shared filter-shape enum for the low/mid/high EQ bands. Each band stores its own
/// [`ButterworthKind`] value, but the set of choices (and their order) is identical, so a single
/// wrapper type is reused across all three `EnumParam`s.
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
