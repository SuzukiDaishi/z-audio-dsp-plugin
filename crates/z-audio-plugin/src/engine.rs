//! Wraps [`SimpleSynth`] with the bookkeeping needed to drive it from a host: (re)construction at
//! the host's sample rate / block size, and per-block translation of the current
//! [`ZAudioSimpleSynthParams`] values plus MIDI note events into a sorted [`TimedEvent`] list for
//! [`SimpleSynth::process_with_context`].

use z_audio_dsp::{
    ButterworthKind, EnvelopeCurve, EventKind, GeneratorKind, LfoTarget, LfoWaveform, ParamId,
    ProcessContext, TimedEvent,
};
use z_audio_synth::{SimpleSynth, SimpleSynthConfig};

use crate::params::ZAudioSimpleSynthParams;

/// `SimpleSynth`'s polyphony is fixed at construction time and is not exposed as an automatable
/// parameter; this matches `ParamId::MaxPolyphony.metadata().default`.
pub const MAX_POLYPHONY: usize = 16;

/// Every automatable [`ParamId`] (i.e. [`ParamId::ALL`] minus [`ParamId::MaxPolyphony`]), in
/// [`ParamId::ALL`] order. Used to scan [`ZAudioSimpleSynthParams`] for changes once per block.
const AUTOMATABLE_PARAM_IDS: [ParamId; 32] = [
    ParamId::MasterGain,
    ParamId::GeneratorKind,
    ParamId::GeneratorGain,
    ParamId::GeneratorPulseWidth,
    ParamId::GeneratorPhaseOffset,
    ParamId::GeneratorPan,
    ParamId::EnvAttack,
    ParamId::EnvDecay,
    ParamId::EnvSustain,
    ParamId::EnvRelease,
    ParamId::EnvCurve,
    ParamId::LfoEnabled,
    ParamId::LfoWaveform,
    ParamId::LfoRateHz,
    ParamId::LfoAmount,
    ParamId::LfoTarget,
    ParamId::LfoRetrigger,
    ParamId::EqLowEnabled,
    ParamId::EqLowFreq,
    ParamId::EqLowType,
    ParamId::EqMidEnabled,
    ParamId::EqMidFreq,
    ParamId::EqMidType,
    ParamId::EqHighEnabled,
    ParamId::EqHighFreq,
    ParamId::EqHighType,
    ParamId::EqLowGainDb,
    ParamId::EqLowQ,
    ParamId::EqMidGainDb,
    ParamId::EqMidQ,
    ParamId::EqHighGainDb,
    ParamId::EqHighQ,
];

fn bool_to_param_value(value: bool) -> f32 {
    if value {
        1.0
    } else {
        0.0
    }
}

/// Reads `id`'s current plain value out of `params`, in the encoding expected by
/// [`SimpleSynth::set_param`]/returned by [`SimpleSynth::param_value`].
fn current_param_value(params: &ZAudioSimpleSynthParams, id: ParamId) -> f32 {
    match id {
        ParamId::MasterGain => params.master.master_gain.value(),
        ParamId::MaxPolyphony => MAX_POLYPHONY as f32,
        ParamId::GeneratorKind => {
            GeneratorKind::from(params.master.generator_kind.value()).to_param_value()
        }
        ParamId::GeneratorGain => params.generator.gain.value(),
        ParamId::GeneratorPulseWidth => params.generator.pulse_width.value(),
        ParamId::GeneratorPhaseOffset => params.generator.phase_offset.value(),
        ParamId::GeneratorPan => params.generator.pan.value(),
        ParamId::EnvAttack => params.envelope.attack.value(),
        ParamId::EnvDecay => params.envelope.decay.value(),
        ParamId::EnvSustain => params.envelope.sustain.value(),
        ParamId::EnvRelease => params.envelope.release.value(),
        ParamId::EnvCurve => EnvelopeCurve::from(params.envelope.curve.value()).to_param_value(),
        ParamId::LfoEnabled => bool_to_param_value(params.lfo.enabled.value()),
        ParamId::LfoWaveform => LfoWaveform::from(params.lfo.waveform.value()).to_param_value(),
        ParamId::LfoRateHz => params.lfo.rate_hz.value(),
        ParamId::LfoAmount => params.lfo.amount.value(),
        ParamId::LfoTarget => LfoTarget::from(params.lfo.target.value()).to_param_value(),
        ParamId::LfoRetrigger => bool_to_param_value(params.lfo.retrigger.value()),
        ParamId::EqLowEnabled => bool_to_param_value(params.eq_low.enabled.value()),
        ParamId::EqLowFreq => params.eq_low.freq.value(),
        ParamId::EqLowType => ButterworthKind::from(params.eq_low.kind.value()).to_param_value(),
        ParamId::EqLowGainDb => params.eq_low.gain_db.value(),
        ParamId::EqLowQ => params.eq_low.q.value(),
        ParamId::EqMidEnabled => bool_to_param_value(params.eq_mid.enabled.value()),
        ParamId::EqMidFreq => params.eq_mid.freq.value(),
        ParamId::EqMidType => ButterworthKind::from(params.eq_mid.kind.value()).to_param_value(),
        ParamId::EqMidGainDb => params.eq_mid.gain_db.value(),
        ParamId::EqMidQ => params.eq_mid.q.value(),
        ParamId::EqHighEnabled => bool_to_param_value(params.eq_high.enabled.value()),
        ParamId::EqHighFreq => params.eq_high.freq.value(),
        ParamId::EqHighType => ButterworthKind::from(params.eq_high.kind.value()).to_param_value(),
        ParamId::EqHighGainDb => params.eq_high.gain_db.value(),
        ParamId::EqHighQ => params.eq_high.q.value(),
    }
}

/// Owns the [`SimpleSynth`] instance and the scratch buffer used to build each block's event
/// list.
pub struct SynthEngine {
    synth: Option<SimpleSynth>,
    events: Vec<TimedEvent>,
    last_values: [f32; AUTOMATABLE_PARAM_IDS.len()],
}

impl SynthEngine {
    pub fn new() -> Self {
        Self {
            synth: None,
            events: Vec::with_capacity(64),
            last_values: [0.0; AUTOMATABLE_PARAM_IDS.len()],
        }
    }

    /// (Re)creates the underlying [`SimpleSynth`] for `sample_rate`/`max_block_size` and applies
    /// every current parameter value. Called from `initialize()` and `reset()`, since
    /// `SimpleSynth` has no public re-prepare API.
    pub fn reinit(
        &mut self,
        sample_rate: f32,
        max_block_size: usize,
        params: &ZAudioSimpleSynthParams,
    ) {
        let wanted_event_capacity = max_block_size
            .max(64)
            .saturating_add(AUTOMATABLE_PARAM_IDS.len());
        if self.events.capacity() < wanted_event_capacity {
            self.events
                .reserve_exact(wanted_event_capacity - self.events.capacity());
        }

        let mut synth = SimpleSynth::new(SimpleSynthConfig {
            sample_rate,
            max_block_size,
            max_polyphony: MAX_POLYPHONY,
        });

        for (i, id) in AUTOMATABLE_PARAM_IDS.iter().enumerate() {
            let value = current_param_value(params, *id);
            synth.set_param(*id, value);
            self.last_values[i] = value;
        }

        self.synth = Some(synth);
    }

    /// Renders one block: builds a sample-accurate event list from `note_events` plus any
    /// parameter changes detected since the last call, then runs [`SimpleSynth::process_with_context`].
    pub fn process_block(
        &mut self,
        params: &ZAudioSimpleSynthParams,
        note_events: impl Iterator<Item = TimedEvent>,
        sample_rate: f32,
        tempo_bpm: f32,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let Some(synth) = self.synth.as_mut() else {
            left.fill(0.0);
            right.fill(0.0);
            return;
        };

        self.events.clear();

        // Parameter-change events are pushed at `sample_offset: 0` first, so that (after the
        // stable sort below) they're applied before any note-on events scheduled for the same
        // sample. This lets notes triggered in this block pick up freshly-changed
        // generator/envelope/LFO settings.
        for (i, id) in AUTOMATABLE_PARAM_IDS.iter().enumerate() {
            let value = current_param_value(params, *id);
            if value != self.last_values[i] {
                self.last_values[i] = value;
                self.events.push(TimedEvent {
                    sample_offset: 0,
                    kind: EventKind::Param { id: *id, value },
                });
            }
        }

        for event in note_events {
            if self.events.len() < self.events.capacity() {
                self.events.push(event);
            }
        }
        self.events.sort_by_key(|event| event.sample_offset);

        let ctx = ProcessContext::new(sample_rate, left.len(), tempo_bpm, &self.events);
        synth.process_with_context(&ctx, left, right);
    }
}
