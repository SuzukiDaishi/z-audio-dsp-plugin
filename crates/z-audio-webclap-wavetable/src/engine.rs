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
    pub warp_mode: u8,
    pub warp_amount: f32,
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
            warp_mode: W_OFF,
            warp_amount: 0.0,
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
    /// Onset delay before the attack begins (the level is held — on a
    /// stolen/retriggered voice that means the previous ringing level).
    pub delay_s: f32,
    /// Peak hold between attack and decay.
    pub hold_s: f32,
}

impl EnvParams {
    fn default_with(sustain: f32) -> Self {
        Self {
            attack_s: 0.005,
            decay_s: 0.2,
            sustain,
            release_s: 0.15,
            curve: 0.0,
            delay_s: 0.0,
            hold_s: 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LfoParams {
    pub wave: u8,
    pub rate_hz: f32,
    pub phase: f32,
    pub retrig: bool,
    /// Output fades in over this many seconds after (re)trigger.
    pub fade_s: f32,
    /// Run one cycle and hold the final value. With retrig off the free
    /// LFO runs its single cycle once per engine lifetime — a curiosity;
    /// pair one-shot with retrig for the useful mini-envelope behavior.
    pub one_shot: bool,
}

impl Default for LfoParams {
    fn default() -> Self {
        Self {
            wave: 0,
            rate_hz: 2.0,
            phase: 0.0,
            retrig: true,
            fade_s: 0.0,
            one_shot: false,
        }
    }
}

/// Vital-style random modulator settings.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RndParams {
    pub mode: u8,
    pub rate_hz: f32,
    pub retrig: bool,
}

impl Default for RndParams {
    fn default() -> Self {
        Self {
            mode: RND_SH,
            rate_hz: 2.0,
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
    pub dist_enable: bool,
    pub dist_mode: u8,
    pub dist_drive: f32,
    pub dist_mix: f32,
    pub rnd1: RndParams,
    pub rnd2: RndParams,
    /// Velocity mod-source response: -1 soft … 0 linear … 1 hard.
    pub vel_curve: f32,
    /// Note mod-source mapping: `((key - center) / range).clamp(-1, 1)`.
    pub note_center: u8,
    pub note_range: u8,
    /// Vital-style macro knobs — plain 0-1 values exposed as mod sources.
    pub macros: [f32; MACRO_COUNT],
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
            dist_enable: false,
            dist_mode: DIST_TANH,
            dist_drive: 0.3,
            dist_mix: 1.0,
            rnd1: RndParams::default(),
            rnd2: RndParams::default(),
            vel_curve: 0.0,
            note_center: 60,
            note_range: 32,
            macros: [0.0; MACRO_COUNT],
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
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

/// DAHDSR with a shared curvature control. Stage position advances
/// linearly per sample; the curve maps position → level through
/// `x^(2^(3c))`, evaluated via a 257-entry LUT so the audio path never
/// calls `powf`. Delay holds the current level (0 on a fresh voice, the
/// ringing level on a retrigger) before the attack; Hold pins the peak
/// between attack and decay.
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
        // The level is left untouched: a fresh voice sits at 0, a
        // retriggered/stolen voice keeps ringing through the delay.
        self.pos = 0.0;
        self.stage = EnvStage::Delay;
    }

    /// Enter the attack, resuming from the current level so retriggers
    /// don't click (shape is monotonic, a coarse inverse is fine here).
    fn begin_attack(&mut self) {
        self.pos = if self.level > 0.0 {
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

    /// Advance one sample and return the new level. The Delay arm falls
    /// through into Attack within the same tick (via the loop), so a zero
    /// delay is bit-identical to the pre-DAHDSR behavior.
    #[inline]
    fn tick(&mut self, p: &EnvParams, inv_sr: f32) -> f32 {
        loop {
            match self.stage {
                EnvStage::Delay => {
                    if p.delay_s > 1.0e-4 {
                        self.pos += inv_sr / p.delay_s;
                        if self.pos < 1.0 {
                            // Level is held (0 fresh, ringing on retrig).
                            return self.level;
                        }
                    }
                    self.begin_attack();
                    continue; // run Attack in the same tick
                }
                EnvStage::Hold => {
                    self.level = 1.0;
                    self.pos += inv_sr / p.hold_s.max(0.0005);
                    if self.pos >= 1.0 {
                        self.pos = 0.0;
                        self.stage = EnvStage::Decay;
                    }
                }
                _ => {}
            }
            break;
        }
        match self.stage {
            EnvStage::Idle => {
                self.level = 0.0;
            }
            EnvStage::Delay | EnvStage::Hold => {}
            EnvStage::Attack => {
                let step = inv_sr / p.attack_s.max(0.0005);
                self.pos += step;
                if self.pos >= 1.0 {
                    self.level = 1.0;
                    self.pos = 0.0;
                    self.stage = if p.hold_s > 1.0e-4 {
                        EnvStage::Hold
                    } else {
                        EnvStage::Decay
                    };
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
    /// Previous S&H value — the smooth-S&H wave interpolates prev→value.
    sh_prev: f32,
    /// Seconds since the last (re)trigger, drives the fade-in.
    age: f32,
    rng: Rng,
}

impl Lfo {
    fn new(seed: u32) -> Self {
        let mut rng = Rng(seed | 1);
        let sh = rng.next_bipolar();
        Self {
            phase: 0.0,
            sh_value: sh,
            sh_prev: sh,
            age: 0.0,
            rng,
        }
    }

    fn retrigger(&mut self, start_phase: f32) {
        self.phase = start_phase.rem_euclid(1.0);
        self.sh_prev = self.sh_value;
        self.sh_value = self.rng.next_bipolar();
        self.age = 0.0;
    }

    /// Advance by `dt` seconds and return the (faded) bipolar value.
    fn advance(&mut self, p: &LfoParams, dt: f32) -> f32 {
        self.age += dt;
        if p.one_shot {
            // Run one cycle and pin at the end (S&H holds its value).
            self.phase = (self.phase + p.rate_hz.max(0.01) * dt).min(1.0);
        } else {
            self.phase += p.rate_hz.max(0.01) * dt;
            if self.phase >= 1.0 {
                self.phase -= self.phase.floor();
                self.sh_prev = self.sh_value;
                self.sh_value = self.rng.next_bipolar();
            }
        }
        self.output(p)
    }

    #[inline]
    fn fade_gain(&self, p: &LfoParams) -> f32 {
        if p.fade_s > 1.0e-4 {
            (self.age / p.fade_s).min(1.0)
        } else {
            1.0
        }
    }

    /// Current faded output (no state change) — shared by `advance` and
    /// the UI meter.
    fn output(&self, p: &LfoParams) -> f32 {
        self.value(p) * self.fade_gain(p)
    }

    fn value(&self, p: &LfoParams) -> f32 {
        let x = self.phase.min(1.0);
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
            4 => self.sh_value,
            5 => 1.0 - 2.0 * x, // ramp down
            6 => {
                // pulse 25%
                if x < 0.25 {
                    1.0
                } else {
                    -1.0
                }
            }
            7 => {
                // smooth S&H: cosine interpolation prev → current
                let t = 0.5 - 0.5 * (core::f32::consts::PI * x).cos();
                self.sh_prev + (self.sh_value - self.sh_prev) * t
            }
            _ => self.sh_value,
        }
    }
}

// ---------------------------------------------------------------------------
// Random modulator (Vital-style)
// ---------------------------------------------------------------------------

/// Random mod source with four flavors: stepped S&H, cosine-smoothed
/// targets, Perlin-ish drift (two smoothed streams at related rates), and
/// a Lorenz-attractor chaos mode. Deterministic per xorshift stream, like
/// everything else in the engine.
#[derive(Clone, Copy)]
struct Rnd {
    phase: f32,
    prev: f32,
    target: f32,
    // Drift's secondary stream at 2.7× the rate.
    phase2: f32,
    prev2: f32,
    target2: f32,
    // Lorenz state for the chaos mode.
    lx: f32,
    ly: f32,
    lz: f32,
    rng: Rng,
}

impl Rnd {
    fn new(seed: u32) -> Self {
        let mut rng = Rng(seed | 1);
        let a = rng.next_bipolar();
        let b = rng.next_bipolar();
        let c = rng.next_bipolar();
        let d = rng.next_bipolar();
        Self {
            phase: 0.0,
            prev: a,
            target: b,
            phase2: 0.0,
            prev2: c,
            target2: d,
            lx: 0.1,
            ly: 0.0,
            lz: 25.0,
            rng: rng,
        }
    }

    fn retrigger(&mut self) {
        self.phase = 0.0;
        self.prev = self.target;
        self.target = self.rng.next_bipolar();
        self.phase2 = 0.0;
        self.prev2 = self.target2;
        self.target2 = self.rng.next_bipolar();
        // Perturb (deterministically) so every note's chaos differs.
        self.lx = 0.1 + 0.05 * self.rng.next_bipolar();
        self.ly = 0.0;
        self.lz = 25.0;
    }

    /// Advance by `dt` seconds and return the bipolar value.
    fn advance(&mut self, p: &RndParams, dt: f32) -> f32 {
        let rate = p.rate_hz.max(0.01);
        match p.mode {
            RND_CHAOS => {
                // Lorenz (σ=10, ρ=28, β=8/3); the rate scales integration
                // speed. Substeps cap the Euler dt for stability.
                let h = rate * dt * 0.4;
                let n = (h / 0.005).ceil().clamp(1.0, 16.0) as usize;
                let hs = h / n as f32;
                for _ in 0..n {
                    let dx = 10.0 * (self.ly - self.lx);
                    let dy = self.lx * (28.0 - self.lz) - self.ly;
                    let dz = self.lx * self.ly - (8.0 / 3.0) * self.lz;
                    self.lx = (self.lx + dx * hs).clamp(-60.0, 60.0);
                    self.ly = (self.ly + dy * hs).clamp(-60.0, 60.0);
                    self.lz = (self.lz + dz * hs).clamp(0.0, 80.0);
                }
            }
            RND_DRIFT => {
                self.phase += rate * dt;
                if self.phase >= 1.0 {
                    self.phase -= self.phase.floor();
                    self.prev = self.target;
                    self.target = self.rng.next_bipolar();
                }
                self.phase2 += rate * 2.7 * dt;
                if self.phase2 >= 1.0 {
                    self.phase2 -= self.phase2.floor();
                    self.prev2 = self.target2;
                    self.target2 = self.rng.next_bipolar();
                }
            }
            _ => {
                // S&H and Smooth share the single-stream accumulator.
                self.phase += rate * dt;
                if self.phase >= 1.0 {
                    self.phase -= self.phase.floor();
                    self.prev = self.target;
                    self.target = self.rng.next_bipolar();
                }
            }
        }
        self.value(p)
    }

    /// Current output (no state change) — shared with the UI meter.
    fn value(&self, p: &RndParams) -> f32 {
        let smooth = |prev: f32, target: f32, x: f32| {
            let t = 0.5 - 0.5 * (core::f32::consts::PI * x).cos();
            prev + (target - prev) * t
        };
        match p.mode {
            RND_SH => self.target,
            RND_SMOOTH => smooth(self.prev, self.target, self.phase),
            RND_DRIFT => {
                0.65 * smooth(self.prev, self.target, self.phase)
                    + 0.35 * smooth(self.prev2, self.target2, self.phase2)
            }
            _ => (self.lx / 20.0).clamp(-1.0, 1.0),
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
// Oscillator warp (Serum-style)
// ---------------------------------------------------------------------------

/// Per-block warp constants. Phase warps (`phase`) remap each unison
/// voice's phase before the table lookup; RM/AM (`post_gain`) scale the
/// oscillator's summed sample instead. All per-sample math is algebraic —
/// no transcendentals in the audio path.
#[derive(Clone, Copy)]
struct WarpKernel {
    mode: u8,
    a: f32,
    k1: f32,
    k2: f32,
}

impl WarpKernel {
    fn new(mode: u8, amount: f32) -> Self {
        let a = amount.clamp(0.0, 1.0);
        let (k1, k2) = match mode {
            W_SYNC => (1.0 + 7.0 * a, 0.0),
            W_SQUEEZE => (1.0 / (1.0 - 0.95 * a), 0.0),
            W_QUANTIZE => {
                let n = (64.0 - 62.0 * a).round().max(2.0);
                (n, 1.0 / n)
            }
            W_FM => (0.5 * a, 0.0),
            _ => (0.0, 0.0),
        };
        Self {
            mode: if a <= 0.0 { W_OFF } else { mode },
            a,
            k1,
            k2,
        }
    }

    /// Warp a phase in [0,1). `other` is the 1-sample-delayed raw sample
    /// of the opposite oscillator (the FM modulator).
    #[inline]
    fn phase(&self, p: f32, other: f32) -> f32 {
        match self.mode {
            W_BEND_P => p + self.a * (p * p - p),
            W_BEND_M => p + self.a * (p - p * p),
            W_SYNC => {
                let x = p * self.k1;
                x - x.floor()
            }
            W_MIRROR => {
                let pm = if p < 0.5 { 2.0 * p } else { 2.0 * (1.0 - p) };
                p + self.a * (pm - p)
            }
            W_SQUEEZE => (p * self.k1).min(0.999_999),
            W_QUANTIZE => (p * self.k1).floor() * self.k2,
            W_FM => {
                let x = p + self.k1 * other;
                x - x.floor()
            }
            _ => p,
        }
    }

    /// Post-lookup gain for the RM/AM modes, 1.0 otherwise.
    #[inline]
    fn post_gain(&self, other: f32) -> f32 {
        match self.mode {
            W_RM => 1.0 + self.a * (other - 1.0),
            W_AM => (1.0 + self.a * other) / (1.0 + self.a),
            _ => 1.0,
        }
    }
}

/// One distortion transfer function on a pre-gained sample. Crush is
/// handled separately (it needs the sample-hold state).
#[inline]
fn distort(mode: u8, x: f32) -> f32 {
    match mode {
        DIST_HARD => x.clamp(-1.0, 1.0),
        DIST_FOLD => {
            // Triangle foldback: identity in [-1,1], reflects beyond.
            let g = x - 4.0 * ((x + 2.0) * 0.25).floor();
            if g.abs() > 1.0 {
                g.signum() * (2.0 - g.abs())
            } else {
                g
            }
        }
        // Master bus only, so a real sin() per sample is affordable.
        DIST_SINE => (core::f32::consts::FRAC_PI_2 * x.clamp(-3.0, 3.0)).sin(),
        _ => soft_clip(x),
    }
}

/// Extra octaves of bandwidth a warp adds — biases the mip pick down so
/// phase warps stay (approximately) alias-free. Quantize/RM/AM return 0:
/// their aliasing is intentional grit, tested only for boundedness.
fn warp_octaves(mode: u8, amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    if a <= 0.0 {
        return 0.0;
    }
    match mode {
        W_SYNC => (1.0 + 7.0 * a).log2(),
        W_BEND_P | W_BEND_M => (1.0 + a).log2(),
        W_MIRROR => a,
        W_SQUEEZE => (1.0 / (1.0 - 0.95 * a)).log2(),
        W_FM => 1.5 * a,
        _ => 0.0,
    }
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
    rnd1: Rnd,
    rnd2: Rnd,
    osc_a: UnisonOsc,
    osc_b: UnisonOsc,
    /// 1-sample-delayed raw table samples (unison voice 0, pre-gain) —
    /// the FM/RM/AM modulator taps. The z⁻¹ makes A→B, B→A and mutual
    /// cross-modulation all well-defined regardless of render order.
    fm_a_prev: f32,
    fm_b_prev: f32,
    svf1_l: SvfState,
    svf1_r: SvfState,
    svf2_l: SvfState,
    svf2_r: SvfState,
    /// Third SVF pair — the Formant filter runs three parallel bands.
    svf3_l: SvfState,
    svf3_r: SvfState,
    /// Cutoff-tracked feedback comb delay lines (sized in
    /// `set_sample_rate` for a 20 Hz floor; ~2.4k samples/ch at 48 kHz).
    comb_l: Vec<f32>,
    comb_r: Vec<f32>,
    comb_pos: usize,
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
            rnd1: Rnd::new(seed.wrapping_mul(2246822519).wrapping_add(3)),
            rnd2: Rnd::new(seed.wrapping_mul(3266489917).wrapping_add(11)),
            osc_a: UnisonOsc::new(),
            osc_b: UnisonOsc::new(),
            fm_a_prev: 0.0,
            fm_b_prev: 0.0,
            svf1_l: SvfState::default(),
            svf1_r: SvfState::default(),
            svf2_l: SvfState::default(),
            svf2_r: SvfState::default(),
            svf3_l: SvfState::default(),
            svf3_r: SvfState::default(),
            comb_l: Vec::new(),
            comb_r: Vec::new(),
            comb_pos: 0,
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
    a_warp: f32,
    b_warp: f32,
    dist_drive: f32,
    a_det: f32,
    b_det: f32,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct SynthEngine {
    params: SynthParams,
    tables: std::sync::Arc<WavetableSet>,
    voices: Vec<Voice>,
    sample_rate: f32,
    inv_sample_rate: f32,
    age_counter: u64,
    note_counter: u32,
    last_note: f32,
    /// Free-running LFO phases shared by voices with retrig off.
    free_lfo1: Lfo,
    free_lfo2: Lfo,
    /// Free-running random modulators for retrig-off voices.
    free_rnd1: Rnd,
    free_rnd2: Rnd,
    /// Most recent note-on velocity/key — the UI meter's VELO/NOTE tiles.
    last_velocity: f32,
    last_key: u8,
    /// Smoothed master gain (one-pole per control block).
    master_smooth: f32,
    /// Sample-hold state for the Crush distortion mode.
    decim_counter: u32,
    decim_hold: (f32, f32),
}

/// One frame of live modulation values for the UI meter packet.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct MeterFrame {
    pub env1: f32,
    pub env2: f32,
    pub lfo1: f32,
    pub lfo2: f32,
    pub rnd1: f32,
    pub rnd2: f32,
    pub velocity: f32,
    pub note: f32,
}

impl SynthEngine {
    pub fn new(sample_rate: f32) -> Self {
        let mut voices = Vec::with_capacity(MAX_VOICES);
        for i in 0..MAX_VOICES {
            voices.push(Voice::new(0x9E37_79B9 ^ (i as u32) << 8));
        }
        let mut e = Self {
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
            free_rnd1: Rnd::new(0xC0FF_EE01),
            free_rnd2: Rnd::new(0xBADD_CAFE),
            last_velocity: 0.0,
            last_key: 60,
            master_smooth: 0.8,
            decim_counter: 0,
            decim_hold: (0.0, 0.0),
        };
        e.resize_combs();
        e
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
        self.resize_combs();
    }

    /// (Re)size every voice's comb delay line for a 20 Hz floor at the
    /// current sample rate.
    fn resize_combs(&mut self) {
        let len = (self.sample_rate / 20.0).ceil() as usize + 4;
        for v in &mut self.voices {
            if v.comb_l.len() != len {
                v.comb_l = vec![0.0; len];
                v.comb_r = vec![0.0; len];
                v.comb_pos = 0;
            }
        }
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
            v.svf3_l = SvfState::default();
            v.svf3_r = SvfState::default();
            v.comb_l.fill(0.0);
            v.comb_r.fill(0.0);
            v.comb_pos = 0;
            v.fm_a_prev = 0.0;
            v.fm_b_prev = 0.0;
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Live modulation values for the UI meter packet.
    pub fn meter(&self) -> MeterFrame {
        let mut env1 = 0.0f32;
        let mut env2 = 0.0f32;
        for v in self.voices.iter().filter(|v| v.active) {
            env1 = env1.max(v.env1.level);
            env2 = env2.max(v.env2.level);
        }
        let p = &self.params;
        MeterFrame {
            env1,
            env2,
            lfo1: self.free_lfo1.output(&p.lfo1),
            lfo2: self.free_lfo2.output(&p.lfo2),
            rnd1: self.free_rnd1.value(&p.rnd1),
            rnd2: self.free_rnd2.value(&p.rnd2),
            velocity: self.last_velocity,
            note: ((self.last_key as f32 - p.note_center as f32)
                / p.note_range.max(1) as f32)
                .clamp(-1.0, 1.0),
        }
    }

    /// Fill `out` with the morphed single-cycle waveform of one oscillator
    /// at its current WT position (for the UI preview).
    pub fn preview_wave(&self, osc_b: bool, out: &mut [f32]) {
        let pos = if osc_b {
            self.params.osc_b.wt_pos
        } else {
            self.params.osc_a.wt_pos
        };
        self.preview_wave_at(osc_b, pos, out);
    }

    /// Fill `out` with one oscillator's single cycle at an arbitrary WT
    /// position — the UI's pseudo-3D stack view samples every frame.
    /// Phase warps show up in the preview; FM/RM/AM need the other
    /// oscillator's live signal, so they draw unwarped.
    pub fn preview_wave_at(&self, osc_b: bool, pos: f32, out: &mut [f32]) {
        let p = if osc_b {
            &self.params.osc_b
        } else {
            &self.params.osc_a
        };
        let table = self.tables.table(p.table as usize);
        let kernel = if p.warp_mode <= W_QUANTIZE {
            WarpKernel::new(p.warp_mode, p.warp_amount)
        } else {
            WarpKernel::new(W_OFF, 0.0)
        };
        let n = out.len().max(1);
        for (i, v) in out.iter_mut().enumerate() {
            *v = table.sample(kernel.phase(i as f32 / n as f32, 0.0), pos, 0, 0.0);
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
        if self.params.rnd1.retrig {
            v.rnd1.retrigger();
        }
        if self.params.rnd2.retrig {
            v.rnd2.retrigger();
        }
        self.last_velocity = v.velocity;
        self.last_key = key;
        v.osc_a.trigger(&self.params.osc_a, &mut rng);
        v.osc_b.trigger(&self.params.osc_b, &mut rng);
        v.fm_a_prev = 0.0;
        v.fm_b_prev = 0.0;
        // Stolen voices must not ring with the previous note's comb tail.
        v.comb_l.fill(0.0);
        v.comb_r.fill(0.0);
        v.comb_pos = 0;
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

        // Free-running LFOs/randoms advance once per block regardless of
        // voices.
        let free1 = self.free_lfo1.advance(&p.lfo1, dt);
        let free2 = self.free_lfo2.advance(&p.lfo2, dt);
        let free_r1 = self.free_rnd1.advance(&p.rnd1, dt);
        let free_r2 = self.free_rnd2.advance(&p.rnd2, dt);

        // Master smoothing toward the (possibly mod-shifted) target happens
        // after voice mods are known; collect the largest master offset.
        // Distortion drive follows the same max-abs pattern (it is a bus
        // effect, so per-voice offsets collapse to one value per block).
        let mut master_mod = 0.0f32;
        let mut dist_mod = 0.0f32;

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
            let rnd1 = if p.rnd1.retrig {
                voice.rnd1.advance(&p.rnd1, dt)
            } else {
                free_r1
            };
            let rnd2 = if p.rnd2.retrig {
                voice.rnd2.advance(&p.rnd2, dt)
            } else {
                free_r2
            };
            // Shaped velocity (curve 0 bypasses powf and stays bit-exact
            // with the pre-expansion behavior) and centered note source.
            let vel_src = if p.vel_curve == 0.0 {
                voice.velocity
            } else {
                voice.velocity.powf((3.0 * p.vel_curve).exp2())
            };
            let note_src = ((voice.key as f32 - p.note_center as f32)
                / p.note_range.max(1) as f32)
                .clamp(-1.0, 1.0);

            // Mod matrix.
            let mut m = ModOffsets::default();
            for slot in &p.mods {
                if slot.amount == 0.0 {
                    continue;
                }
                let src = match slot.source as usize {
                    SRC_ENV1 => voice.env1.level,
                    SRC_ENV2 => voice.env2.level,
                    SRC_LFO1 => lfo1,
                    SRC_LFO2 => lfo2,
                    SRC_VELOCITY => vel_src,
                    SRC_NOTE => note_src,
                    SRC_RND1 => rnd1,
                    SRC_RND2 => rnd2,
                    s @ SRC_MACRO1..=SRC_MACRO4 => p.macros[s - SRC_MACRO1],
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
                    DST_A_WARP => m.a_warp += x,
                    DST_B_WARP => m.b_warp += x,
                    DST_DIST_DRIVE => m.dist_drive += x,
                    DST_A_UNI_DET => m.a_det += x,
                    DST_B_UNI_DET => m.b_det += x,
                    _ => {}
                }
            }
            master_mod = if master_mod.abs() > m.master.abs() {
                master_mod
            } else {
                m.master
            };
            dist_mod = if dist_mod.abs() > m.dist_drive.abs() {
                dist_mod
            } else {
                m.dist_drive
            };

            // Effective oscillator settings for this block: start from the
            // params and fold in the modulated warp amount / unison detune
            // (the copies also feed `mip_for_increment`, keeping the alias
            // guarantee under modulation).
            let mut oa = p.osc_a;
            let mut ob = p.osc_b;
            oa.warp_amount = (oa.warp_amount + m.a_warp).clamp(0.0, 1.0);
            ob.warp_amount = (ob.warp_amount + m.b_warp).clamp(0.0, 1.0);
            oa.uni_detune = (oa.uni_detune + m.a_det).clamp(0.0, 1.0);
            ob.uni_detune = (ob.uni_detune + m.b_det).clamp(0.0, 1.0);
            let a_kernel = WarpKernel::new(oa.warp_mode, oa.warp_amount);
            let b_kernel = WarpKernel::new(ob.warp_mode, ob.warp_amount);

            let a_wt = (oa.wt_pos + m.a_wt).clamp(0.0, 1.0);
            let b_wt = (ob.wt_pos + m.b_wt).clamp(0.0, 1.0);
            let a_level = (oa.level + m.a_level).clamp(0.0, 1.0);
            let b_level = (ob.level + m.b_level).clamp(0.0, 1.0);
            let a_pan = (oa.pan + m.a_pan).clamp(-1.0, 1.0);
            let b_pan = (ob.pan + m.b_pan).clamp(-1.0, 1.0);
            voice.osc_a.update_control(&oa, a_pan, a_level);
            voice.osc_b.update_control(&ob, b_pan, b_level);

            let a_semis = voice.note_pitch - 69.0
                + oa.octave as f32 * 12.0
                + oa.semi as f32
                + oa.fine_cents / 100.0
                + m.a_pitch;
            let b_semis = voice.note_pitch - 69.0
                + ob.octave as f32 * 12.0
                + ob.semi as f32
                + ob.fine_cents / 100.0
                + m.b_pitch;
            let a_inc = 440.0 * (a_semis / 12.0).exp2() * self.inv_sample_rate;
            let b_inc = 440.0 * (b_semis / 12.0).exp2() * self.inv_sample_rate;

            // Mip pick: highest unison voice must stay under Nyquist.
            let a_mip = mip_for_increment(a_inc, &oa);
            let b_mip = mip_for_increment(b_inc, &ob);

            // Filter coefficients (keytrack shifts cutoff with the note).
            let keytrack_oct = p.keytrack * (voice.note_pitch - 60.0) / 12.0;
            let cutoff = p.cutoff_hz * (m.cutoff_oct + keytrack_oct).exp2();
            let reso = (p.resonance + m.reso).clamp(0.0, 1.0);
            let coeffs = SvfCoeffs::compute(cutoff, reso, self.sample_rate);
            let drive_gain = 1.0 + p.drive * 6.0;
            let drive_comp = 1.0 / (1.0 + p.drive * 1.5);

            // Comb: cutoff-tracked fractional delay, resonance sets the
            // feedback. Stable for |fb| < 1; comp keeps loudness in check.
            let comb_len = voice.comb_l.len().max(4);
            let is_comb = p.filter_type == FT_COMB_P || p.filter_type == FT_COMB_M;
            let (comb_delay, comb_fb, comb_comp) = if is_comb {
                let d = (self.sample_rate / cutoff.max(20.0)).clamp(2.0, (comb_len - 2) as f32);
                let mag = 0.50 + 0.48 * reso;
                let fb = if p.filter_type == FT_COMB_M { -mag } else { mag };
                (d, fb, 1.0 - 0.5 * mag)
            } else {
                (0.0, 0.0, 1.0)
            };
            // Formant: the cutoff knob's log position picks the vowel
            // (200 Hz = "a" … 4 kHz = "u"), three parallel band-passes.
            let formant_coeffs = if p.filter_type == FT_FORMANT {
                let t = (cutoff.max(1.0).ln() - 200.0f32.ln())
                    / (4000.0f32.ln() - 200.0f32.ln());
                let (freqs, _bws) = crate::wavetable::vowel_at(t.clamp(0.0, 1.0));
                [
                    SvfCoeffs::compute(freqs[0], reso, self.sample_rate),
                    SvfCoeffs::compute(freqs[1], reso, self.sample_rate),
                    SvfCoeffs::compute(freqs[2], reso, self.sample_rate),
                ]
            } else {
                [coeffs; 3]
            };

            // ---- audio-rate render ---------------------------------------
            let table_a = self.tables.table(oa.table as usize);
            let table_b = self.tables.table(ob.table as usize);
            let route_a = p.filter_enable && p.route_a;
            let route_b = p.filter_enable && p.route_b;
            let n_uni_a = (oa.unison as usize).clamp(1, MAX_UNISON);
            let n_uni_b = (ob.unison as usize).clamp(1, MAX_UNISON);

            for i in 0..n {
                let mut wet_l = 0.0f32;
                let mut wet_r = 0.0f32;
                let mut dry_l = 0.0f32;
                let mut dry_r = 0.0f32;
                // Raw unison-voice-0 samples become next sample's FM taps.
                let mut raw_a = 0.0f32;
                let mut raw_b = 0.0f32;

                if oa.enable {
                    let mut l = 0.0f32;
                    let mut r = 0.0f32;
                    for u in 0..n_uni_a {
                        let ph = a_kernel.phase(voice.osc_a.phases[u], voice.fm_b_prev);
                        let s = table_a.sample(ph, a_wt, a_mip.0, a_mip.1);
                        if u == 0 {
                            raw_a = s;
                        }
                        l += s * voice.osc_a.gains_l[u];
                        r += s * voice.osc_a.gains_r[u];
                        let ph = voice.osc_a.phases[u] + a_inc * voice.osc_a.ratios[u];
                        voice.osc_a.phases[u] = ph - ph.floor();
                    }
                    let g = a_kernel.post_gain(voice.fm_b_prev);
                    l *= g;
                    r *= g;
                    if route_a {
                        wet_l += l;
                        wet_r += r;
                    } else {
                        dry_l += l;
                        dry_r += r;
                    }
                }
                if ob.enable {
                    let mut l = 0.0f32;
                    let mut r = 0.0f32;
                    for u in 0..n_uni_b {
                        let ph = b_kernel.phase(voice.osc_b.phases[u], voice.fm_a_prev);
                        let s = table_b.sample(ph, b_wt, b_mip.0, b_mip.1);
                        if u == 0 {
                            raw_b = s;
                        }
                        l += s * voice.osc_b.gains_l[u];
                        r += s * voice.osc_b.gains_r[u];
                        let ph = voice.osc_b.phases[u] + b_inc * voice.osc_b.ratios[u];
                        voice.osc_b.phases[u] = ph - ph.floor();
                    }
                    let g = b_kernel.post_gain(voice.fm_a_prev);
                    l *= g;
                    r *= g;
                    if route_b {
                        wet_l += l;
                        wet_r += r;
                    } else {
                        dry_l += l;
                        dry_r += r;
                    }
                }
                voice.fm_a_prev = raw_a;
                voice.fm_b_prev = raw_b;

                let (mut out_l, mut out_r) = (dry_l, dry_r);
                if p.filter_enable {
                    let x_l = soft_clip(wet_l * drive_gain) * drive_comp;
                    let x_r = soft_clip(wet_r * drive_gain) * drive_comp;
                    let (f_l, f_r) = match p.filter_type {
                        FT_LP12 => (
                            voice.svf1_l.tick(x_l, &coeffs).0,
                            voice.svf1_r.tick(x_r, &coeffs).0,
                        ),
                        FT_LP24 => {
                            // Two cascaded LP12 stages.
                            let l1 = voice.svf1_l.tick(x_l, &coeffs).0;
                            let r1 = voice.svf1_r.tick(x_r, &coeffs).0;
                            (
                                voice.svf2_l.tick(l1, &coeffs).0,
                                voice.svf2_r.tick(r1, &coeffs).0,
                            )
                        }
                        FT_HP12 => (
                            voice.svf1_l.tick(x_l, &coeffs).2,
                            voice.svf1_r.tick(x_r, &coeffs).2,
                        ),
                        FT_BP12 => {
                            // Band output scaled by k for unity peak.
                            let bl = voice.svf1_l.tick(x_l, &coeffs).1;
                            let br = voice.svf1_r.tick(x_r, &coeffs).1;
                            (bl * coeffs.k, br * coeffs.k)
                        }
                        FT_NOTCH12 => {
                            let (ll, _, hl) = voice.svf1_l.tick(x_l, &coeffs);
                            let (lr, _, hr) = voice.svf1_r.tick(x_r, &coeffs);
                            (ll + hl, lr + hr)
                        }
                        FT_COMB_P | FT_COMB_M => {
                            // y[n] = x[n] ± fb·y[n-d], fractional read.
                            let rp = voice.comb_pos as f32 - comb_delay;
                            let rp = if rp < 0.0 { rp + comb_len as f32 } else { rp };
                            let i0 = (rp as usize).min(comb_len - 1);
                            let i1 = if i0 + 1 >= comb_len { 0 } else { i0 + 1 };
                            let fr = rp - i0 as f32;
                            let dl = voice.comb_l[i0] + (voice.comb_l[i1] - voice.comb_l[i0]) * fr;
                            let dr = voice.comb_r[i0] + (voice.comb_r[i1] - voice.comb_r[i0]) * fr;
                            let yl = x_l + comb_fb * dl;
                            let yr = x_r + comb_fb * dr;
                            voice.comb_l[voice.comb_pos] = flush_denormal(yl);
                            voice.comb_r[voice.comb_pos] = flush_denormal(yr);
                            voice.comb_pos += 1;
                            if voice.comb_pos >= comb_len {
                                voice.comb_pos = 0;
                            }
                            (yl * comb_comp, yr * comb_comp)
                        }
                        _ => {
                            // Formant: three parallel band-passes at the
                            // interpolated vowel's F1/F2/F3.
                            use crate::wavetable::VOWEL_AMPS;
                            let c = &formant_coeffs;
                            let fl = voice.svf1_l.tick(x_l, &c[0]).1 * c[0].k * VOWEL_AMPS[0]
                                + voice.svf2_l.tick(x_l, &c[1]).1 * c[1].k * VOWEL_AMPS[1]
                                + voice.svf3_l.tick(x_l, &c[2]).1 * c[2].k * VOWEL_AMPS[2];
                            let fr = voice.svf1_r.tick(x_r, &c[0]).1 * c[0].k * VOWEL_AMPS[0]
                                + voice.svf2_r.tick(x_r, &c[1]).1 * c[1].k * VOWEL_AMPS[1]
                                + voice.svf3_r.tick(x_r, &c[2]).1 * c[2].k * VOWEL_AMPS[2];
                            (fl, fr)
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
            voice.svf3_l.flush();
            voice.svf3_r.flush();

            if voice.env1.is_idle() {
                voice.active = false;
            }
        }

        // Global distortion: voice sum → shaper → master gain.
        if p.dist_enable {
            let drive = (p.dist_drive + dist_mod).clamp(0.0, 1.0);
            let gain = 1.0 + drive * 11.0;
            let comp = 1.0 / (1.0 + drive * 2.0);
            let mix = p.dist_mix.clamp(0.0, 1.0);
            if p.dist_mode == DIST_CRUSH {
                // Sample-rate divide: hold every Nth sample (~750 Hz at
                // 48 kHz and full drive).
                let hold = 1 + (drive * 63.0).round() as u32;
                for i in 0..n {
                    if self.decim_counter == 0 {
                        self.decim_hold = (left[i], right[i]);
                    }
                    self.decim_counter = (self.decim_counter + 1) % hold;
                    left[i] += (self.decim_hold.0 - left[i]) * mix;
                    right[i] += (self.decim_hold.1 - right[i]) * mix;
                }
            } else {
                for i in 0..n {
                    let y_l = distort(p.dist_mode, left[i] * gain) * comp;
                    let y_r = distort(p.dist_mode, right[i] * gain) * comp;
                    left[i] += (y_l - left[i]) * mix;
                    right[i] += (y_r - right[i]) * mix;
                }
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
    // Worst-case unison ratio pushes the pitch up by the full spread, and
    // a hot warp widens the spectrum by a few more octaves.
    let worst = inc
        * (p.uni_detune * UNISON_SPREAD_CENTS / 1200.0).exp2()
        * warp_octaves(p.warp_mode, p.warp_amount).exp2();
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

    /// Goertzel power at `freq` Hz over `buf` at 48 kHz.
    fn goertzel(buf: &[f32], freq: f64) -> f64 {
        let w = core::f64::consts::TAU * freq / 48_000.0;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in buf {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let re = s1 - s2 * w.cos();
        let im = s2 * w.sin();
        re * re + im * im
    }

    #[test]
    fn warp_modes_stay_finite() {
        for mode in 0..WARP_MODE_COUNT as u8 {
            for amount in [0.3f32, 1.0] {
                let mut e = engine();
                let mut p = *e.params();
                p.osc_a.warp_mode = mode;
                p.osc_a.warp_amount = amount;
                p.osc_a.unison = 4;
                p.osc_b.enable = true;
                p.osc_b.warp_mode = mode;
                p.osc_b.warp_amount = amount;
                e.set_params(p);
                for k in [24u8, 60, 108] {
                    e.note_on(k, 1.0);
                }
                let (l, r) = render_seconds(&mut e, 0.25);
                assert!(
                    l.iter().chain(r.iter()).all(|v| v.is_finite()),
                    "mode {mode} amount {amount} produced non-finite output"
                );
                assert!(peak(&l) < 8.0, "mode {mode} amount {amount} exploded");
            }
        }
    }

    #[test]
    fn warp_amount_zero_matches_warp_off() {
        let render = |mode: u8| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.warp_mode = mode;
            p.osc_a.warp_amount = 0.0;
            p.filter_enable = false;
            e.set_params(p);
            e.note_on(48, 1.0);
            render_seconds(&mut e, 0.1).0
        };
        let off = render(W_OFF);
        for mode in 1..WARP_MODE_COUNT as u8 {
            assert_eq!(off, render(mode), "mode {mode} at amount 0 must be a no-op");
        }
    }

    #[test]
    fn warp_biases_the_mip_choice_down() {
        let mut p = OscParams::default_with(true);
        p.uni_detune = 0.0;
        let inc = 440.0 / 48_000.0;
        let base = mip_for_increment(inc, &p);
        p.warp_mode = W_SYNC;
        p.warp_amount = 1.0;
        let synced = mip_for_increment(inc, &p);
        assert!(
            synced.0 >= base.0 + 3,
            "full sync spans 3 octaves: base {} synced {}",
            base.0,
            synced.0
        );
    }

    #[test]
    fn sync_warp_does_not_alias() {
        // Sync at r=2 is exact transposition (the table is periodic across
        // the wrap), so the mip bias must keep it strictly alias-free even
        // at a very high note: no energy where the folded 2nd harmonic of
        // the doubled pitch would land.
        let mut e = engine();
        let mut p = *e.params();
        p.osc_a.wt_pos = 0.5; // saw region
        p.osc_a.uni_detune = 0.0;
        p.osc_a.warp_mode = W_SYNC;
        p.osc_a.warp_amount = 1.0 / 7.0; // r = 2 exactly
        p.filter_enable = false;
        p.env1.attack_s = 0.0;
        e.set_params(p);
        e.note_on(120, 1.0); // f0 ≈ 8372 Hz → warped pitch ≈ 16744 Hz
        let frames = 8192;
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        e.render(&mut l, &mut r);
        e.render(&mut l, &mut r);
        let f0 = 2.0 * 440.0 * ((120.0f64 - 69.0) / 12.0).exp2();
        let fundamental = goertzel(&l, f0);
        let alias = goertzel(&l, 48_000.0 - 2.0 * f0);
        assert!(fundamental > 0.0);
        assert!(
            alias < fundamental * 0.05,
            "sync alias/fund = {}",
            alias / fundamental
        );
    }

    #[test]
    fn fm_warp_needs_an_enabled_modulator() {
        let render = |b_enable: bool, amount: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.warp_mode = W_FM;
            p.osc_a.warp_amount = amount;
            p.osc_a.rand_phase = 0.0;
            p.osc_b.enable = b_enable;
            p.osc_b.level = 0.0; // silent, but the pre-gain tap still works
            p.osc_b.rand_phase = 0.0;
            p.filter_enable = false;
            e.set_params(p);
            e.note_on(48, 1.0);
            render_seconds(&mut e, 0.1).0
        };
        // A silent-but-enabled osc B drives FM…
        let flat = render(true, 0.0);
        let modulated = render(true, 0.8);
        let diff: f32 =
            flat.iter().zip(&modulated).map(|(a, b)| (a - b).abs()).sum::<f32>() / flat.len() as f32;
        assert!(diff > 1.0e-4, "FM from a silent osc B must alter the signal");
        // …while a disabled osc B leaves the tap at zero (no-op).
        assert_eq!(render(false, 0.0), render(false, 0.8));
    }

    #[test]
    fn comb_filter_bounded_at_max_feedback() {
        let mut e = engine();
        let mut p = *e.params();
        p.filter_type = FT_COMB_P;
        p.cutoff_hz = 220.0;
        p.resonance = 1.0;
        e.set_params(p);
        e.note_on(45, 1.0);
        let (l, r) = render_seconds(&mut e, 2.0);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
        assert!(peak(&l) < 4.0, "comb must stay bounded at max feedback");
    }

    #[test]
    fn comb_boosts_its_tuned_frequency() {
        // A 110 Hz saw through a comb tuned to 220 Hz: the 220 Hz partial
        // is reinforced while 110 Hz sits in a feedback null, so the
        // 110:220 power ratio must collapse vs the unfiltered render.
        let ratio_with_filter = |enable: bool| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.wt_pos = 0.5; // saw region
            p.filter_enable = enable;
            p.filter_type = FT_COMB_P;
            p.cutoff_hz = 220.0;
            p.resonance = 0.7;
            p.env1.attack_s = 0.0;
            e.set_params(p);
            e.note_on(45, 1.0); // A2 = 110 Hz
            let frames = 16_384;
            let mut l = vec![0.0f32; frames];
            let mut r = vec![0.0f32; frames];
            e.render(&mut l, &mut r);
            e.render(&mut l, &mut r);
            goertzel(&l, 110.0) / goertzel(&l, 220.0).max(1.0e-12)
        };
        let dry = ratio_with_filter(false);
        let combed = ratio_with_filter(true);
        assert!(
            combed < dry * 0.5,
            "comb must favor 220 Hz over 110 Hz: dry={dry} combed={combed}"
        );
    }

    #[test]
    fn notch_removes_the_cutoff_band() {
        let energy_at = |enable: bool| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.wt_pos = 0.5;
            p.filter_enable = enable;
            p.filter_type = FT_NOTCH12;
            p.cutoff_hz = 440.0;
            p.resonance = 0.8; // narrow notch
            p.env1.attack_s = 0.0;
            e.set_params(p);
            e.note_on(45, 1.0); // 110 Hz — h4 sits on the notch
            let frames = 16_384;
            let mut l = vec![0.0f32; frames];
            let mut r = vec![0.0f32; frames];
            e.render(&mut l, &mut r);
            e.render(&mut l, &mut r);
            (goertzel(&l, 440.0), goertzel(&l, 110.0))
        };
        let (dry_notched, dry_fund) = energy_at(false);
        let (wet_notched, wet_fund) = energy_at(true);
        assert!(
            wet_notched < dry_notched * 0.1,
            "notch must cut 440 Hz: {wet_notched} vs {dry_notched}"
        );
        assert!(
            wet_fund > dry_fund * 0.3,
            "notch must pass the fundamental: {wet_fund} vs {dry_fund}"
        );
    }

    #[test]
    fn formant_filter_shapes_vowel_peaks() {
        // Cutoff 200 Hz → vowel "a" (F1 = 730 Hz). A 55 Hz saw's partial
        // near F1 (h13 ≈ 715 Hz) must tower over one far above F3
        // (h90 ≈ 4950 Hz) much more than in the dry signal.
        let ratio = |enable: bool| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.wt_pos = 0.5;
            p.filter_enable = enable;
            p.filter_type = FT_FORMANT;
            p.cutoff_hz = 200.0;
            p.resonance = 0.5;
            p.env1.attack_s = 0.0;
            e.set_params(p);
            e.note_on(33, 1.0); // A1 = 55 Hz
            let frames = 16_384;
            let mut l = vec![0.0f32; frames];
            let mut r = vec![0.0f32; frames];
            e.render(&mut l, &mut r);
            e.render(&mut l, &mut r);
            goertzel(&l, 55.0 * 13.0) / goertzel(&l, 55.0 * 90.0).max(1.0e-12)
        };
        let dry = ratio(false);
        let wet = ratio(true);
        assert!(
            wet > dry * 10.0,
            "formant must emphasize F1 over the far band: dry={dry} wet={wet}"
        );
    }

    #[test]
    fn distortion_modes_are_finite_and_bounded() {
        for mode in 0..DIST_MODE_COUNT as u8 {
            let mut e = engine();
            let mut p = *e.params();
            p.dist_enable = true;
            p.dist_mode = mode;
            p.dist_drive = 1.0;
            p.osc_a.unison = 8;
            p.osc_b.enable = true;
            e.set_params(p);
            for k in [24u8, 60, 96] {
                e.note_on(k, 1.0);
            }
            let (l, r) = render_seconds(&mut e, 0.25);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
            assert!(peak(&l) < 4.0, "dist mode {mode} must stay bounded");
        }
    }

    #[test]
    fn distortion_mix_zero_is_transparent() {
        let render = |enable: bool| {
            let mut e = engine();
            let mut p = *e.params();
            p.dist_enable = enable;
            p.dist_mode = DIST_FOLD;
            p.dist_drive = 1.0;
            p.dist_mix = 0.0;
            e.set_params(p);
            e.note_on(48, 1.0);
            render_seconds(&mut e, 0.1).0
        };
        assert_eq!(render(false), render(true));
    }

    #[test]
    fn crush_holds_samples() {
        let mut e = engine();
        let mut p = *e.params();
        p.dist_enable = true;
        p.dist_mode = DIST_CRUSH;
        p.dist_drive = 1.0; // hold = 64 samples
        e.set_params(p);
        e.note_on(48, 1.0);
        // Let the master-gain smoothing settle, then inspect a window.
        let _ = render_seconds(&mut e, 0.3);
        let (l, _r) = render_seconds(&mut e, 0.1);
        let equal_pairs = l.windows(2).filter(|w| w[0] == w[1]).count();
        assert!(
            equal_pairs as f32 > l.len() as f32 * 0.9,
            "crush must hold samples: {} / {}",
            equal_pairs,
            l.len()
        );
    }

    #[test]
    fn detune_mod_dest_widens_stereo() {
        let stereo_diff = |amount: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.osc_a.unison = 8;
            p.osc_a.uni_detune = 0.0;
            p.filter_enable = false;
            p.mods[0] = ModSlot {
                source: SRC_VELOCITY as u8,
                dest: DST_A_UNI_DET as u8,
                amount,
            };
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
        let flat = stereo_diff(0.0);
        let wide = stereo_diff(0.8);
        assert!(
            wide > flat * 4.0,
            "detune modulation should widen the image: flat={flat} wide={wide}"
        );
    }

    #[test]
    fn env_delay_delays_onset() {
        let mut e = engine();
        let mut p = *e.params();
        p.env1.delay_s = 0.2;
        p.env1.attack_s = 0.0;
        e.set_params(p);
        e.note_on(60, 1.0);
        let (early, _) = render_seconds(&mut e, 0.15);
        assert!(peak(&early) < 1.0e-4, "audio must be silent during delay");
        let (later, _) = render_seconds(&mut e, 0.2);
        assert!(peak(&later) > 0.01, "audio must start after the delay");
    }

    #[test]
    fn env_hold_holds_peak() {
        let level_at_200ms = |hold_s: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.env1.attack_s = 0.0;
            p.env1.hold_s = hold_s;
            p.env1.decay_s = 0.02;
            p.env1.sustain = 0.0;
            e.set_params(p);
            e.note_on(60, 1.0);
            let _ = render_seconds(&mut e, 0.18);
            let (l, _) = render_seconds(&mut e, 0.04);
            peak(&l)
        };
        assert!(level_at_200ms(0.3) > 0.1, "hold must keep the peak alive");
        assert!(level_at_200ms(0.0) < 1.0e-3, "no hold: decayed by 200 ms");
    }

    #[test]
    fn env_delay_zero_is_transparent() {
        // Same param set with and without explicitly-zero delay/hold must
        // render bit-identically (the DAHDSR regression guard).
        let render = || {
            let mut e = engine();
            e.note_on(48, 0.9);
            render_seconds(&mut e, 0.2).0
        };
        let a = render();
        let b = render();
        assert_eq!(a, b);
    }

    #[test]
    fn env_retrigger_during_ring_is_continuous() {
        // Unit-level: gate on, run, gate off, ring down a little, retrigger
        // with a delay — the level must hold during Delay (no jump to 0).
        let p = EnvParams {
            attack_s: 0.05,
            decay_s: 0.2,
            sustain: 0.7,
            release_s: 0.5,
            curve: 0.0,
            delay_s: 0.05,
            hold_s: 0.0,
        };
        let inv_sr = 1.0 / 48_000.0;
        let mut env = Adsr::new();
        env.gate_on();
        for _ in 0..4800 {
            env.tick(&p, inv_sr);
        }
        env.gate_off();
        for _ in 0..2400 {
            env.tick(&p, inv_sr);
        }
        let ring = env.level;
        assert!(ring > 0.01);
        env.gate_on();
        // During the retrigger delay the level must stay put.
        for _ in 0..1200 {
            let l = env.tick(&p, inv_sr);
            assert!(
                (l - ring).abs() < 1.0e-5,
                "delay must hold the ringing level: {l} vs {ring}"
            );
        }
    }

    #[test]
    fn lfo_fade_ramps() {
        // Tremolo depth = spread of per-chunk RMS. With a 2 s fade the
        // depth must grow between the first and last half-second.
        let tremolo_depth = |window: core::ops::Range<usize>, buf: &[f32]| {
            let chunks: Vec<f64> = buf[window]
                .chunks(1024)
                .map(|c| (c.iter().map(|v| (v * v) as f64).sum::<f64>() / c.len() as f64).sqrt())
                .collect();
            let lo = chunks.iter().cloned().fold(f64::MAX, f64::min);
            let hi = chunks.iter().cloned().fold(0.0f64, f64::max);
            hi - lo
        };
        let mut e = engine();
        let mut p = *e.params();
        p.lfo1.rate_hz = 8.0;
        p.lfo1.fade_s = 3.0;
        p.filter_enable = false;
        p.env1.attack_s = 0.0;
        p.mods[0] = ModSlot {
            source: SRC_LFO1 as u8,
            dest: DST_A_LEVEL as u8,
            amount: 0.7,
        };
        e.set_params(p);
        e.note_on(48, 1.0);
        let frames = 96_000; // 2 s
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        e.render(&mut l, &mut r);
        let early = tremolo_depth(4_800..28_800, &l);
        let late = tremolo_depth(72_000..96_000, &l);
        assert!(
            late > early * 2.0,
            "fade-in should deepen the LFO over time: early={early} late={late}"
        );
    }

    #[test]
    fn lfo_one_shot_stops() {
        let p = LfoParams {
            wave: 2, // saw up
            rate_hz: 4.0,
            phase: 0.0,
            retrig: true,
            fade_s: 0.0,
            one_shot: true,
        };
        let mut lfo = Lfo::new(1);
        lfo.retrigger(0.0);
        let mut last = 0.0;
        for _ in 0..2000 {
            last = lfo.advance(&p, 0.001); // 2 s total, period 0.25 s
        }
        assert!((last - 1.0).abs() < 1.0e-6, "saw one-shot pins at 1.0");
        // S&H one-shot holds a constant.
        let p_sh = LfoParams { wave: 4, ..p };
        let mut sh = Lfo::new(7);
        sh.retrigger(0.0);
        let first = sh.advance(&p_sh, 0.5);
        for _ in 0..100 {
            assert_eq!(sh.advance(&p_sh, 0.5), first);
        }
    }

    #[test]
    fn lfo_new_waves_bounded_and_smooth_sh_is_continuous() {
        for wave in [5u8, 6, 7] {
            let p = LfoParams {
                wave,
                rate_hz: 3.0,
                ..LfoParams::default()
            };
            let mut lfo = Lfo::new(11 + wave as u32);
            let mut prev = lfo.advance(&p, 0.0007);
            for _ in 0..10_000 {
                let v = lfo.advance(&p, 0.0007);
                assert!(v.is_finite() && v.abs() <= 1.0, "wave {wave} out of range");
                if wave == 7 {
                    assert!(
                        (v - prev).abs() < 0.1,
                        "smooth S&H must be continuous: {prev} -> {v}"
                    );
                }
                prev = v;
            }
        }
    }

    #[test]
    fn random_modes_finite_distinct_deterministic() {
        let sequence = |mode: u8, seed: u32| -> Vec<f32> {
            let p = RndParams {
                mode,
                rate_hz: 6.0,
                retrig: true,
            };
            let mut rnd = Rnd::new(seed);
            (0..400).map(|_| rnd.advance(&p, 0.002)).collect()
        };
        let mut outputs = Vec::new();
        for mode in 0..RND_MODE_COUNT as u8 {
            let a = sequence(mode, 42);
            let b = sequence(mode, 42);
            assert_eq!(a, b, "mode {mode} must be deterministic");
            assert!(a.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
            assert!(
                a.iter().any(|v| v.abs() > 1.0e-3),
                "mode {mode} must actually move"
            );
            outputs.push(a);
        }
        for i in 0..outputs.len() {
            for j in i + 1..outputs.len() {
                assert_ne!(outputs[i], outputs[j], "modes {i} and {j} identical");
            }
        }
    }

    #[test]
    fn chaos_stays_bounded_at_max_rate() {
        let p = RndParams {
            mode: RND_CHAOS,
            rate_hz: 50.0,
            retrig: true,
        };
        let mut rnd = Rnd::new(99);
        let dt = 32.0 / 48_000.0;
        for _ in 0..100_000 {
            let v = rnd.advance(&p, dt);
            assert!(v.is_finite() && v.abs() <= 1.0);
        }
    }

    #[test]
    fn random_source_modulates_the_engine() {
        let mut e = engine();
        let mut p = *e.params();
        p.filter_enable = false;
        p.rnd1.mode = RND_SMOOTH;
        p.rnd1.rate_hz = 12.0;
        p.mods[0] = ModSlot {
            source: SRC_RND1 as u8,
            dest: DST_A_LEVEL as u8,
            amount: 1.0,
        };
        e.set_params(p);
        e.note_on(48, 1.0);
        let (with_mod, _) = render_seconds(&mut e, 0.5);
        let mut e2 = engine();
        let mut p2 = *e2.params();
        p2.filter_enable = false;
        e2.set_params(p2);
        e2.note_on(48, 1.0);
        let (without, _) = render_seconds(&mut e2, 0.5);
        let diff: f32 = with_mod
            .iter()
            .zip(&without)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / without.len() as f32;
        assert!(diff > 1.0e-4, "random source must alter the render");
    }

    #[test]
    fn macro_source_drives_the_matrix() {
        let peak_with_macro = |value: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.filter_enable = false;
            p.macros[0] = value;
            // Full macro pulls the default 0.75 level down to silence.
            p.mods[0] = ModSlot {
                source: SRC_MACRO1 as u8,
                dest: DST_A_LEVEL as u8,
                amount: -0.75,
            };
            e.set_params(p);
            e.note_on(60, 1.0);
            let _ = render_seconds(&mut e, 0.05);
            let (l, _) = render_seconds(&mut e, 0.1);
            peak(&l)
        };
        assert!(peak_with_macro(0.0) > 0.05, "macro at 0 must be inert");
        assert!(
            peak_with_macro(1.0) < 0.02,
            "macro at 1 must apply its full amount"
        );
    }

    #[test]
    fn vel_curve_shapes_velocity_source() {
        let rms = |curve: f32| {
            let mut e = engine();
            let mut p = *e.params();
            p.filter_enable = false;
            p.vel_curve = curve;
            p.mods[0] = ModSlot {
                source: SRC_VELOCITY as u8,
                dest: DST_A_LEVEL as u8,
                amount: -0.74, // pulls the 0.75 default level down by velocity
            };
            e.set_params(p);
            e.note_on(60, 0.5);
            let _ = render_seconds(&mut e, 0.1);
            let (l, _) = render_seconds(&mut e, 0.2);
            (l.iter().map(|v| (v * v) as f64).sum::<f64>() / l.len() as f64).sqrt()
        };
        // Soft curve boosts a mid velocity (more negative mod → quieter);
        // hard curve shrinks it (less mod → louder).
        let soft = rms(-1.0);
        let hard = rms(1.0);
        assert!(
            hard > soft * 1.2,
            "curve must reshape the velocity source: soft={soft} hard={hard}"
        );
    }

    #[test]
    fn note_center_shifts_the_source_sign() {
        let pitch_offset = |center: f32, key: u8| {
            let mut e = engine();
            let mut p = *e.params();
            p.filter_enable = false;
            p.note_center = center as u8;
            p.note_range = 12;
            p.env1.attack_s = 0.0;
            p.mods[0] = ModSlot {
                source: SRC_NOTE as u8,
                dest: DST_A_PITCH as u8,
                amount: 1.0,
            };
            e.set_params(p);
            e.note_on(key, 1.0);
            let frames = 8192;
            let mut l = vec![0.0f32; frames];
            let mut r = vec![0.0f32; frames];
            e.render(&mut l, &mut r);
            e.render(&mut l, &mut r);
            // Dominant frequency vs the unmodulated pitch tells the sign.
            let f0 = 440.0 * ((key as f64 - 69.0) / 12.0).exp2();
            let up = goertzel(&l, f0 * 2.0); // +12 semitone shift target
            let down = goertzel(&l, f0 * 0.5);
            up.partial_cmp(&down).unwrap()
        };
        // Key one octave above center → source +1 → pitch up dominates.
        assert_eq!(pitch_offset(48.0, 60), core::cmp::Ordering::Greater);
        // Same key below center 72 → source -1 → pitch down dominates.
        assert_eq!(pitch_offset(72.0, 60), core::cmp::Ordering::Less);
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
