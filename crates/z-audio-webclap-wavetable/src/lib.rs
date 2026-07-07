//! Z Audio Wave Synth — a Serum-inspired wavetable synthesizer WCLAP
//! instrument.
//!
//! Two morphing wavetable oscillators (factory tables with band-limited
//! mip levels, up to 8-voice unison each), a per-voice TPT state-variable
//! filter, two ADSR envelopes (Env1 hard-wired to amp), two LFOs, and an
//! 8-slot modulation matrix — every slot field is an automatable host
//! parameter. See [`engine::SynthEngine`] for the DSP and [`params`] for
//! the id surface (web ids 500-603).
//!
//! The [`engine`], [`params`] and [`protocol`] modules are `pub` because
//! this crate is also meant to serve as the engine library for a future
//! native VST3/CLAP build (same pattern as the granular synth).
//!
//! Pitch bend: the WebCLAP scaffold only delivers note on/off events, so
//! the Bend Range parameter is declared (for id-surface stability) but
//! has no effect until a native build feeds real bend input.

pub mod engine;
pub mod params;
pub mod protocol;
pub mod wavetable;

use std::sync::OnceLock;

use engine::{EnvParams, LfoParams, OscParams, SynthEngine, SynthParams};
use params::*;
use protocol::{encode_meter, encode_wave, PREVIEW_LEN};
use wclap_plugin::{
    init_plugin, send_to_ui, silence, NoteEventKind, ParamDef, Plugin, PluginDef, ProcessCtx,
    ProcessStatus,
};

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.wavetable\0",
    name: b"Z Audio Wave Synth\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.1.0\0",
    description: b"Serum-inspired wavetable synth: 2 morphing oscillators with unison, SVF filter, 2 envelopes, 2 LFOs, mod matrix\0",
    features: &[b"instrument\0", b"synthesizer\0", b"stereo\0"],
    audio_inputs: 0,
    audio_outputs: 1,
    note_inputs: 1,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

fn osc<'a>(p: &'a SynthParams, base: u32) -> &'a OscParams {
    if base == OSC_A_BASE {
        &p.osc_a
    } else {
        &p.osc_b
    }
}

fn env<'a>(p: &'a SynthParams, base: u32) -> &'a EnvParams {
    if base == ENV1_BASE {
        &p.env1
    } else {
        &p.env2
    }
}

fn lfo<'a>(p: &'a SynthParams, base: u32) -> &'a LfoParams {
    if base == LFO1_BASE {
        &p.lfo1
    } else {
        &p.lfo2
    }
}

/// Reads one engine param by web id (shared by `get_param` and tests).
pub fn param_value(p: &SynthParams, id: u32) -> f64 {
    let v: f32 = match id {
        P_MASTER => p.master,
        P_POLYPHONY => p.polyphony as f32,
        P_BEND_RANGE => p.bend_range,
        P_GLIDE => p.glide_s,
        P_FILTER_ENABLE => p.filter_enable as u8 as f32,
        P_FILTER_TYPE => p.filter_type as f32,
        P_FILTER_CUTOFF => p.cutoff_hz,
        P_FILTER_RESO => p.resonance,
        P_FILTER_DRIVE => p.drive,
        P_FILTER_KEYTRACK => p.keytrack,
        P_FILTER_MIX => p.filter_mix,
        P_FILTER_ROUTE_A => p.route_a as u8 as f32,
        P_FILTER_ROUTE_B => p.route_b as u8 as f32,
        id if (OSC_A_BASE..OSC_A_BASE + OSC_FIELDS).contains(&id)
            || (OSC_B_BASE..OSC_B_BASE + OSC_FIELDS).contains(&id) =>
        {
            let base = if id >= OSC_B_BASE {
                OSC_B_BASE
            } else {
                OSC_A_BASE
            };
            let o = osc(p, base);
            match id - base {
                OSC_ENABLE => o.enable as u8 as f32,
                OSC_TABLE => o.table as f32,
                OSC_WT_POS => o.wt_pos,
                OSC_OCTAVE => o.octave as f32,
                OSC_SEMI => o.semi as f32,
                OSC_FINE => o.fine_cents,
                OSC_UNISON => o.unison as f32,
                OSC_UNI_DETUNE => o.uni_detune,
                OSC_UNI_BLEND => o.uni_blend,
                OSC_PHASE => o.phase,
                OSC_RAND_PHASE => o.rand_phase,
                OSC_PAN => o.pan,
                _ => o.level,
            }
        }
        id if (ENV1_BASE..ENV1_BASE + ENV_FIELDS).contains(&id)
            || (ENV2_BASE..ENV2_BASE + ENV_FIELDS).contains(&id) =>
        {
            let base = if id >= ENV2_BASE {
                ENV2_BASE
            } else {
                ENV1_BASE
            };
            let e = env(p, base);
            match id - base {
                ENV_ATTACK => e.attack_s,
                ENV_DECAY => e.decay_s,
                ENV_SUSTAIN => e.sustain,
                ENV_RELEASE => e.release_s,
                _ => e.curve,
            }
        }
        id if (LFO1_BASE..LFO1_BASE + LFO_FIELDS).contains(&id)
            || (LFO2_BASE..LFO2_BASE + LFO_FIELDS).contains(&id) =>
        {
            let base = if id >= LFO2_BASE {
                LFO2_BASE
            } else {
                LFO1_BASE
            };
            let l = lfo(p, base);
            match id - base {
                LFO_WAVE => l.wave as f32,
                LFO_RATE => l.rate_hz,
                LFO_PHASE => l.phase,
                _ => l.retrig as u8 as f32,
            }
        }
        id if (MOD_BASE..MOD_BASE + MOD_SLOTS * MOD_FIELDS).contains(&id) => {
            let slot = &p.mods[((id - MOD_BASE) / MOD_FIELDS) as usize];
            match (id - MOD_BASE) % MOD_FIELDS {
                MOD_SOURCE => slot.source as f32,
                MOD_DEST => slot.dest as f32,
                _ => slot.amount,
            }
        }
        _ => 0.0,
    };
    v as f64
}

/// Writes one engine param by web id, clamped to its declared range
/// (shared by `set_param` and tests).
pub fn apply_param(p: &mut SynthParams, id: u32, value: f64) {
    let v = value as f32;
    match id {
        P_MASTER => p.master = v.clamp(0.0, 1.0),
        P_POLYPHONY => p.polyphony = v.clamp(1.0, 16.0).round() as u8,
        P_BEND_RANGE => p.bend_range = v.clamp(0.0, 24.0).round(),
        P_GLIDE => p.glide_s = v.clamp(0.0, 2.0),
        P_FILTER_ENABLE => p.filter_enable = v >= 0.5,
        P_FILTER_TYPE => p.filter_type = v.clamp(0.0, 3.0).round() as u8,
        P_FILTER_CUTOFF => p.cutoff_hz = v.clamp(20.0, 20_000.0),
        P_FILTER_RESO => p.resonance = v.clamp(0.0, 1.0),
        P_FILTER_DRIVE => p.drive = v.clamp(0.0, 1.0),
        P_FILTER_KEYTRACK => p.keytrack = v.clamp(0.0, 1.0),
        P_FILTER_MIX => p.filter_mix = v.clamp(0.0, 1.0),
        P_FILTER_ROUTE_A => p.route_a = v >= 0.5,
        P_FILTER_ROUTE_B => p.route_b = v >= 0.5,
        id if (OSC_A_BASE..OSC_A_BASE + OSC_FIELDS).contains(&id)
            || (OSC_B_BASE..OSC_B_BASE + OSC_FIELDS).contains(&id) =>
        {
            let base = if id >= OSC_B_BASE {
                OSC_B_BASE
            } else {
                OSC_A_BASE
            };
            let o = if base == OSC_A_BASE {
                &mut p.osc_a
            } else {
                &mut p.osc_b
            };
            match id - base {
                OSC_ENABLE => o.enable = v >= 0.5,
                OSC_TABLE => {
                    o.table = v.clamp(0.0, (wavetable::TABLE_COUNT - 1) as f32).round() as u8
                }
                OSC_WT_POS => o.wt_pos = v.clamp(0.0, 1.0),
                OSC_OCTAVE => o.octave = v.clamp(-4.0, 4.0).round() as i8,
                OSC_SEMI => o.semi = v.clamp(-12.0, 12.0).round() as i8,
                OSC_FINE => o.fine_cents = v.clamp(-100.0, 100.0),
                OSC_UNISON => o.unison = v.clamp(1.0, 8.0).round() as u8,
                OSC_UNI_DETUNE => o.uni_detune = v.clamp(0.0, 1.0),
                OSC_UNI_BLEND => o.uni_blend = v.clamp(0.0, 1.0),
                OSC_PHASE => o.phase = v.clamp(0.0, 1.0),
                OSC_RAND_PHASE => o.rand_phase = v.clamp(0.0, 1.0),
                OSC_PAN => o.pan = v.clamp(-1.0, 1.0),
                _ => o.level = v.clamp(0.0, 1.0),
            }
        }
        id if (ENV1_BASE..ENV1_BASE + ENV_FIELDS).contains(&id)
            || (ENV2_BASE..ENV2_BASE + ENV_FIELDS).contains(&id) =>
        {
            let base = if id >= ENV2_BASE {
                ENV2_BASE
            } else {
                ENV1_BASE
            };
            let e = if base == ENV1_BASE {
                &mut p.env1
            } else {
                &mut p.env2
            };
            match id - base {
                ENV_ATTACK => e.attack_s = v.clamp(0.0, 5.0),
                ENV_DECAY => e.decay_s = v.clamp(0.0, 5.0),
                ENV_SUSTAIN => e.sustain = v.clamp(0.0, 1.0),
                ENV_RELEASE => e.release_s = v.clamp(0.0, 5.0),
                _ => e.curve = v.clamp(-1.0, 1.0),
            }
        }
        id if (LFO1_BASE..LFO1_BASE + LFO_FIELDS).contains(&id)
            || (LFO2_BASE..LFO2_BASE + LFO_FIELDS).contains(&id) =>
        {
            let base = if id >= LFO2_BASE {
                LFO2_BASE
            } else {
                LFO1_BASE
            };
            let l = if base == LFO1_BASE {
                &mut p.lfo1
            } else {
                &mut p.lfo2
            };
            match id - base {
                LFO_WAVE => l.wave = v.clamp(0.0, 4.0).round() as u8,
                LFO_RATE => l.rate_hz = v.clamp(0.01, 20.0),
                LFO_PHASE => l.phase = v.clamp(0.0, 1.0),
                _ => l.retrig = v >= 0.5,
            }
        }
        id if (MOD_BASE..MOD_BASE + MOD_SLOTS * MOD_FIELDS).contains(&id) => {
            let slot = &mut p.mods[((id - MOD_BASE) / MOD_FIELDS) as usize];
            match (id - MOD_BASE) % MOD_FIELDS {
                MOD_SOURCE => slot.source = v.clamp(0.0, (SRC_COUNT - 1) as f32).round() as u8,
                MOD_DEST => slot.dest = v.clamp(0.0, (DST_COUNT - 1) as f32).round() as u8,
                _ => slot.amount = v.clamp(-1.0, 1.0),
            }
        }
        _ => {}
    }
}

struct ZAudioWebWavetable {
    engine: SynthEngine,
    note_events: Vec<wclap_plugin::NoteEvent>,
    left: Vec<f32>,
    right: Vec<f32>,
    /// UI has said `ready` at least once, so pushes have a peer.
    ui_seen: bool,
    /// Output samples until the next meter push (~30 Hz).
    meter_countdown: usize,
    /// Waveform previews resend when these change (table, wt_pos ×2).
    last_preview_key: (u8, u32, u8, u32),
    sample_rate: f32,
}

impl ZAudioWebWavetable {
    fn preview_key(p: &SynthParams) -> (u8, u32, u8, u32) {
        (
            p.osc_a.table,
            p.osc_a.wt_pos.to_bits(),
            p.osc_b.table,
            p.osc_b.wt_pos.to_bits(),
        )
    }

    fn push_previews(&mut self) {
        let mut buf = [0.0f32; PREVIEW_LEN];
        self.engine.preview_wave(false, &mut buf);
        send_to_ui(&encode_wave(false, &buf));
        self.engine.preview_wave(true, &mut buf);
        send_to_ui(&encode_wave(true, &buf));
        self.last_preview_key = Self::preview_key(self.engine.params());
    }

    fn push_meter(&mut self) {
        let (env1, env2, lfo1, lfo2) = self.engine.meter();
        let voices = self.engine.active_voices().min(255) as u8;
        send_to_ui(&encode_meter(voices, env1, env2, lfo1, lfo2));
    }
}

impl Plugin for ZAudioWebWavetable {
    fn new() -> Self {
        Self {
            engine: SynthEngine::new(48_000.0),
            note_events: Vec::with_capacity(128),
            left: vec![0.0; 128],
            right: vec![0.0; 128],
            ui_seen: false,
            meter_countdown: 0,
            last_preview_key: (255, 0, 255, 0),
            sample_rate: 48_000.0,
        }
    }

    fn activate(&mut self, sample_rate: f64, max_frames: u32) {
        self.sample_rate = sample_rate as f32;
        self.engine.set_sample_rate(sample_rate as f32);
        let frames = (max_frames as usize).max(1);
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        if self.note_events.capacity() < frames.max(64) {
            self.note_events
                .reserve_exact(frames.max(64) - self.note_events.capacity());
        }
    }

    fn reset(&mut self) {
        self.engine.reset_voices();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(param_defs)
    }

    fn get_param(&self, id: u32) -> f64 {
        param_value(self.engine.params(), id)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let mut p = *self.engine.params();
        apply_param(&mut p, id, value);
        self.engine.set_params(p);
    }

    fn on_ui_message(&mut self, bytes: &[u8]) -> bool {
        if bytes == b"\x65ready" {
            // UI (re)opened: after the scaffold's automatic params
            // snapshot, seed its oscillator canvases.
            self.ui_seen = true;
            self.push_previews();
            return true;
        }
        false
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        let frames = ctx.frames();
        if frames == 0 || frames > self.left.len() || frames > self.right.len() {
            silence(ctx);
            return ProcessStatus::Continue;
        }

        ctx.collect_note_events(&mut self.note_events);

        let left = &mut self.left[..frames];
        let right = &mut self.right[..frames];
        left.fill(0.0);
        right.fill(0.0);

        // Render in segments split at note-event offsets so triggers land
        // sample-accurately within the block.
        let mut start = 0usize;
        let mut event_index = 0usize;
        while start < frames {
            let mut end = frames;
            while event_index < self.note_events.len() {
                let ev = &self.note_events[event_index];
                let at = (ev.time as usize).min(frames);
                if at <= start {
                    let key = ev.key.clamp(0, 127) as u8;
                    match ev.kind {
                        NoteEventKind::On => self.engine.note_on(key, ev.velocity as f32),
                        NoteEventKind::Off => self.engine.note_off(key),
                    }
                    event_index += 1;
                    continue;
                }
                end = at;
                break;
            }
            if end > start {
                self.engine
                    .render(&mut left[start..end], &mut right[start..end]);
            }
            start = end;
        }
        while event_index < self.note_events.len() {
            let ev = &self.note_events[event_index];
            let key = ev.key.clamp(0, 127) as u8;
            match ev.kind {
                NoteEventKind::On => self.engine.note_on(key, ev.velocity as f32),
                NoteEventKind::Off => self.engine.note_off(key),
            }
            event_index += 1;
        }

        // UI pushes at ~30 Hz: meters always, previews only when the
        // table/morph inputs actually changed.
        if self.ui_seen {
            self.meter_countdown = self.meter_countdown.saturating_sub(frames);
            if self.meter_countdown == 0 {
                self.meter_countdown = (self.sample_rate / 30.0) as usize;
                self.push_meter();
                if Self::preview_key(self.engine.params()) != self.last_preview_key {
                    self.push_previews();
                }
            }
        }

        let wrote_l = match ctx.output_mut(0, 0) {
            Some(out) => {
                out[..frames].copy_from_slice(&self.left[..frames]);
                true
            }
            None => false,
        };
        let wrote_r = match ctx.output_mut(0, 1) {
            Some(out) => {
                out[..frames].copy_from_slice(&self.right[..frames]);
                true
            }
            None => false,
        };
        if !wrote_l && !wrote_r {
            silence(ctx);
        }
        ProcessStatus::Continue
    }
}

// Only exported from the wasm cdylib; a future native VST3/CLAP plugin
// links this crate as an rlib and must not re-export a WASI entry point.
#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioWebWavetable>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::ModSlot;

    #[test]
    fn param_defaults_round_trip_through_the_id_surface() {
        let p = SynthParams::default();
        for def in param_defs() {
            let got = param_value(&p, def.id);
            // Engine state is f32, so defaults round-trip at f32 precision.
            assert!(
                (got - def.default).abs() < 1.0e-6,
                "param {} default mismatch: engine {} vs surface {}",
                def.id,
                got,
                def.default
            );
        }
    }

    #[test]
    fn apply_param_clamps_to_declared_ranges() {
        let mut p = SynthParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max + 1_000.0);
            assert!(
                param_value(&p, def.id) <= def.max + 1.0e-9,
                "param {} must clamp to max",
                def.id
            );
            apply_param(&mut p, def.id, def.min - 1_000.0);
            assert!(
                param_value(&p, def.id) >= def.min - 1.0e-9,
                "param {} must clamp to min",
                def.id
            );
            apply_param(&mut p, def.id, def.default);
        }
        assert_eq!(p, SynthParams::default());
    }

    #[test]
    fn set_and_get_agree_for_every_param() {
        let mut p = SynthParams::default();
        for def in param_defs() {
            let probe = (def.min + def.max) * 0.5;
            apply_param(&mut p, def.id, probe);
            let got = param_value(&p, def.id);
            // Stepped params round; continuous ones must match exactly.
            assert!(
                (got - probe).abs() <= 0.5 + 1.0e-6,
                "param {} probe {} → {}",
                def.id,
                probe,
                got
            );
        }
    }

    #[test]
    fn mod_slot_ids_map_to_the_right_slots() {
        let mut p = SynthParams::default();
        apply_param(&mut p, MOD_BASE + 7 * MOD_FIELDS + MOD_SOURCE, 2.0);
        apply_param(&mut p, MOD_BASE + 7 * MOD_FIELDS + MOD_DEST, 9.0);
        apply_param(&mut p, MOD_BASE + 7 * MOD_FIELDS + MOD_AMOUNT, -0.5);
        assert_eq!(p.mods[7].source as usize, SRC_LFO1);
        assert_eq!(p.mods[7].dest as usize, DST_CUTOFF);
        assert!((p.mods[7].amount + 0.5).abs() < 1.0e-6);
        assert_eq!(p.mods[0], ModSlot::default());
    }
}
