//! The wavetable synth engine: voices, unison oscillators, per-voice TPT
//! state-variable filter, two ADSR envelopes, two LFOs, and an 8-slot
//! modulation matrix.
//!
//! Rate split:
//! - **Control rate** (every [`CONTROL_BLOCK`] samples): LFO/mod-matrix
//!   evaluation, effective pitch/WT-pos/gain targets, filter coefficients,
//!   mip selection, glide slew.
//! - **Audio rate**: wavetable reads (with per-block linear ramps on WT
//!   pos and gains), envelope levels, filter state updates.
//!
//! All randomness is a per-engine xorshift32 (same recipe as the granular
//! synth), so renders are deterministic and unit-testable.

use crate::params::*;
use crate::wavetable::{WavetableSet, MAX_HARMONICS, MIPS};
use z_audio_dsp::flush_denormal;

pub const MAX_VOICES: usize = 16;
pub const MAX_UNISON: usize = 8;
pub const CONTROL_BLOCK: usize = 32;

// Full-scale modulation reach, per the plan: ±12 semitones for pitch
// destinations, ±4 octaves for cutoff.
const PITCH_MOD_SEMIS: f32 = 12.0;
const CUTOFF_MOD_OCTAVES: f32 = 4.0;
// Unison detune param 1.0 spreads the outermost voices ±50 cents.
const UNISON_SPREAD_CENTS: f32 = 50.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OscParams {
    pub enable: bool,
    pub table: u8,
    pub wt_pos: f32,
    pub octave: i8,
    pub semi: i8,
    pub fine_cents: f32,
    pub unison: u8,
    pub uni_detune: f32,
    pub uni_blend: f32,
    pub phase: f32,
    pub rand_phase: f32,
    pub pan: f32,
    pub level: f32,
}

impl OscParams {
    fn default_with(enable: bool) -> Self {
        Self {
            enable,
            table: 0,
            wt_pos: 0.0,
            octave: 0,
            semi: 0,
            fine_cents: 0.0,
            unison: 1,
            uni_detune: 0.25,
            uni_blend: 0.75,
            phase: 0.0,
            rand_phase: 1.0,
            pan: 0.0,
            level: 0.75,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EnvParams {
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
    pub curve: f32,
}

impl EnvParams {
    fn default_with(sustain: f32) -> Self {
        Self {
            attack_s: 0.005,
            decay_s: 0.2,
            sustain,
            release_s: 0.15,
            curve: 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LfoParams {
    pub wave: u8,
    pub rate_hz: f32,
    pub phase: f32,
    pub retrig: bool,
}

impl Default for LfoParams {
    fn default() -> Self {
        Self {
            wave: 0,
            rate_hz: 2.0,
            phase: 0.0,
            retrig: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ModSlot {
    pub source: u8,
    pub dest: u8,
    pub amount: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SynthParams {
    pub master: f32,
    pub polyphony: u8,
    pub bend_range: f32,
    pub glide_s: f32,
    pub osc_a: OscParams,
    pub osc_b: OscParams,
    pub filter_enable: bool,
    pub filter_type: u8,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub drive: f32,
    pub keytrack: f32,
    pub filter_mix: f32,
    pub route_a: bool,
    pub route_b: bool,
    pub env1: EnvParams,
    pub env2: EnvParams,
    pub lfo1: LfoParams,
    pub lfo2: LfoParams,
    pub mods: [ModSlot; MOD_SLOTS as usize],
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            master: 0.8,
            polyphony: 8,
            bend_range: 2.0,
            glide_s: 0.0,
            osc_a: OscParams::default_with(true),
            osc_b: OscParams::default_with(false),
            filter_enable: true,
            filter_type: 0,
            cutoff_hz: 20_000.0,
            resonance: 0.15,
            drive: 0.0,
            keytrack: 0.0,
            filter_mix: 1.0,
            route_a: true,
            route_b: true,
            env1: EnvParams::default_with(0.7),
            env2: EnvParams::default_with(0.5),
            lfo1: LfoParams::default(),
            lfo2: LfoParams::default(),
            mods: [ModSlot::default(); MOD_SLOTS as usize],
        }
    }
}

/// xorshift32 — same deterministic PRNG recipe as the granular engine.
#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }

    /// Uniform in [-1, 1].
    fn next_bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------------------
// ADSR
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// ADSR with a shared curvature control. Stage position advances linearly
/// per sample; the curve maps position → level through `x^(2^(3c))`,
/// evaluated via a 257-entry LUT so the audio path never calls `powf`.
#[derive(Clone)]
struct Adsr {
    stage: EnvStage,
    /// Position within the current stage, 0..1.
    pos: f32,
    level: f32,
    release_from: f32,
    curve_lut: [f32; 257],
    lut_curve: f32,
}

impl Adsr {
    fn new() -> Self {
        let mut e = Self {
            stage: EnvStage::Idle,
            pos: 0.0,
            level: 0.0,
            release_from: 0.0,
            curve_lut: [0.0; 257],
            lut_curve: f32::NAN,
        };
        e.rebuild_lut(0.0);
        e
    }

    fn rebuild_lut(&mut self, curve: f32) {
        if curve == self.lut_curve {
            return;
        }
        let exponent = (3.0 * curve.clamp(-1.0, 1.0)).exp2();
        for (i, v) in self.curve_lut.iter_mut().enumerate() {
            *v = (i as f32 / 256.0).powf(exponent);
        }
        self.lut_curve = curve;
    }

    #[inline]
    fn shape(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0) * 256.0;
        let i = (x as usize).min(255);
        let f = x - i as f32;
        self.curve_lut[i] + (self.curve_lut[i + 1] - self.curve_lut[i]) * f
    }

    fn gate_on(&mut self) {
        // Restart the attack from the current level so retriggers don't click.
        self.pos = if self.level > 0.0 {
            // Find the attack position that resumes at the current level:
            // shape is monotonic, a coarse inverse is fine here.
            let mut lo = 0.0f32;
            let mut hi = 1.0f32;
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                if self.shape(mid) < self.level {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        } else {
            0.0
        };
        self.stage = EnvStage::Attack;
    }

    fn gate_off(&mut self) {
        if !matches!(self.stage, EnvStage::Idle | EnvStage::Release) {
            self.release_from = self.level;
            self.pos = 0.0;
            self.stage = EnvStage::Release;
        }
    }

    fn is_idle(&self) -> bool {
        matches!(self.stage, EnvStage::Idle)
    }

    /// Advance one sample and return the new level.
    #[inline]
    fn tick(&mut self, p: &EnvParams, inv_sr: f32) -> f32 {
        match self.stage {
            EnvStage::Idle => {
                self.level = 0.0;
            }
            EnvStage::Attack => {
                let step = inv_sr / p.attack_s.max(0.0005);
                self.pos += step;
                if self.pos >= 1.0 {
                    self.level = 1.0;
                    self.pos = 0.0;
                    self.stage = EnvStage::Decay;
                } else {
                    self.level = self.shape(self.pos);
                }
            }
            EnvStage::Decay => {
                let step = inv_sr / p.decay_s.max(0.0005);
                self.pos += step;
                let sustain = p.sustain.clamp(0.0, 1.0);
                if self.pos >= 1.0 {
                    self.level = sustain;
                    self.stage = EnvStage::Sustain;
                } else {
                    self.level = sustain + (1.0 - sustain) * self.shape(1.0 - self.pos);
                }
            }
            EnvStage::Sustain => {
                self.level = p.sustain.clamp(0.0, 1.0);
                if self.level <= 1.0e-5 {
                    // Silent sustain behaves like the note ended.
                    self.stage = EnvStage::Idle;
                }
            }
            EnvStage::Release => {
                let step = inv_sr / p.release_s.max(0.0005);
                self.pos += step;
                if self.pos >= 1.0 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                } else {
                    self.level = self.release_from * self.shape(1.0 - self.pos);
                }
            }
        }
        self.level
    }
}

// ---------------------------------------------------------------------------
// LFO
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Lfo {
    phase: f32,
    /// Latched sample-and-hold value, renewed on each phase wrap.
    sh_value: f32,
    rng: Rng,
}

impl Lfo {
    fn new(seed: u32) -> Self {
        let mut rng = Rng(seed | 1);
        let sh = rng.next_bipolar();
        Self {
            phase: 0.0,
            sh_value: sh,
            rng,
        }
    }

    fn retrigger(&mut self, start_phase: f32) {
        self.phase = start_phase.rem_euclid(1.0);
        self.sh_value = self.rng.next_bipolar();
    }

    /// Advance by `dt` seconds and return the bipolar value in [-1, 1].
    fn advance(&mut self, p: &LfoParams, dt: f32) -> f32 {
        self.phase += p.rate_hz.max(0.01) * dt;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.sh_value = self.rng.next_bipolar();
        }
        self.value(p)
    }

    fn value(&self, p: &LfoParams) -> f32 {
        let x = self.phase;
        match p.wave {
            0 => (core::f32::consts::TAU * x).sin(),
            1 => {
                // triangle: -1 → 1 → -1
                if x < 0.5 {
                    4.0 * x - 1.0
                } else {
                    3.0 - 4.0 * x
                }
            }
            2 => 2.0 * x - 1.0, // saw up
            3 => {
                if x < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            _ => self.sh_value,
        }
    }
}

// ---------------------------------------------------------------------------
// TPT state-variable filter
// ---------------------------------------------------------------------------

/// One TPT (cytomic) SVF stage; stereo pairs live side by side in `Voice`.
#[derive(Clone, Copy, Default)]
struct SvfState {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy)]
struct SvfCoeffs {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl SvfCoeffs {
    fn compute(cutoff_hz: f32, resonance: f32, sample_rate: f32) -> Self {
        let fc = cutoff_hz.clamp(10.0, sample_rate * 0.49);
        // tan() via sin/cos keeps this no-surprises on wasm.
        let w = core::f32::consts::PI * fc / sample_rate;
        let g = w.sin() / w.cos();
        let k = 2.0 - 1.9 * resonance.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { k, a1, a2, a3 }
    }
}

impl SvfState {
    /// One TPT tick returning (low, band, high).
    #[inline]
    fn tick(&mut self, x: f32, c: &SvfCoeffs) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let high = x - c.k * v1 - v2;
        (v2, v1, high)
    }

    fn flush(&mut self) {
        self.ic1 = flush_denormal(self.ic1);
        self.ic2 = flush_denormal(self.ic2);
    }
}

/// Cheap tanh-shaped saturator for the filter drive.
#[inline]
fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

// ---------------------------------------------------------------------------
// Unison oscillator
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct UnisonOsc {
    phases: [f32; MAX_UNISON],
    /// Per-unison-voice pitch offset factors, recomputed at control rate.
    ratios: [f32; MAX_UNISON],
    gains_l: [f32; MAX_UNISON],
    gains_r: [f32; MAX_UNISON],
}

impl UnisonOsc {
    fn new() -> Self {
        Self {
            phases: [0.0; MAX_UNISON],
            ratios: [1.0; MAX_UNISON],
            gains_l: [0.0; MAX_UNISON],
            gains_r: [0.0; MAX_UNISON],
        }
    }

    fn trigger(&mut self, p: &OscParams, rng: &mut Rng) {
        for v in 0..MAX_UNISON {
            let random = rng.next_f32();
            self.phases[v] = (p.phase + p.rand_phase * random).rem_euclid(1.0);
        }
    }

    /// Symmetric detune offset in [-1, 1] for unison voice `v` of `n`.
    #[inline]
    fn spread(v: usize, n: usize) -> f32 {
        if n <= 1 {
            0.0
        } else {
            (2.0 * v as f32 / (n - 1) as f32) - 1.0
        }
    }

    /// Refresh ratios and stereo gains for the current params + mods.
    /// `pan` is the already-modulated oscillator pan in [-1, 1].
    fn update_control(&mut self, p: &OscParams, pan: f32, level: f32) {
        let n = (p.unison as usize).clamp(1, MAX_UNISON);
        let mut norm = 0.0f32;
        let mut gains = [0.0f32; MAX_UNISON];
        for v in 0..n {
            let s = Self::spread(v, n);
            let cents = s * p.uni_detune * UNISON_SPREAD_CENTS;
            self.ratios[v] = (cents / 1200.0).exp2();
            // Center voices full, outer voices scaled by blend.
            let g = 1.0 - s.abs() * (1.0 - p.uni_blend);
            gains[v] = g;
            norm += g * g;
        }
        let norm = if norm > 0.0 { norm.sqrt().recip() } else { 0.0 };
        // Equal-power master pan for the oscillator...
        let angle = (pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
        let (pan_l, pan_r) = (angle.cos(), angle.sin());
        for v in 0..n {
            // ...plus a per-voice constant spread that widens with detune.
            let s = Self::spread(v, n);
            let width = 0.75 * p.uni_detune.min(1.0);
            let l_w = 1.0 - (s * width).max(0.0);
            let r_w = 1.0 + (s * width).min(0.0);
            let g = gains[v] * norm * level;
            self.gains_l[v] = g * pan_l * l_w;
            self.gains_r[v] = g * pan_r * r_w;
        }
        for v in n..MAX_UNISON {
            self.gains_l[v] = 0.0;
            self.gains_r[v] = 0.0;
        }
    }
}

// ---------------------------------------------------------------------------
// Voice
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Voice {
    active: bool,
    key: u8,
    velocity: f32,
    gate: bool,
    age: u64,
    /// Current (possibly gliding) pitch in MIDI-note units.
    note_pitch: f32,
    glide_target: f32,
    env1: Adsr,
    env2: Adsr,
    lfo1: Lfo,
    lfo2: Lfo,
    osc_a: UnisonOsc,
    osc_b: UnisonOsc,
    svf1_l: SvfState,
    svf1_r: SvfState,
    svf2_l: SvfState,
    svf2_r: SvfState,
}

impl Voice {
    fn new(seed: u32) -> Self {
        Self {
            active: false,
            key: 0,
            velocity: 0.0,
            gate: false,
            age: 0,
            note_pitch: 60.0,
            glide_target: 60.0,
            env1: Adsr::new(),
            env2: Adsr::new(),
            lfo1: Lfo::new(seed.wrapping_mul(2654435761).wrapping_add(1)),
            lfo2: Lfo::new(seed.wrapping_mul(40503).wrapping_add(7)),
            osc_a: UnisonOsc::new(),
            osc_b: UnisonOsc::new(),
            svf1_l: SvfState::default(),
            svf1_r: SvfState::default(),
            svf2_l: SvfState::default(),
            svf2_r: SvfState::default(),
        }
    }
}

/// Per-control-block modulation offsets, one entry per destination.
#[derive(Clone, Copy, Default)]
struct ModOffsets {
    a_wt: f32,
    a_pitch: f32,
    a_level: f32,
    a_pan: f32,
    b_wt: f32,
    b_pitch: f32,
    b_level: f32,
    b_pan: f32,
    cutoff_oct: f32,
    reso: f32,
    master: f32,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct SynthEngine {
    params: SynthParams,
    tables: WavetableSet,
    voices: Vec<Voice>,
    sample_rate: f32,
    inv_sample_rate: f32,
    age_counter: u64,
    note_counter: u32,
    last_note: f32,
    /// Free-running LFO phases shared by voices with retrig off.
    free_lfo1: Lfo,
    free_lfo2: Lfo,
    /// Smoothed master gain (one-pole per control block).
    master_smooth: f32,
}

impl SynthEngine {
    pub fn new(sample_rate: f32) -> Self {
        let mut voices = Vec::with_capacity(MAX_VOICES);
        for i in 0..MAX_VOICES {
            voices.push(Voice::new(0x9E37_79B9 ^ (i as u32) << 8));
        }
        Self {
            params: SynthParams::default(),
            tables: WavetableSet::factory(),
            voices,
            sample_rate: sample_rate.max(8_000.0),
            inv_sample_rate: 1.0 / sample_rate.max(8_000.0),
            age_counter: 0,
            note_counter: 0,
            last_note: 60.0,
            free_lfo1: Lfo::new(0xA511_E9B3),
            free_lfo2: Lfo::new(0x1234_5678),
            master_smooth: 0.8,
        }
    }

    pub fn params(&self) -> &SynthParams {
        &self.params
    }

    pub fn set_params(&mut self, p: SynthParams) {
        self.params = p;
        for v in &mut self.voices {
            v.env1.rebuild_lut(p.env1.curve);
            v.env2.rebuild_lut(p.env2.curve);
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8_000.0);
        self.inv_sample_rate = 1.0 / self.sample_rate;
    }

    pub fn reset_voices(&mut self) {
        for v in &mut self.voices {
            v.active = false;
            v.gate = false;
            v.env1 = Adsr::new();
            v.env2 = Adsr::new();
            v.env1.rebuild_lut(self.params.env1.curve);
            v.env2.rebuild_lut(self.params.env2.curve);
            v.svf1_l = SvfState::default();
            v.svf1_r = SvfState::default();
            v.svf2_l = SvfState::default();
            v.svf2_r = SvfState::default();
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Peak env levels + LFO values for the UI meter packet.
    pub fn meter(&self) -> (f32, f32, f32, f32) {
        let mut env1 = 0.0f32;
        let mut env2 = 0.0f32;
        for v in self.voices.iter().filter(|v| v.active) {
            env1 = env1.max(v.env1.level);
            env2 = env2.max(v.env2.level);
        }
        (
            env1,
            env2,
            self.free_lfo1.value(&self.params.lfo1),
            self.free_lfo2.value(&self.params.lfo2),
        )
    }

    /// Fill `out` with the morphed single-cycle waveform of one oscillator
    /// (for the UI preview). Uses a mid mip so the preview stays smooth.
    pub fn preview_wave(&self, osc_b: bool, out: &mut [f32]) {
        let p = if osc_b {
            &self.params.osc_b
        } else {
            &self.params.osc_a
        };
        let table = self.tables.table(p.table as usize);
        let n = out.len().max(1);
        for (i, v) in out.iter_mut().enumerate() {
            *v = table.sample(i as f32 / n as f32, p.wt_pos, 0, 0.0);
        }
    }

    pub fn note_on(&mut self, key: u8, velocity: f32) {
        self.note_counter = self.note_counter.wrapping_add(1);
        self.age_counter += 1;
        let poly = (self.params.polyphony as usize).clamp(1, MAX_VOICES);

        // Prefer an idle voice; otherwise steal the oldest.
        let slot = {
            let mut idle = None;
            let mut oldest = 0usize;
            let mut oldest_age = u64::MAX;
            let mut active = 0usize;
            for (i, v) in self.voices.iter().enumerate() {
                if v.active {
                    active += 1;
                    if v.age < oldest_age {
                        oldest_age = v.age;
                        oldest = i;
                    }
                } else if idle.is_none() {
                    idle = Some(i);
                }
            }
            match idle {
                Some(i) if active < poly => i,
                _ => oldest,
            }
        };

        let glide = self.params.glide_s > 1.0e-4;
        let start_pitch = if glide { self.last_note } else { key as f32 };
        self.last_note = key as f32;

        let seed = self
            .note_counter
            .wrapping_mul(747796405)
            .wrapping_add(2891336453);
        let mut rng = Rng(seed | 1);

        let v = &mut self.voices[slot];
        v.active = true;
        v.gate = true;
        v.key = key;
        v.velocity = velocity.clamp(0.0, 1.0);
        v.age = self.age_counter;
        v.note_pitch = start_pitch;
        v.glide_target = key as f32;
        v.env1.gate_on();
        v.env2.gate_on();
        if self.params.lfo1.retrig {
            v.lfo1.retrigger(self.params.lfo1.phase);
        }
        if self.params.lfo2.retrig {
            v.lfo2.retrigger(self.params.lfo2.phase);
        }
        v.osc_a.trigger(&self.params.osc_a, &mut rng);
        v.osc_b.trigger(&self.params.osc_b, &mut rng);
    }

    pub fn note_off(&mut self, key: u8) {
        for v in &mut self.voices {
            if v.active && v.gate && v.key == key {
                v.gate = false;
                v.env1.gate_off();
                v.env2.gate_off();
            }
        }
    }

    /// Render additively into `left`/`right` (caller zeroes the buffers).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let mut at = 0usize;
        while at < frames {
            let n = CONTROL_BLOCK.min(frames - at);
            self.render_block(&mut left[at..at + n], &mut right[at..at + n]);
            at += n;
        }
    }

    fn render_block(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        let dt = n as f32 * self.inv_sample_rate;
        let p = self.params;

        // Free-running LFOs advance once per block regardless of voices.
        let free1 = self.free_lfo1.advance(&p.lfo1, dt);
        let free2 = self.free_lfo2.advance(&p.lfo2, dt);

        // Master smoothing toward the (possibly mod-shifted) target happens
        // after voice mods are known; collect the largest master offset.
        let mut master_mod = 0.0f32;

        for vi in 0..self.voices.len() {
            let voice = &mut self.voices[vi];
            if !voice.active {
                continue;
            }

            // ---- control-rate update -------------------------------------
            // Glide slew.
            if p.glide_s > 1.0e-4 {
                let coeff = 1.0 - (-dt / p.glide_s).exp();
                voice.note_pitch += (voice.glide_target - voice.note_pitch) * coeff;
            } else {
                voice.note_pitch = voice.glide_target;
            }

            // Per-voice LFOs (retrig) or the shared free-running values.
            let lfo1 = if p.lfo1.retrig {
                voice.lfo1.advance(&p.lfo1, dt)
            } else {
                free1
            };
            let lfo2 = if p.lfo2.retrig {
                voice.lfo2.advance(&p.lfo2, dt)
            } else {
                free2
            };

            // Mod matrix.
            let mut m = ModOffsets::default();
            for slot in &p.mods {
                if slot.amount == 0.0 {
                    continue;
                }
                let src = match slot.source as usize {
                    SRC_ENV2 => voice.env2.level,
                    SRC_LFO1 => lfo1,
                    SRC_LFO2 => lfo2,
                    SRC_VELOCITY => voice.velocity,
                    SRC_NOTE => (voice.key as f32 - 60.0) / 32.0,
                    _ => continue,
                };
                let x = slot.amount * src;
                match slot.dest as usize {
                    DST_A_WT_POS => m.a_wt += x,
                    DST_A_PITCH => m.a_pitch += x * PITCH_MOD_SEMIS,
                    DST_A_LEVEL => m.a_level += x,
                    DST_A_PAN => m.a_pan += x,
                    DST_B_WT_POS => m.b_wt += x,
                    DST_B_PITCH => m.b_pitch += x * PITCH_MOD_SEMIS,
                    DST_B_LEVEL => m.b_level += x,
                    DST_B_PAN => m.b_pan += x,
                    DST_CUTOFF => m.cutoff_oct += x * CUTOFF_MOD_OCTAVES,
                    DST_RESO => m.reso += x,
                    DST_MASTER => m.master += x,
                    _ => {}
                }
            }
            master_mod = if master_mod.abs() > m.master.abs() {
                master_mod
            } else {
                m.master
            };

            // Effective oscillator settings for this block.
            let a_wt = (p.osc_a.wt_pos + m.a_wt).clamp(0.0, 1.0);
            let b_wt = (p.osc_b.wt_pos + m.b_wt).clamp(0.0, 1.0);
            let a_level = (p.osc_a.level + m.a_level).clamp(0.0, 1.0);
            let b_level = (p.osc_b.level + m.b_level).clamp(0.0, 1.0);
            let a_pan = (p.osc_a.pan + m.a_pan).clamp(-1.0, 1.0);
            let b_pan = (p.osc_b.pan + m.b_pan).clamp(-1.0, 1.0);
            voice.osc_a.update_control(&p.osc_a, a_pan, a_level);
            voice.osc_b.update_control(&p.osc_b, b_pan, b_level);

            let a_semis = voice.note_pitch - 69.0
                + p.osc_a.octave as f32 * 12.0
                + p.osc_a.semi as f32
                + p.osc_a.fine_cents / 100.0
                + m.a_pitch;
            let b_semis = voice.note_pitch - 69.0
                + p.osc_b.octave as f32 * 12.0
                + p.osc_b.semi as f32
                + p.osc_b.fine_cents / 100.0
                + m.b_pitch;
            let a_inc = 440.0 * (a_semis / 12.0).exp2() * self.inv_sample_rate;
            let b_inc = 440.0 * (b_semis / 12.0).exp2() * self.inv_sample_rate;

            // Mip pick: highest unison voice must stay under Nyquist.
            let a_mip = mip_for_increment(a_inc, &p.osc_a);
            let b_mip = mip_for_increment(b_inc, &p.osc_b);

            // Filter coefficients (keytrack shifts cutoff with the note).
            let keytrack_oct = p.keytrack * (voice.note_pitch - 60.0) / 12.0;
            let cutoff = p.cutoff_hz * (m.cutoff_oct + keytrack_oct).exp2();
            let reso = (p.resonance + m.reso).clamp(0.0, 1.0);
            let coeffs = SvfCoeffs::compute(cutoff, reso, self.sample_rate);
            let drive_gain = 1.0 + p.drive * 6.0;
            let drive_comp = 1.0 / (1.0 + p.drive * 1.5);

            // ---- audio-rate render ---------------------------------------
            let table_a = self.tables.table(p.osc_a.table as usize);
            let table_b = self.tables.table(p.osc_b.table as usize);
            let route_a = p.filter_enable && p.route_a;
            let route_b = p.filter_enable && p.route_b;
            let n_uni_a = (p.osc_a.unison as usize).clamp(1, MAX_UNISON);
            let n_uni_b = (p.osc_b.unison as usize).clamp(1, MAX_UNISON);

            for i in 0..n {
                let mut wet_l = 0.0f32;
                let mut wet_r = 0.0f32;
                let mut dry_l = 0.0f32;
                let mut dry_r = 0.0f32;

                if p.osc_a.enable {
                    let mut l = 0.0f32;
                    let mut r = 0.0f32;
                    for u in 0..n_uni_a {
                        let s = table_a.sample(voice.osc_a.phases[u], a_wt, a_mip.0, a_mip.1);
                        l += s * voice.osc_a.gains_l[u];
                        r += s * voice.osc_a.gains_r[u];
                        let ph = voice.osc_a.phases[u] + a_inc * voice.osc_a.ratios[u];
                        voice.osc_a.phases[u] = ph - ph.floor();
                    }
                    if route_a {
                        wet_l += l;
                        wet_r += r;
                    } else {
                        dry_l += l;
                        dry_r += r;
                    }
                }
                if p.osc_b.enable {
                    let mut l = 0.0f32;
                    let mut r = 0.0f32;
                    for u in 0..n_uni_b {
                        let s = table_b.sample(voice.osc_b.phases[u], b_wt, b_mip.0, b_mip.1);
                        l += s * voice.osc_b.gains_l[u];
                        r += s * voice.osc_b.gains_r[u];
                        let ph = voice.osc_b.phases[u] + b_inc * voice.osc_b.ratios[u];
                        voice.osc_b.phases[u] = ph - ph.floor();
                    }
                    if route_b {
                        wet_l += l;
                        wet_r += r;
                    } else {
                        dry_l += l;
                        dry_r += r;
                    }
                }

                let (mut out_l, mut out_r) = (dry_l, dry_r);
                if p.filter_enable {
                    let x_l = soft_clip(wet_l * drive_gain) * drive_comp;
                    let x_r = soft_clip(wet_r * drive_gain) * drive_comp;
                    let (f_l, f_r) = match p.filter_type {
                        0 => {
                            // LP12
                            (
                                voice.svf1_l.tick(x_l, &coeffs).0,
                                voice.svf1_r.tick(x_r, &coeffs).0,
                            )
                        }
                        1 => {
                            // LP24: two cascaded LP12 stages
                            let l1 = voice.svf1_l.tick(x_l, &coeffs).0;
                            let r1 = voice.svf1_r.tick(x_r, &coeffs).0;
                            (
                                voice.svf2_l.tick(l1, &coeffs).0,
                                voice.svf2_r.tick(r1, &coeffs).0,
                            )
                        }
                        2 => {
                            // HP12
                            (
                                voice.svf1_l.tick(x_l, &coeffs).2,
                                voice.svf1_r.tick(x_r, &coeffs).2,
                            )
                        }
                        _ => {
                            // BP12 (band output scaled by k for unity peak)
                            let bl = voice.svf1_l.tick(x_l, &coeffs).1;
                            let br = voice.svf1_r.tick(x_r, &coeffs).1;
                            (bl * coeffs.k, br * coeffs.k)
                        }
                    };
                    out_l += wet_l + (f_l - wet_l) * p.filter_mix;
                    out_r += wet_r + (f_r - wet_r) * p.filter_mix;
                } else {
                    out_l += wet_l;
                    out_r += wet_r;
                }

                let amp = voice.env1.tick(&p.env1, self.inv_sample_rate) * voice.velocity;
                voice.env2.tick(&p.env2, self.inv_sample_rate);
                left[i] += out_l * amp;
                right[i] += out_r * amp;
            }

            voice.svf1_l.flush();
            voice.svf1_r.flush();
            voice.svf2_l.flush();
            voice.svf2_r.flush();

            if voice.env1.is_idle() {
                voice.active = false;
            }
        }

        // Master gain with modulation, smoothed once per block.
        let target = (p.master + master_mod).clamp(0.0, 1.0);
        let smooth_coeff = 1.0 - (-dt / 0.01).exp();
        self.master_smooth += (target - self.master_smooth) * smooth_coeff;
        let g0 = self.master_smooth;
        for i in 0..n {
            left[i] = flush_denormal(left[i] * g0);
            right[i] = flush_denormal(right[i] * g0);
        }
    }
}

/// Pick the mip level whose full harmonic band stays below Nyquist for the
/// most-detuned unison voice — rounded *up*, so playback is strictly
/// alias-free (brightness steps at octave boundaries are the accepted
/// tradeoff for milestone 1; `Wavetable::sample` already supports a
/// crossfade fraction should a blended scheme land later). `inc` is cycles
/// per sample of the center voice.
fn mip_for_increment(inc: f32, p: &OscParams) -> (usize, f32) {
    // Worst-case unison ratio pushes the pitch up by the full spread.
    let worst = inc * (p.uni_detune * UNISON_SPREAD_CENTS / 1200.0).exp2();
    if worst <= 0.0 {
        return (0, 0.0);
    }
    let allowed = 0.5 / worst;
    if allowed >= MAX_HARMONICS as f32 {
        return (0, 0.0);
    }
    let level = (MAX_HARMONICS as f32 / allowed.max(1.0)).log2().ceil();
    ((level.max(0.0) as usize).min(MIPS - 1), 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> SynthEngine {
        SynthEngine::new(48_000.0)
    }

    fn render_seconds(e: &mut SynthEngine, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let frames = (48_000.0 * seconds) as usize;
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        e.render(&mut l, &mut r);
        (l, r)
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn note_on_produces_audio_and_note_off_decays_to_silence() {
        let mut e = engine();
        e.note_on(60, 1.0);
        let (l, _r) = render_seconds(&mut e, 0.2);
        assert!(peak(&l) > 0.01, "note should be audible");
        e.note_off(60);
        // Render past the release tail.
        let _ = render_seconds(&mut e, 1.0);
        let (l2, _r2) = render_seconds(&mut e, 0.1);
        assert!(peak(&l2) < 1.0e-4, "voice must fully release");
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn output_is_always_finite_under_extreme_settings() {
        let mut e = engine();
        let mut p = *e.params();
        p.osc_a.unison = 8;
        p.osc_a.uni_detune = 1.0;
        p.osc_b.enable = true;
        p.osc_b.unison = 8;
        p.cutoff_hz = 20.0;
        p.resonance = 1.0;
        p.drive = 1.0;
        p.filter_type = 1;
        e.set_params(p);
        for k in [21u8, 60, 108, 127] {
            e.note_on(k, 1.0);
        }
        let (l, r) = render_seconds(&mut e, 0.5);
        assert!(l.iter().all(|v| v.is_finite()));
        assert!(r.iter().all(|v| v.is_finite()));
        assert!(peak(&l) < 4.0, "filter must stay bounded");
        assert!(peak(&r) < 4.0);
    }

    #[test]
    fn envelope_reaches_sustain_and_attack_takes_time() {
        let mut e = engine();
        let mut p = *e.params();
        p.env1.attack_s = 0.1;
        p.env1.sustain = 0.5;
        e.set_params(p);
        e.note_on(69, 1.0);
        let (l, _r) = render_seconds(&mut e, 0.02);
        let early = peak(&l);
        let _ = render_seconds(&mut e, 0.5);
        let (l2, _r2) = render_seconds(&mut e, 0.05);
        let sustained = peak(&l2);
        assert!(early < sustained, "attack should still be rising at 20ms");
        assert!(sustained > 0.05);
    }

    #[test]
    fn lfo_to_wt_pos_changes_the_waveform() {
        let render_with_mod = |amount: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.table = 0;
            p.osc_a.wt_pos = 0.5;
            p.filter_enable = false;
            p.mods[0] = ModSlot {
                source: SRC_LFO1 as u8,
                dest: DST_A_WT_POS as u8,
                amount,
            };
            p.lfo1.rate_hz = 8.0;
            e.set_params(p);
            e.note_on(60, 1.0);
            let frames = 24_000;
            let mut l = vec![0.0f32; frames];
            let mut r = vec![0.0f32; frames];
            e.render(&mut l, &mut r);
            l
        };
        let flat = render_with_mod(0.0);
        let wobbled = render_with_mod(0.9);
        let diff: f32 = flat
            .iter()
            .zip(&wobbled)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / flat.len() as f32;
        assert!(diff > 1.0e-4, "wt-pos modulation must alter the signal");
    }

    #[test]
    fn polyphony_limit_steals_voices() {
        let mut e = engine();
        let mut p = *e.params();
        p.polyphony = 2;
        e.set_params(p);
        for k in [60u8, 64, 67, 71] {
            e.note_on(k, 0.8);
        }
        assert!(e.active_voices() <= 4);
        // Render a little; stolen voices must not exceed the polyphony cap
        // by more than the natural release overlap of MAX_VOICES slots.
        let (l, _r) = render_seconds(&mut e, 0.05);
        assert!(peak(&l) > 0.0);
    }

    #[test]
    fn filter_lowpass_darkens_high_notes() {
        let energy_with_cutoff = |cutoff: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.table = 0;
            p.osc_a.wt_pos = 0.5; // saw-ish region
            p.cutoff_hz = cutoff;
            p.resonance = 0.0;
            e.set_params(p);
            e.note_on(84, 1.0); // high C
            let (l, _r) = {
                let frames = 12_000;
                let mut l = vec![0.0f32; frames];
                let mut r = vec![0.0f32; frames];
                e.render(&mut l, &mut r);
                (l, r)
            };
            l.iter().map(|v| (v * v) as f64).sum::<f64>()
        };
        let open = energy_with_cutoff(18_000.0);
        let closed = energy_with_cutoff(200.0);
        assert!(
            closed < open * 0.5,
            "closing the filter must remove energy: open={open} closed={closed}"
        );
    }

    #[test]
    fn unison_widens_stereo_image() {
        let stereo_diff = |unison: u8| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.unison = unison;
            p.osc_a.uni_detune = 0.8;
            p.filter_enable = false;
            e.set_params(p);
            e.note_on(48, 1.0);
            let frames = 24_000;
            let mut l = vec![0.0f32; frames];
            let mut r = vec![0.0f32; frames];
            e.render(&mut l, &mut r);
            l.iter()
                .zip(&r)
                .map(|(a, b)| (a - b).abs() as f64)
                .sum::<f64>()
                / frames as f64
        };
        let mono = stereo_diff(1);
        let wide = stereo_diff(8);
        assert!(
            wide > mono * 4.0,
            "8-voice unison should be far wider: mono={mono} wide={wide}"
        );
    }

    #[test]
    fn high_notes_do_not_alias_above_the_mip_band() {
        // Render a very high saw note and verify energy above the mip's
        // allowed band is tiny compared to the fundamental region.
        let mut e = engine();
        let mut p = *e.params();
        p.osc_a.wt_pos = 0.5;
        p.filter_enable = false;
        p.env1.attack_s = 0.0;
        e.set_params(p);
        e.note_on(120, 1.0); // ~8.4 kHz fundamental at 48 kHz
        let frames = 8192;
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        // Skip the attack transient, then capture a steady window.
        e.render(&mut l, &mut r);
        e.render(&mut l, &mut r);
        // Coarse DFT: compare energy near the fundamental vs a band that
        // would only contain aliases (just below Nyquist, non-harmonic).
        let sr = 48_000.0f64;
        let f0 = 440.0 * ((120.0f64 - 69.0) / 12.0).exp2();
        let goertzel = |freq: f64| {
            let w = core::f64::consts::TAU * freq / sr;
            let coeff = 2.0 * w.cos();
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for &x in &l {
                let s0 = x as f64 + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            let re = s1 - s2 * w.cos();
            let im = s2 * w.sin();
            re * re + im * im
        };
        let fundamental = goertzel(f0);
        // An aliased 2nd harmonic of a naive table would fold to |sr - 2*f0|.
        let alias = goertzel(sr - 2.0 * f0);
        assert!(fundamental > 0.0);
        assert!(
            alias < fundamental * 0.05,
            "alias energy must be far below the fundamental: alias/fund = {}",
            alias / fundamental
        );
    }
}
