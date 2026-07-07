//! Phase Plant-style granular synthesis engine.
//!
//! One source sample, MIDI-note-triggered grain streams. Each active note
//! (voice) schedules grains around the automatable **Position** play
//! cursor; every grain is an independent windowed slice of the source with
//! its own pitch, direction, level and pan, summed under a per-voice amp
//! ADSR.
//!
//! All randomness comes from a per-voice xorshift32 PRNG seeded from a
//! note-on counter, so renders are deterministic and unit-testable.
//!
//! Design notes:
//! - Grain amplitude windows use two shared 257-entry lookup tables
//!   (attack / decay with adjustable curvature), rebuilt lazily when the
//!   curve parameters change — no per-sample `powf`.
//! - Linear sample interpolation; per-grain level×pan gains are baked at
//!   spawn time. Both keep the wasm per-sample budget flat at the
//!   [`MAX_ACTIVE_GRAINS`] worst case.
//! - Grain edges always get a tiny minimum fade (`MIN_EDGE_FRAMES`) so
//!   zero attack/decay settings cannot click.

use z_audio_dsp::flush_denormal;

use crate::params::SYNC_BEATS;

/// Simultaneous MIDI notes.
pub const MAX_VOICES: usize = 16;
/// Grains one voice can sustain.
pub const MAX_GRAINS_PER_VOICE: usize = 64;
/// Global budget across all voices; excess spawns steal the most-finished
/// grain in their own voice.
pub const MAX_ACTIVE_GRAINS: usize = 256;
/// Minimum grain fade edge (frames) so hard envelope settings never click.
const MIN_EDGE_FRAMES: u32 = 16;
/// Release stage ends (voice freed) when the amp envelope falls below this.
const RELEASE_END_LEVEL: f32 = 1.0e-4;
/// Entries in each grain-envelope lookup table (+1 for the endpoint).
const LUT_SIZE: usize = 257;

/// Chord interval tables, indexed by the Chord Type parameter:
/// Off · Octave · Fifth · Major · Minor · Maj7 · Min7 · Dom7 · Sus2 · Sus4.
const CHORD_TABLES: [&[i32]; 10] = [
    &[0],
    &[0, 12],
    &[0, 7],
    &[0, 4, 7],
    &[0, 3, 7],
    &[0, 4, 7, 11],
    &[0, 3, 7, 10],
    &[0, 4, 7, 10],
    &[0, 2, 7],
    &[0, 5, 7],
];

/// All engine parameters, mirrored 1:1 by the CLAP param surface
/// (`crate::params`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GranularParams {
    pub level: f32,
    pub pitch_semitones: f32,
    pub fine_cents: f32,
    /// The seek bar: 0..1 across the loaded sample.
    pub position: f32,
    pub grain_length_ms: f32,
    pub length_keytrack: bool,
    /// Attack / decay as fractions of the grain length.
    pub grain_attack: f32,
    pub grain_decay: f32,
    pub attack_curve: f32,
    pub decay_curve: f32,
    /// 0 free (Hz) · 1 tempo sync · 2 density (target overlap).
    pub spawn_mode: u8,
    pub rate_hz: f32,
    /// Index into [`SYNC_BEATS`].
    pub sync_index: u8,
    pub density: f32,
    pub root_note: u8,
    pub align_phases: bool,
    pub warm_start: bool,
    pub random_position_ms: f32,
    pub random_timing: f32,
    pub random_pitch: f32,
    pub random_level: f32,
    pub random_pan: f32,
    /// Probability that a grain plays reversed.
    pub random_reverse: f32,
    pub chord_type: u8,
    pub chord_range: u8,
    pub chord_pattern: u8,
    pub amp_attack_s: f32,
    pub amp_decay_s: f32,
    pub amp_sustain: f32,
    pub amp_release_s: f32,
}

impl Default for GranularParams {
    fn default() -> Self {
        // Defaults match `crate::params::param_defs()` (pinned by test).
        Self {
            level: 1.0,
            pitch_semitones: 0.0,
            fine_cents: 0.0,
            position: 0.0,
            grain_length_ms: 100.0,
            length_keytrack: false,
            grain_attack: 0.5,
            grain_decay: 0.5,
            attack_curve: 0.0,
            decay_curve: 0.0,
            spawn_mode: 0,
            rate_hz: 25.0,
            sync_index: 4,
            density: 8.0,
            root_note: 60,
            align_phases: false,
            warm_start: false,
            random_position_ms: 0.0,
            random_timing: 0.0,
            random_pitch: 0.0,
            random_level: 0.0,
            random_pan: 0.0,
            random_reverse: 0.0,
            chord_type: 0,
            chord_range: 1,
            chord_pattern: 0,
            amp_attack_s: 0.002,
            amp_decay_s: 0.0,
            amp_sustain: 1.0,
            amp_release_s: 0.25,
        }
    }
}

/// The uploaded (or natively decoded) source sample.
pub struct SourceSample {
    pub sample_rate: f32,
    pub channels: u8,
    /// Interleaved when stereo.
    pub data: Vec<f32>,
}

impl SourceSample {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
}

/// An in-progress chunked PCM upload (protocol `BeginSample`/`SampleChunk`).
struct Upload {
    sample_rate: f32,
    channels: u8,
    data: Vec<f32>,
}

/// Tiny xorshift32; `uniform` uses the top 24 bits for a clean 0..1 float.
#[derive(Clone, Copy)]
struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    fn new(seed: u32) -> Self {
        Self {
            state: seed | 1, // never zero
        }
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    fn uniform(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }

    fn bipolar(&mut self) -> f32 {
        self.uniform() * 2.0 - 1.0
    }
}

#[derive(Clone, Copy)]
struct Grain {
    active: bool,
    /// Read head in source frames; advanced by `step` per output sample.
    pos: f64,
    /// Source frames per output sample; negative plays reversed.
    step: f64,
    /// Output samples rendered so far.
    age: u32,
    /// Total lifetime in output samples.
    dur: u32,
    /// Attack ends at this age; decay starts at `decay_start`.
    attack_end: u32,
    decay_start: u32,
    /// Intra-block offset for grains spawned mid-block; reset after use.
    block_start: u32,
    /// Level × constant-power pan, baked at spawn.
    gain_l: f32,
    gain_r: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            active: false,
            pos: 0.0,
            step: 0.0,
            age: 0,
            dur: 0,
            attack_end: 0,
            decay_start: 0,
            block_start: 0,
            gain_l: 0.0,
            gain_r: 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

struct Voice {
    active: bool,
    released: bool,
    key: u8,
    velocity: f32,
    /// Monotonic note-on ordinal, for oldest-voice stealing.
    serial: u32,
    env_stage: EnvStage,
    env_level: f32,
    /// Output samples until the next grain spawn (may span blocks).
    spawn_countdown: f64,
    chord_cursor: i32,
    chord_dir: i32,
    rng: XorShift32,
    grains: [Grain; MAX_GRAINS_PER_VOICE],
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            released: false,
            key: 0,
            velocity: 0.0,
            serial: 0,
            env_stage: EnvStage::Idle,
            env_level: 0.0,
            spawn_countdown: 0.0,
            chord_cursor: 0,
            chord_dir: 1,
            rng: XorShift32::new(1),
            grains: [Grain::default(); MAX_GRAINS_PER_VOICE],
        }
    }
}

impl Voice {
    fn active_grain_count(&self) -> usize {
        self.grains.iter().filter(|g| g.active).count()
    }

    /// One amp-envelope sample. Linear attack/decay, exponential release.
    fn env_next(&mut self, atk_step: f32, dec_step: f32, sustain: f32, rel_coeff: f32) -> f32 {
        match self.env_stage {
            EnvStage::Idle => self.env_level = 0.0,
            EnvStage::Attack => {
                self.env_level += atk_step;
                if self.env_level >= 1.0 {
                    self.env_level = 1.0;
                    self.env_stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                if self.env_level <= sustain || dec_step == f32::INFINITY {
                    self.env_level = sustain;
                    self.env_stage = EnvStage::Sustain;
                } else {
                    self.env_level = (self.env_level - dec_step).max(sustain);
                }
            }
            EnvStage::Sustain => self.env_level = sustain,
            EnvStage::Release => {
                self.env_level *= rel_coeff;
                if self.env_level <= RELEASE_END_LEVEL {
                    self.env_level = 0.0;
                    self.env_stage = EnvStage::Idle;
                }
            }
        }
        self.env_level
    }
}

pub struct GranularEngine {
    sample_rate: f32,
    tempo_bpm: f64,
    params: GranularParams,
    source: Option<SourceSample>,
    upload: Option<Upload>,
    voices: Vec<Voice>,
    /// Monotonic note-on counter: voice serials + deterministic RNG seeds.
    note_serial: u32,
    /// Lifetime spawn counter (test/diagnostic aid).
    spawned_grains: u64,
    atk_lut: [f32; LUT_SIZE],
    dec_lut: [f32; LUT_SIZE],
    lut_curves: (f32, f32),
    lut_dirty: bool,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
}

impl GranularEngine {
    pub fn new(sample_rate: f32) -> Self {
        let mut engine = Self {
            sample_rate: sample_rate.max(1.0),
            tempo_bpm: 120.0,
            params: GranularParams::default(),
            source: None,
            upload: None,
            voices: (0..MAX_VOICES).map(|_| Voice::default()).collect(),
            note_serial: 0,
            spawned_grains: 0,
            atk_lut: [0.0; LUT_SIZE],
            dec_lut: [0.0; LUT_SIZE],
            lut_curves: (0.0, 0.0),
            lut_dirty: true,
            scratch_l: Vec::new(),
            scratch_r: Vec::new(),
        };
        engine.rebuild_luts();
        engine
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.reset_voices();
    }

    /// Host tempo for Sync spawn mode (the WebCLAP build has no transport
    /// and leaves this at the 120 BPM default).
    pub fn set_tempo(&mut self, bpm: f64) {
        if bpm.is_finite() && bpm > 0.0 {
            self.tempo_bpm = bpm;
        }
    }

    /// Drops all active voices (host reset), keeping the sample.
    pub fn reset_voices(&mut self) {
        for voice in &mut self.voices {
            *voice = Voice::default();
        }
    }

    pub fn params(&self) -> &GranularParams {
        &self.params
    }

    pub fn set_params(&mut self, params: GranularParams) {
        if params != self.params {
            if (params.attack_curve, params.decay_curve) != self.lut_curves {
                self.lut_dirty = true;
            }
            self.params = params;
        }
    }

    pub fn has_sample(&self) -> bool {
        self.source.is_some()
    }

    pub fn source_info(&self) -> (u32, f32, u8) {
        match &self.source {
            Some(s) => (s.frames() as u32, s.sample_rate, s.channels),
            None => (0, 0.0, 0),
        }
    }

    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    pub fn active_grain_count(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.active)
            .map(Voice::active_grain_count)
            .sum()
    }

    /// Lifetime number of grains ever spawned (diagnostics/tests).
    pub fn spawned_grain_count(&self) -> u64 {
        self.spawned_grains
    }

    /// Pre-sizes the per-voice scratch buffers for the host's maximum
    /// block size so `render` never allocates on the audio thread.
    pub fn reserve_block(&mut self, max_frames: usize) {
        if self.scratch_l.len() < max_frames {
            self.scratch_l.resize(max_frames, 0.0);
            self.scratch_r.resize(max_frames, 0.0);
        }
    }

    /// Fills `out` with normalized (0..1) positions of active grains and
    /// returns how many were written — the UI's grain-activity display.
    pub fn grain_positions(&self, out: &mut [f32]) -> usize {
        let Some(source) = &self.source else {
            return 0;
        };
        let frames = source.frames().max(2) as f64;
        let mut n = 0;
        for voice in self.voices.iter().filter(|v| v.active) {
            for grain in voice.grains.iter().filter(|g| g.active) {
                if n >= out.len() {
                    return n;
                }
                out[n] = (grain.pos / (frames - 1.0)).clamp(0.0, 1.0) as f32;
                n += 1;
            }
        }
        n
    }

    /// Installs a new source sample, replacing any previous one. Live
    /// grains whose read heads fall outside the new sample end themselves
    /// on the next block.
    pub fn set_source(&mut self, source: SourceSample) {
        self.source = Some(source);
    }

    pub fn clear(&mut self) {
        self.source = None;
        self.upload = None;
        self.reset_voices();
    }

    // -- chunked upload ---------------------------------------------------

    pub fn begin_upload(&mut self, sample_rate: f32, channels: u8, frames: u32) {
        self.upload = Some(Upload {
            sample_rate,
            channels,
            data: vec![0.0; frames as usize * channels.max(1) as usize],
        });
    }

    pub fn upload_chunk(&mut self, float_offset: u32, pcm_bytes: &[u8]) {
        let Some(upload) = self.upload.as_mut() else {
            return;
        };
        let offset = float_offset as usize;
        let count = pcm_bytes.len() / 4;
        let Some(dst) = upload.data.get_mut(offset..offset.saturating_add(count)) else {
            // Out-of-bounds chunk: drop the whole transfer rather than
            // committing a sample with silent holes.
            self.upload = None;
            return;
        };
        for (i, out) in dst.iter_mut().enumerate() {
            let b = &pcm_bytes[i * 4..i * 4 + 4];
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            *out = if v.is_finite() {
                v.clamp(-4.0, 4.0)
            } else {
                0.0
            };
        }
    }

    /// The pending upload (if any survived) becomes the new source.
    pub fn commit_sample(&mut self) {
        if let Some(upload) = self.upload.take() {
            self.set_source(SourceSample {
                sample_rate: upload.sample_rate,
                channels: upload.channels,
                data: upload.data,
            });
        }
    }

    // -- voices -------------------------------------------------------------

    pub fn note_on(&mut self, key: u8, velocity01: f32) {
        let key = key & 0x7f;
        let vi = self.alloc_voice();
        self.note_serial = self.note_serial.wrapping_add(1);
        let params = self.params;
        let seed = self
            .note_serial
            .wrapping_mul(2_654_435_761)
            .wrapping_add(key as u32);

        let voice = &mut self.voices[vi];
        *voice = Voice {
            active: true,
            released: false,
            key,
            velocity: velocity01.clamp(0.0, 1.0),
            serial: self.note_serial,
            env_stage: EnvStage::Attack,
            env_level: 0.0,
            spawn_countdown: 0.0,
            chord_cursor: match params.chord_pattern {
                1 => chord_offset_count(&params) - 1, // Down starts at the top
                _ => 0,
            },
            chord_dir: 1,
            rng: XorShift32::new(seed),
            grains: [Grain::default(); MAX_GRAINS_PER_VOICE],
        };

        // Warm start: pre-fill the grains that would already be sounding
        // had the stream been running, so the note opens at steady state
        // instead of ramping in over one grain length.
        if params.warm_start && self.source.is_some() {
            let interval = self.spawn_interval_base(key);
            let dur = self.grain_dur_frames(key);
            let count = ((dur / interval) as usize).min(MAX_GRAINS_PER_VOICE - 1);
            let mut total = self.count_active_grains();
            let (Some(source), Some(voice)) = (self.source.as_ref(), self.voices.get_mut(vi))
            else {
                return;
            };
            for k in 1..=count {
                let age = (k as f64 * interval) as u32;
                spawn_grain(
                    voice,
                    &params,
                    self.sample_rate,
                    source,
                    0,
                    age,
                    &mut total,
                    &mut self.spawned_grains,
                );
            }
        }
    }

    pub fn note_off(&mut self, key: u8) {
        let key = key & 0x7f;
        for voice in &mut self.voices {
            if voice.active && !voice.released && voice.key == key {
                voice.released = true;
                voice.env_stage = EnvStage::Release;
            }
        }
    }

    fn alloc_voice(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // Steal: quietest released voice, else the oldest.
        let released = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.released)
            .min_by(|a, b| a.1.env_level.total_cmp(&b.1.env_level))
            .map(|(i, _)| i);
        released.unwrap_or_else(|| {
            self.voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.serial)
                .map(|(i, _)| i)
                .unwrap_or(0)
        })
    }

    fn count_active_grains(&self) -> usize {
        self.voices.iter().map(Voice::active_grain_count).sum()
    }

    // -- scheduling ---------------------------------------------------------

    /// Un-jittered spawn interval in output samples for `key`.
    fn spawn_interval_base(&self, key: u8) -> f64 {
        let p = &self.params;
        let sr = self.sample_rate as f64;
        let interval = match p.spawn_mode {
            1 => {
                let beats = SYNC_BEATS[p.sync_index.min(8) as usize];
                sr * (60.0 / self.tempo_bpm.max(1.0)) * beats
            }
            2 => self.grain_dur_frames(key) / p.density.clamp(0.5, 64.0) as f64,
            _ => sr / p.rate_hz.clamp(0.1, 400.0) as f64,
        };
        interval.max(1.0)
    }

    /// Grain lifetime in output samples for `key` (keytracked if enabled).
    fn grain_dur_frames(&self, key: u8) -> f64 {
        let p = &self.params;
        let mut dur =
            p.grain_length_ms.clamp(2.0, 1000.0) as f64 * self.sample_rate as f64 / 1000.0;
        if p.length_keytrack {
            dur *= (-((key as f64) - (p.root_note as f64)) / 12.0).exp2();
        }
        dur.clamp(2.0, self.sample_rate as f64 * 10.0)
    }

    // -- envelope LUTs ------------------------------------------------------

    fn rebuild_luts(&mut self) {
        let ac = self.params.attack_curve.clamp(-1.0, 1.0);
        let dc = self.params.decay_curve.clamp(-1.0, 1.0);
        for i in 0..LUT_SIZE {
            let x = i as f32 / (LUT_SIZE - 1) as f32;
            self.atk_lut[i] = curve_shape(x, ac);
            self.dec_lut[i] = curve_shape(1.0 - x, dc);
        }
        self.lut_curves = (ac, dc);
        self.lut_dirty = false;
    }

    // -- rendering ----------------------------------------------------------

    /// Renders one block additively over zeroed buffers, applying per-voice
    /// amp envelopes and the master level. `left` and `right` must be the
    /// same length.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        left[..frames].fill(0.0);
        right[..frames].fill(0.0);
        if frames == 0 {
            return;
        }
        if self.lut_dirty {
            self.rebuild_luts();
        }
        if self.source.is_none() {
            // No sample: keep envelopes moving so released voices still
            // expire instead of hanging active forever.
            self.tick_envelopes_only(frames);
            return;
        }

        if self.scratch_l.len() < frames {
            self.scratch_l.resize(frames, 0.0);
            self.scratch_r.resize(frames, 0.0);
        }
        let mut scratch_l = core::mem::take(&mut self.scratch_l);
        let mut scratch_r = core::mem::take(&mut self.scratch_r);

        let mut total_active = self.count_active_grains();
        for vi in 0..self.voices.len() {
            if self.voices[vi].active {
                self.render_voice(
                    vi,
                    &mut scratch_l[..frames],
                    &mut scratch_r[..frames],
                    &mut left[..frames],
                    &mut right[..frames],
                    &mut total_active,
                );
            }
        }

        self.scratch_l = scratch_l;
        self.scratch_r = scratch_r;

        let level = self.params.level.clamp(0.0, 2.0);
        for i in 0..frames {
            left[i] = flush_denormal(left[i] * level);
            right[i] = flush_denormal(right[i] * level);
        }
    }

    /// Advances amp envelopes without producing audio (no sample loaded).
    fn tick_envelopes_only(&mut self, frames: usize) {
        let (atk_step, dec_step, sustain, rel_coeff) = self.env_steps();
        for voice in self.voices.iter_mut().filter(|v| v.active) {
            for _ in 0..frames {
                voice.env_next(atk_step, dec_step, sustain, rel_coeff);
                if voice.env_stage == EnvStage::Idle {
                    break;
                }
            }
            if voice.env_stage == EnvStage::Idle {
                *voice = Voice::default();
            }
        }
    }

    fn env_steps(&self) -> (f32, f32, f32, f32) {
        let p = &self.params;
        let sr = self.sample_rate;
        let atk_step = 1.0 / (p.amp_attack_s.max(0.001) * sr);
        let sustain = p.amp_sustain.clamp(0.0, 1.0);
        let dec_step = if p.amp_decay_s <= 1.0e-4 {
            f32::INFINITY
        } else {
            (1.0 - sustain).max(1.0e-6) / (p.amp_decay_s * sr)
        };
        // Exponential release: ~-60 dB after amp_release_s seconds.
        let rel_coeff = (-6.907_755 / (p.amp_release_s.max(0.01) as f64 * sr as f64)).exp() as f32;
        (atk_step, dec_step, sustain, rel_coeff)
    }

    fn render_voice(
        &mut self,
        vi: usize,
        scratch_l: &mut [f32],
        scratch_r: &mut [f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        total_active: &mut usize,
    ) {
        let frames = out_l.len();
        let params = self.params;
        let sample_rate = self.sample_rate;
        let (atk_step, dec_step, sustain, rel_coeff) = self.env_steps();
        let base_interval = self.spawn_interval_base(self.voices[vi].key);

        let Some(source) = self.source.as_ref() else {
            return;
        };
        let voice = &mut self.voices[vi];

        // Schedule this block's grain spawns at their intra-block offsets.
        while voice.spawn_countdown < frames as f64 {
            let at = voice.spawn_countdown.max(0.0) as u32;
            spawn_grain(
                voice,
                &params,
                sample_rate,
                source,
                at,
                0,
                total_active,
                &mut self.spawned_grains,
            );
            let jitter = 1.0
                + params.random_timing.clamp(0.0, 1.0) as f64 * voice.rng.bipolar() as f64 * 0.75;
            voice.spawn_countdown += (base_interval * jitter).max(1.0);
        }
        voice.spawn_countdown -= frames as f64;

        // Accumulate every grain into the voice scratch, then fold the
        // scratch into the output under the amp envelope.
        scratch_l[..frames].fill(0.0);
        scratch_r[..frames].fill(0.0);
        let max_pos = (source.frames() as f64 - 2.0).max(0.0);
        for grain in voice.grains.iter_mut().filter(|g| g.active) {
            let start = (grain.block_start as usize).min(frames);
            grain.block_start = 0;
            for (l, r) in scratch_l[start..frames]
                .iter_mut()
                .zip(scratch_r[start..frames].iter_mut())
            {
                let env = grain_env(grain, &self.atk_lut, &self.dec_lut);
                let (sl, sr) = read_source(source, grain.pos);
                *l += sl * env * grain.gain_l;
                *r += sr * env * grain.gain_r;
                grain.pos += grain.step;
                grain.age += 1;
                if grain.age >= grain.dur || grain.pos < 0.0 || grain.pos > max_pos {
                    grain.active = false;
                    *total_active = total_active.saturating_sub(1);
                    break;
                }
            }
        }

        let velocity = voice.velocity;
        for i in 0..frames {
            let env = voice.env_next(atk_step, dec_step, sustain, rel_coeff);
            if voice.env_stage == EnvStage::Idle {
                break;
            }
            let gain = env * velocity;
            out_l[i] += scratch_l[i] * gain;
            out_r[i] += scratch_r[i] * gain;
        }
        if voice.env_stage == EnvStage::Idle {
            *total_active = total_active.saturating_sub(voice.active_grain_count());
            *voice = Voice::default();
        }
    }
}

/// Number of chord offsets the current chord settings cycle through.
fn chord_offset_count(p: &GranularParams) -> i32 {
    if p.chord_type == 0 {
        return 1;
    }
    let table = CHORD_TABLES[p.chord_type.min(9) as usize];
    (table.len() * p.chord_range.clamp(1, 4) as usize) as i32
}

/// Consumes one chord offset (semitones) per grain spawn, advancing the
/// voice's picking cursor per the Chord Pattern (Up/Down/UpDown/Random).
fn next_chord_offset(voice: &mut Voice, p: &GranularParams) -> i32 {
    if p.chord_type == 0 {
        return 0;
    }
    let table = CHORD_TABLES[p.chord_type.min(9) as usize];
    let count = chord_offset_count(p);
    let idx = match p.chord_pattern {
        1 => {
            // Down.
            let i = voice.chord_cursor.rem_euclid(count);
            voice.chord_cursor = (i - 1).rem_euclid(count);
            i
        }
        2 => {
            // UpDown ping-pong, endpoints unrepeated.
            let i = voice.chord_cursor.clamp(0, count - 1);
            if count > 1 {
                let mut next = i + voice.chord_dir;
                if next >= count {
                    voice.chord_dir = -1;
                    next = count - 2;
                } else if next < 0 {
                    voice.chord_dir = 1;
                    next = 1.min(count - 1);
                }
                voice.chord_cursor = next;
            }
            i
        }
        3 => (voice.rng.next_u32() as i32).rem_euclid(count),
        _ => {
            // Up.
            let i = voice.chord_cursor.rem_euclid(count);
            voice.chord_cursor = (i + 1) % count;
            i
        }
    };
    let idx = idx.rem_euclid(count) as usize;
    table[idx % table.len()] + 12 * (idx / table.len()) as i32
}

/// Spawns one grain into `voice` (stealing its most-finished grain when the
/// voice array or the global budget is full). `at` is the intra-block start
/// offset; `age` backdates the grain for Warm Start pre-fill.
#[allow(clippy::too_many_arguments)]
fn spawn_grain(
    voice: &mut Voice,
    p: &GranularParams,
    sample_rate: f32,
    source: &SourceSample,
    at: u32,
    age: u32,
    total_active: &mut usize,
    spawned: &mut u64,
) {
    let src_frames = source.frames();
    if src_frames < 2 {
        return;
    }

    // Duration (keytracked), envelope split with click-proof minimum edges.
    let mut dur_f = p.grain_length_ms.clamp(2.0, 1000.0) as f64 * sample_rate as f64 / 1000.0;
    if p.length_keytrack {
        dur_f *= (-((voice.key as f64) - (p.root_note as f64)) / 12.0).exp2();
    }
    let dur = (dur_f.round() as u32).clamp(2, (sample_rate as f64 * 10.0) as u32);
    if age >= dur {
        return;
    }
    let min_edge = MIN_EDGE_FRAMES.min(dur / 2).max(1);
    let mut attack = (p.grain_attack.clamp(0.0, 1.0) as f64 * dur as f64) as u32;
    let mut decay = (p.grain_decay.clamp(0.0, 1.0) as f64 * dur as f64) as u32;
    if attack + decay > dur {
        // Normalize so attack+decay fills (never exceeds) the grain.
        let total = (attack + decay).max(1);
        attack = ((attack as u64 * dur as u64) / total as u64) as u32;
        decay = dur - attack;
    }
    attack = attack.max(min_edge);
    decay = decay.max(min_edge);
    if attack + decay > dur {
        attack = dur / 2;
        decay = dur - attack;
    }

    // Pitch: note transposition around root + Pitch/Fine + chord + random.
    let chord = next_chord_offset(voice, p);
    let semis = (voice.key as f64 - p.root_note.min(127) as f64)
        + p.pitch_semitones.clamp(-48.0, 48.0) as f64
        + chord as f64
        + p.random_pitch.clamp(0.0, 24.0) as f64 * voice.rng.bipolar() as f64;
    let ratio = (semis / 12.0 + p.fine_cents.clamp(-100.0, 100.0) as f64 / 1200.0).exp2();
    let mut step = ratio * source.sample_rate as f64 / sample_rate as f64;
    if voice.rng.uniform() < p.random_reverse.clamp(0.0, 1.0) {
        step = -step;
    }

    // Position: the seek bar plus spray, optionally phase-aligned.
    let max_pos = (src_frames as f64 - 2.0).max(0.0);
    let center = p.position.clamp(0.0, 1.0) as f64 * (src_frames as f64 - 1.0);
    let spray = p.random_position_ms.clamp(0.0, 2000.0) as f64 * source.sample_rate as f64 / 1000.0;
    let mut pos = center + voice.rng.bipolar() as f64 * spray;
    if p.align_phases {
        // Quantize the offset from the cursor to whole periods of the
        // played pitch so overlapping grains stay phase-coherent.
        let freq = ((voice.key as f64 + p.pitch_semitones as f64 + chord as f64 - 69.0) / 12.0)
            .exp2()
            * 440.0;
        if freq > 1.0 {
            let period = source.sample_rate as f64 / freq;
            if period >= 1.0 {
                pos = center + ((pos - center) / period).round() * period;
            }
        }
    }
    pos = pos.clamp(0.0, max_pos);
    if age > 0 {
        // Warm-start backdate: advance the read head to where this grain
        // would be now.
        pos += step * age as f64;
        if pos < 0.0 || pos > max_pos {
            return;
        }
    }

    // Level and constant-power pan, baked into per-channel gains
    // (pan 0 stays at unity on both channels).
    let level = (1.0 - p.random_level.clamp(0.0, 1.0) * voice.rng.uniform()).clamp(0.0, 1.0);
    let pan = (p.random_pan.clamp(0.0, 1.0) * voice.rng.bipolar()).clamp(-1.0, 1.0);
    let angle = (pan + 1.0) * core::f32::consts::FRAC_PI_4;
    let gain_l = level * angle.cos() * core::f32::consts::SQRT_2;
    let gain_r = level * angle.sin() * core::f32::consts::SQRT_2;

    // Slot: free grain, else steal this voice's most-finished grain. The
    // global budget forces a steal too (never a net-new active grain).
    let must_steal = *total_active >= MAX_ACTIVE_GRAINS;
    let slot = if must_steal {
        most_finished_grain(&voice.grains)
    } else {
        voice
            .grains
            .iter()
            .position(|g| !g.active)
            .or_else(|| most_finished_grain(&voice.grains))
    };
    let Some(slot) = slot else {
        return;
    };
    let was_active = voice.grains[slot].active;
    voice.grains[slot] = Grain {
        active: true,
        pos,
        step,
        age,
        dur,
        attack_end: attack,
        decay_start: dur - decay,
        block_start: at,
        gain_l,
        gain_r,
    };
    if !was_active {
        *total_active += 1;
    }
    *spawned += 1;
}

fn most_finished_grain(grains: &[Grain; MAX_GRAINS_PER_VOICE]) -> Option<usize> {
    grains
        .iter()
        .enumerate()
        .filter(|(_, g)| g.active)
        .max_by(|a, b| {
            let pa = a.1.age as f64 / a.1.dur.max(1) as f64;
            let pb = b.1.age as f64 / b.1.dur.max(1) as f64;
            pa.total_cmp(&pb)
        })
        .map(|(i, _)| i)
}

/// Grain amplitude at its current age, via the shared curve LUTs.
fn grain_env(grain: &Grain, atk_lut: &[f32; LUT_SIZE], dec_lut: &[f32; LUT_SIZE]) -> f32 {
    let age = grain.age;
    if age < grain.attack_end {
        lut_at(atk_lut, age as f32 / grain.attack_end.max(1) as f32)
    } else if age >= grain.decay_start {
        // +1 so the grain's final sample (age == dur-1) lands exactly on 0.
        let span = grain.dur.saturating_sub(grain.decay_start).max(1);
        lut_at(dec_lut, (age - grain.decay_start + 1) as f32 / span as f32)
    } else {
        1.0
    }
}

fn lut_at(lut: &[f32; LUT_SIZE], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32;
    let i = x as usize;
    if i >= LUT_SIZE - 1 {
        return lut[LUT_SIZE - 1];
    }
    let frac = x - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * frac
}

/// Curvature-shaped ramp: `c` in -1..1 maps to the exponent `2^(3c)`
/// (c=0 linear, c>0 slow start, c<0 fast start).
fn curve_shape(x: f32, c: f32) -> f32 {
    x.clamp(0.0, 1.0).powf((3.0 * c).exp2())
}

/// Linear-interpolated stereo read; mono sources feed both channels.
fn read_source(source: &SourceSample, pos: f64) -> (f32, f32) {
    let i = pos as usize;
    let frac = (pos - i as f64) as f32;
    match source.channels {
        2 => {
            let l0 = source.data[i * 2];
            let r0 = source.data[i * 2 + 1];
            let l1 = source.data[i * 2 + 2];
            let r1 = source.data[i * 2 + 3];
            (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
        }
        _ => {
            let s0 = source.data[i];
            let s1 = source.data[i + 1];
            let s = s0 + (s1 - s0) * frac;
            (s, s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_source(frames: usize, sample_rate: f32) -> SourceSample {
        SourceSample {
            sample_rate,
            channels: 1,
            data: (0..frames)
                .map(|i| (core::f32::consts::TAU * 440.0 * i as f32 / sample_rate).sin())
                .collect(),
        }
    }

    fn dc_source(frames: usize) -> SourceSample {
        SourceSample {
            sample_rate: 48_000.0,
            channels: 1,
            data: vec![0.5; frames],
        }
    }

    fn engine_with_sine() -> GranularEngine {
        let mut e = GranularEngine::new(48_000.0);
        e.set_source(sine_source(96_000, 48_000.0));
        let mut p = *e.params();
        p.position = 0.5;
        e.set_params(p);
        e
    }

    fn render_seconds(e: &mut GranularEngine, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let total = (seconds * 48_000.0) as usize;
        let mut left = vec![0.0f32; total];
        let mut right = vec![0.0f32; total];
        for (l, r) in left.chunks_mut(128).zip(right.chunks_mut(128)) {
            e.render(l, r);
        }
        (left, right)
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len().max(1) as f32).sqrt()
    }

    #[test]
    fn silent_with_no_sample() {
        let mut e = GranularEngine::new(48_000.0);
        e.note_on(60, 1.0);
        let (l, r) = render_seconds(&mut e, 0.05);
        assert!(l.iter().chain(r.iter()).all(|v| *v == 0.0));
    }

    #[test]
    fn released_voices_expire_even_without_a_sample() {
        let mut e = GranularEngine::new(48_000.0);
        e.note_on(60, 1.0);
        e.note_off(60);
        render_seconds(&mut e, 1.0);
        assert_eq!(e.active_voice_count(), 0);
    }

    #[test]
    fn note_makes_sound_and_note_off_releases() {
        let mut e = engine_with_sine();
        e.note_on(60, 1.0);
        let (l, _) = render_seconds(&mut e, 0.3);
        assert!(rms(&l) > 0.01, "grain stream should be audible");
        assert!(l.iter().all(|v| v.is_finite()));
        e.note_off(60);
        render_seconds(&mut e, 2.0);
        assert_eq!(e.active_voice_count(), 0, "release should end the voice");
        let (tail, _) = render_seconds(&mut e, 0.05);
        assert!(tail.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn free_rate_spawns_the_expected_grain_count() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.rate_hz = 25.0;
        p.spawn_mode = 0;
        e.set_params(p);
        e.note_on(60, 1.0);
        render_seconds(&mut e, 1.0);
        let n = e.spawned_grain_count();
        assert!((24..=26).contains(&n), "expected ~25 grains, got {n}");
    }

    #[test]
    fn density_mode_holds_the_target_overlap() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.spawn_mode = 2;
        p.density = 8.0;
        p.grain_length_ms = 100.0;
        e.set_params(p);
        e.note_on(60, 1.0);
        render_seconds(&mut e, 0.5); // several grain lengths → steady state
        let active = e.active_grain_count();
        assert!(
            (7..=9).contains(&active),
            "density 8 should hold ~8 overlapping grains, got {active}"
        );
    }

    #[test]
    fn sync_interval_follows_tempo() {
        let count_at = |bpm: f64| {
            let mut e = engine_with_sine();
            e.set_tempo(bpm);
            let mut p = *e.params();
            p.spawn_mode = 1;
            p.sync_index = 6; // 1/4 beat
            e.set_params(p);
            e.note_on(60, 1.0);
            render_seconds(&mut e, 1.0);
            e.spawned_grain_count()
        };
        let slow = count_at(60.0); // 0.25 s per grain → ~4
        let fast = count_at(120.0); // ~8
        assert!((3..=5).contains(&slow), "60 BPM: {slow}");
        assert!((7..=9).contains(&fast), "120 BPM: {fast}");
    }

    #[test]
    fn grain_edges_are_click_free() {
        // One grain over DC: output must fade in from and out to ~zero.
        let mut e = GranularEngine::new(48_000.0);
        e.set_source(dc_source(96_000));
        let mut p = *e.params();
        p.position = 0.5;
        p.rate_hz = 0.1; // one grain in the window
        p.grain_length_ms = 50.0;
        p.grain_attack = 0.0; // hard settings still get the micro edge
        p.grain_decay = 0.0;
        e.set_params(p);
        e.note_on(60, 1.0);
        let (l, _) = render_seconds(&mut e, 0.1);
        let first_loud = l.iter().position(|v| v.abs() > 0.25).unwrap();
        assert!(l[..=first_loud]
            .windows(2)
            .all(|w| (w[1] - w[0]).abs() < 0.1));
        let dur = (0.05 * 48_000.0) as usize;
        assert!(l[dur - 1].abs() < 1.0e-2, "grain end should fade to ~0");
    }

    #[test]
    fn lut_is_monotonic_with_exact_endpoints() {
        let mut e = GranularEngine::new(48_000.0);
        for curve in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            let mut p = *e.params();
            p.attack_curve = curve;
            p.decay_curve = curve;
            e.set_params(p);
            e.rebuild_luts();
            assert_eq!(e.atk_lut[0], 0.0);
            assert!((e.atk_lut[LUT_SIZE - 1] - 1.0).abs() < 1.0e-6);
            assert!((e.dec_lut[0] - 1.0).abs() < 1.0e-6);
            assert_eq!(e.dec_lut[LUT_SIZE - 1], 0.0);
            assert!(e.atk_lut.windows(2).all(|w| w[1] >= w[0]));
            assert!(e.dec_lut.windows(2).all(|w| w[1] <= w[0]));
        }
    }

    #[test]
    fn pitch_math_tracks_the_keyboard() {
        let mut e = engine_with_sine();
        e.note_on(60, 1.0); // at root
        let mut l = vec![0.0; 64];
        let mut r = vec![0.0; 64];
        e.render(&mut l, &mut r);
        let step_at = |e: &GranularEngine| {
            e.voices
                .iter()
                .filter(|v| v.active)
                .flat_map(|v| v.grains.iter().filter(|g| g.active))
                .map(|g| g.step)
                .next()
                .expect("an active grain")
        };
        let root_step = step_at(&e);
        assert!((root_step - 1.0).abs() < 1.0e-9, "root note plays at 1:1");

        let mut e2 = engine_with_sine();
        e2.note_on(72, 1.0); // +1 octave
        e2.render(&mut l, &mut r);
        assert!(
            (step_at(&e2) - 2.0).abs() < 1.0e-9,
            "+12 st doubles the step"
        );
    }

    #[test]
    fn random_reverse_flips_the_step() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.random_reverse = 1.0; // always reversed
        p.rate_hz = 100.0;
        e.set_params(p);
        e.note_on(60, 1.0);
        let mut l = vec![0.0; 512];
        let mut r = vec![0.0; 512];
        e.render(&mut l, &mut r);
        let steps: Vec<f64> = e
            .voices
            .iter()
            .filter(|v| v.active)
            .flat_map(|v| v.grains.iter().filter(|g| g.active))
            .map(|g| g.step)
            .collect();
        assert!(!steps.is_empty());
        assert!(steps.iter().all(|s| *s < 0.0));
    }

    #[test]
    fn keytracked_length_halves_per_octave() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.length_keytrack = true;
        p.grain_length_ms = 100.0;
        e.set_params(p);
        assert!((e.grain_dur_frames(60) - 4_800.0).abs() < 1.0);
        assert!((e.grain_dur_frames(72) - 2_400.0).abs() < 1.0);
        assert!((e.grain_dur_frames(48) - 9_600.0).abs() < 1.0);
    }

    #[test]
    fn warm_start_opens_at_steady_state() {
        // DC source: a sine would let evenly spaced grains cancel in
        // anti-phase, which is not what this test is about.
        let build = |warm: bool| {
            let mut e = GranularEngine::new(48_000.0);
            e.set_source(dc_source(96_000));
            let mut p = *e.params();
            p.position = 0.5;
            p.spawn_mode = 2;
            p.density = 8.0;
            p.warm_start = warm;
            p.amp_attack_s = 0.001;
            e.set_params(p);
            e.note_on(60, 1.0);
            e
        };
        let cold = build(false);
        let mut warm = build(true);
        assert_eq!(cold.active_grain_count(), 0, "cold start ramps in");
        let active = warm.active_grain_count();
        assert!(
            (6..=9).contains(&active),
            "warm start should pre-fill ~8 grains, got {active}"
        );
        // And the very first block is already loud.
        let mut l = vec![0.0; 2_048];
        let mut r = vec![0.0; 2_048];
        warm.render(&mut l, &mut r);
        assert!(
            rms(&l[96..]) > 0.05,
            "warm start should be instantly audible"
        );
    }

    #[test]
    fn align_phases_offsets_are_whole_periods() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.align_phases = true;
        p.random_position_ms = 500.0;
        p.rate_hz = 200.0;
        e.set_params(p);
        e.note_on(69, 1.0); // A4: period = 48000/440 with root 60 offset +9 st
        let mut l = vec![0.0; 2_048];
        let mut r = vec![0.0; 2_048];
        e.render(&mut l, &mut r);
        let center = 0.5 * (96_000.0 - 1.0);
        let freq = ((69.0f64 - 69.0) / 12.0).exp2() * 440.0;
        let period = 48_000.0 / freq;
        for voice in e.voices.iter().filter(|v| v.active) {
            for g in voice.grains.iter().filter(|g| g.active) {
                let spawn_pos = g.pos - g.step * g.age as f64;
                let periods = (spawn_pos - center) / period;
                assert!(
                    (periods - periods.round()).abs() < 1.0e-6,
                    "grain offset {periods} periods should be whole"
                );
            }
        }
    }

    #[test]
    fn chord_up_cycles_intervals_and_range_extends_octaves() {
        let p = GranularParams {
            chord_type: 3, // Major [0,4,7]
            chord_range: 2,
            chord_pattern: 0, // Up
            ..GranularParams::default()
        };
        let mut voice = Voice::default();
        let picked: Vec<i32> = (0..8).map(|_| next_chord_offset(&mut voice, &p)).collect();
        assert_eq!(picked, vec![0, 4, 7, 12, 16, 19, 0, 4]);

        let down = GranularParams {
            chord_pattern: 1,
            ..p
        };
        // (Down starts from the cursor note_on seeds; emulate that here.)
        let mut voice = Voice {
            chord_cursor: chord_offset_count(&down) - 1,
            ..Voice::default()
        };
        let picked: Vec<i32> = (0..4)
            .map(|_| next_chord_offset(&mut voice, &down))
            .collect();
        assert_eq!(picked, vec![19, 16, 12, 7]);
    }

    #[test]
    fn updown_pattern_ping_pongs_without_repeating_endpoints() {
        let p = GranularParams {
            chord_type: 3, // Major [0,4,7]
            chord_range: 1,
            chord_pattern: 2,
            ..GranularParams::default()
        };
        let mut voice = Voice::default();
        let picked: Vec<i32> = (0..7).map(|_| next_chord_offset(&mut voice, &p)).collect();
        assert_eq!(picked, vec![0, 4, 7, 4, 0, 4, 7]);
    }

    #[test]
    fn renders_are_deterministic() {
        let run = || {
            let mut e = engine_with_sine();
            let mut p = *e.params();
            p.random_position_ms = 300.0;
            p.random_pitch = 7.0;
            p.random_level = 0.5;
            p.random_pan = 1.0;
            p.random_timing = 0.8;
            p.random_reverse = 0.5;
            e.set_params(p);
            e.note_on(60, 0.9);
            e.note_on(67, 0.7);
            render_seconds(&mut e, 0.25)
        };
        let (l1, r1) = run();
        let (l2, r2) = run();
        assert_eq!(l1, l2, "same note sequence must render identically");
        assert_eq!(r1, r2);
    }

    #[test]
    fn global_grain_budget_is_never_exceeded() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.spawn_mode = 2;
        p.density = 64.0;
        p.grain_length_ms = 1000.0;
        e.set_params(p);
        for key in 40..56 {
            e.note_on(key, 1.0);
        }
        render_seconds(&mut e, 0.5);
        assert!(e.active_grain_count() <= MAX_ACTIVE_GRAINS);
        assert!(e.active_voice_count() <= MAX_VOICES);
    }

    #[test]
    fn chunked_upload_assembles_and_commits() {
        let mut e = GranularEngine::new(48_000.0);
        e.begin_upload(44_100.0, 1, 1_000);
        let pcm: Vec<f32> = (0..1_000)
            .map(|i| (core::f32::consts::TAU * 100.0 * i as f32 / 44_100.0).sin())
            .collect();
        for (chunk_index, chunk) in pcm.chunks(256).enumerate() {
            let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
            e.upload_chunk((chunk_index * 256) as u32, &bytes);
        }
        e.commit_sample();
        assert!(e.has_sample());
        let (frames, rate, channels) = e.source_info();
        assert_eq!(frames, 1_000);
        assert_eq!(rate, 44_100.0);
        assert_eq!(channels, 1);
        e.note_on(60, 1.0);
        let (l, _) = render_seconds(&mut e, 0.02);
        assert!(l.iter().any(|v| v.abs() > 0.0));
    }

    #[test]
    fn out_of_bounds_chunk_aborts_the_upload() {
        let mut e = GranularEngine::new(48_000.0);
        e.begin_upload(44_100.0, 1, 100);
        let bytes: Vec<u8> = (0..64u32).flat_map(|_| 0.5f32.to_le_bytes()).collect();
        e.upload_chunk(90, &bytes); // 90 + 64 > 100
        e.commit_sample();
        assert!(
            !e.has_sample(),
            "poisoned upload must not become the source"
        );
    }

    #[test]
    fn clear_drops_sample_and_voices() {
        let mut e = engine_with_sine();
        e.note_on(60, 1.0);
        e.clear();
        assert!(!e.has_sample());
        assert_eq!(e.active_voice_count(), 0);
    }

    #[test]
    fn grain_positions_report_normalized_spread() {
        let mut e = engine_with_sine();
        let mut p = *e.params();
        p.position = 0.5;
        p.random_position_ms = 400.0;
        p.rate_hz = 200.0;
        e.set_params(p);
        e.note_on(60, 1.0);
        let mut l = vec![0.0; 4_096];
        let mut r = vec![0.0; 4_096];
        e.render(&mut l, &mut r);
        let mut out = [0.0f32; 32];
        let n = e.grain_positions(&mut out);
        assert!(n >= 2);
        assert!(out[..n].iter().all(|p| (0.0..=1.0).contains(p)));
        let spread = out[..n]
            .iter()
            .fold((1.0f32, 0.0f32), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        assert!(spread.1 - spread.0 > 0.001, "spray should spread positions");
    }

    #[test]
    fn stereo_source_reads_both_channels() {
        // Left channel DC 0.5, right channel DC -0.5.
        let mut e = GranularEngine::new(48_000.0);
        let mut data = Vec::with_capacity(96_000 * 2);
        for _ in 0..96_000 {
            data.push(0.5);
            data.push(-0.5);
        }
        e.set_source(SourceSample {
            sample_rate: 48_000.0,
            channels: 2,
            data,
        });
        let mut p = *e.params();
        p.position = 0.5;
        p.spawn_mode = 2;
        p.density = 8.0;
        p.warm_start = true;
        e.set_params(p);
        e.note_on(60, 1.0);
        let (l, r) = render_seconds(&mut e, 0.2);
        assert!(rms(&l[4_800..]) > 0.05);
        let corr: f32 = l.iter().zip(r.iter()).skip(4_800).map(|(a, b)| a * b).sum();
        assert!(corr < 0.0, "opposite-polarity channels must stay distinct");
    }
}
