//! The wavetable synth's parameter surface.
//!
//! Web ids 500-603 — the next free block (simple synth 100s, sampler
//! 300s, granular 400s). A future native VST3/CLAP build must mirror
//! these ids one-to-one.
//!
//! Layout: 500s globals, 510s OSC A, 530s OSC B, 550s filter, 560s
//! envelopes, 570s LFOs, 580-603 the 8×3 mod matrix.

use wclap_plugin::{ParamDef, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED};

pub const P_MASTER: u32 = 500;
pub const P_POLYPHONY: u32 = 501;
pub const P_BEND_RANGE: u32 = 502;
pub const P_GLIDE: u32 = 503;

/// Per-oscillator field offsets from the block base (A=510, B=530).
pub const OSC_A_BASE: u32 = 510;
pub const OSC_B_BASE: u32 = 530;
pub const OSC_ENABLE: u32 = 0;
pub const OSC_TABLE: u32 = 1;
pub const OSC_WT_POS: u32 = 2;
pub const OSC_OCTAVE: u32 = 3;
pub const OSC_SEMI: u32 = 4;
pub const OSC_FINE: u32 = 5;
pub const OSC_UNISON: u32 = 6;
pub const OSC_UNI_DETUNE: u32 = 7;
pub const OSC_UNI_BLEND: u32 = 8;
pub const OSC_PHASE: u32 = 9;
pub const OSC_RAND_PHASE: u32 = 10;
pub const OSC_PAN: u32 = 11;
pub const OSC_LEVEL: u32 = 12;
pub const OSC_FIELDS: u32 = 13;

pub const P_FILTER_ENABLE: u32 = 550;
pub const P_FILTER_TYPE: u32 = 551;
pub const P_FILTER_CUTOFF: u32 = 552;
pub const P_FILTER_RESO: u32 = 553;
pub const P_FILTER_DRIVE: u32 = 554;
pub const P_FILTER_KEYTRACK: u32 = 555;
pub const P_FILTER_MIX: u32 = 556;
pub const P_FILTER_ROUTE_A: u32 = 557;
pub const P_FILTER_ROUTE_B: u32 = 558;

/// Envelope field offsets from the block base (ENV1=560, ENV2=565).
pub const ENV1_BASE: u32 = 560;
pub const ENV2_BASE: u32 = 565;
pub const ENV_ATTACK: u32 = 0;
pub const ENV_DECAY: u32 = 1;
pub const ENV_SUSTAIN: u32 = 2;
pub const ENV_RELEASE: u32 = 3;
pub const ENV_CURVE: u32 = 4;
pub const ENV_FIELDS: u32 = 5;

/// LFO field offsets from the block base (LFO1=570, LFO2=574).
pub const LFO1_BASE: u32 = 570;
pub const LFO2_BASE: u32 = 574;
pub const LFO_WAVE: u32 = 0;
pub const LFO_RATE: u32 = 1;
pub const LFO_PHASE: u32 = 2;
pub const LFO_RETRIG: u32 = 3;
pub const LFO_FIELDS: u32 = 4;

/// Mod matrix: 8 slots × (source, dest, amount) starting at 580.
pub const MOD_BASE: u32 = 580;
pub const MOD_SLOTS: u32 = 8;
pub const MOD_SOURCE: u32 = 0;
pub const MOD_DEST: u32 = 1;
pub const MOD_AMOUNT: u32 = 2;
pub const MOD_FIELDS: u32 = 3;

/// Mod sources, in stepped-parameter order.
pub const SRC_NONE: usize = 0;
pub const SRC_ENV2: usize = 1;
pub const SRC_LFO1: usize = 2;
pub const SRC_LFO2: usize = 3;
pub const SRC_VELOCITY: usize = 4;
pub const SRC_NOTE: usize = 5;
pub const SRC_COUNT: usize = 6;

/// Mod destinations, in stepped-parameter order.
pub const DST_NONE: usize = 0;
pub const DST_A_WT_POS: usize = 1;
pub const DST_A_PITCH: usize = 2;
pub const DST_A_LEVEL: usize = 3;
pub const DST_A_PAN: usize = 4;
pub const DST_B_WT_POS: usize = 5;
pub const DST_B_PITCH: usize = 6;
pub const DST_B_LEVEL: usize = 7;
pub const DST_B_PAN: usize = 8;
pub const DST_CUTOFF: usize = 9;
pub const DST_RESO: usize = 10;
pub const DST_MASTER: usize = 11;
pub const DST_COUNT: usize = 12;

fn def(id: u32, name: &'static [u8], min: f64, max: f64, default: f64, stepped: bool) -> ParamDef {
    ParamDef {
        id,
        flags: if stepped {
            PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED
        } else {
            PARAM_IS_AUTOMATABLE
        },
        name,
        module: b"\0",
        min,
        max,
        default,
    }
}

fn osc_defs(defs: &mut Vec<ParamDef>, base: u32, enabled: f64) {
    // Static names per block — ParamDef needs 'static byte strings.
    let names: &[&'static [u8]] = if base == OSC_A_BASE {
        &[
            b"A Enable\0",
            b"A Table\0",
            b"A WT Pos\0",
            b"A Octave\0",
            b"A Semi\0",
            b"A Fine\0",
            b"A Unison\0",
            b"A Uni Detune\0",
            b"A Uni Blend\0",
            b"A Phase\0",
            b"A Rand Phase\0",
            b"A Pan\0",
            b"A Level\0",
        ]
    } else {
        &[
            b"B Enable\0",
            b"B Table\0",
            b"B WT Pos\0",
            b"B Octave\0",
            b"B Semi\0",
            b"B Fine\0",
            b"B Unison\0",
            b"B Uni Detune\0",
            b"B Uni Blend\0",
            b"B Phase\0",
            b"B Rand Phase\0",
            b"B Pan\0",
            b"B Level\0",
        ]
    };
    defs.push(def(base + OSC_ENABLE, names[0], 0.0, 1.0, enabled, true));
    defs.push(def(
        base + OSC_TABLE,
        names[1],
        0.0,
        (crate::wavetable::TABLE_COUNT - 1) as f64,
        0.0,
        true,
    ));
    defs.push(def(base + OSC_WT_POS, names[2], 0.0, 1.0, 0.0, false));
    defs.push(def(base + OSC_OCTAVE, names[3], -4.0, 4.0, 0.0, true));
    defs.push(def(base + OSC_SEMI, names[4], -12.0, 12.0, 0.0, true));
    defs.push(def(base + OSC_FINE, names[5], -100.0, 100.0, 0.0, false));
    defs.push(def(base + OSC_UNISON, names[6], 1.0, 8.0, 1.0, true));
    defs.push(def(base + OSC_UNI_DETUNE, names[7], 0.0, 1.0, 0.25, false));
    defs.push(def(base + OSC_UNI_BLEND, names[8], 0.0, 1.0, 0.75, false));
    defs.push(def(base + OSC_PHASE, names[9], 0.0, 1.0, 0.0, false));
    defs.push(def(base + OSC_RAND_PHASE, names[10], 0.0, 1.0, 1.0, false));
    defs.push(def(base + OSC_PAN, names[11], -1.0, 1.0, 0.0, false));
    defs.push(def(base + OSC_LEVEL, names[12], 0.0, 1.0, 0.75, false));
}

fn env_defs(defs: &mut Vec<ParamDef>, base: u32, sustain: f64) {
    let names: &[&'static [u8]] = if base == ENV1_BASE {
        &[
            b"Env1 Attack\0",
            b"Env1 Decay\0",
            b"Env1 Sustain\0",
            b"Env1 Release\0",
            b"Env1 Curve\0",
        ]
    } else {
        &[
            b"Env2 Attack\0",
            b"Env2 Decay\0",
            b"Env2 Sustain\0",
            b"Env2 Release\0",
            b"Env2 Curve\0",
        ]
    };
    defs.push(def(base + ENV_ATTACK, names[0], 0.0, 5.0, 0.005, false));
    defs.push(def(base + ENV_DECAY, names[1], 0.0, 5.0, 0.2, false));
    defs.push(def(base + ENV_SUSTAIN, names[2], 0.0, 1.0, sustain, false));
    defs.push(def(base + ENV_RELEASE, names[3], 0.0, 5.0, 0.15, false));
    defs.push(def(base + ENV_CURVE, names[4], -1.0, 1.0, 0.0, false));
}

fn lfo_defs(defs: &mut Vec<ParamDef>, base: u32) {
    let names: &[&'static [u8]] = if base == LFO1_BASE {
        &[
            b"LFO1 Wave\0",
            b"LFO1 Rate\0",
            b"LFO1 Phase\0",
            b"LFO1 Retrig\0",
        ]
    } else {
        &[
            b"LFO2 Wave\0",
            b"LFO2 Rate\0",
            b"LFO2 Phase\0",
            b"LFO2 Retrig\0",
        ]
    };
    defs.push(def(base + LFO_WAVE, names[0], 0.0, 4.0, 0.0, true));
    defs.push(def(base + LFO_RATE, names[1], 0.01, 20.0, 2.0, false));
    defs.push(def(base + LFO_PHASE, names[2], 0.0, 1.0, 0.0, false));
    defs.push(def(base + LFO_RETRIG, names[3], 0.0, 1.0, 1.0, true));
}

pub fn param_defs() -> Vec<ParamDef> {
    let mut defs = Vec::with_capacity(84);
    defs.push(def(P_MASTER, b"Master\0", 0.0, 1.0, 0.8, false));
    defs.push(def(P_POLYPHONY, b"Polyphony\0", 1.0, 16.0, 8.0, true));
    defs.push(def(P_BEND_RANGE, b"Bend Range\0", 0.0, 24.0, 2.0, true));
    defs.push(def(P_GLIDE, b"Glide\0", 0.0, 2.0, 0.0, false));

    osc_defs(&mut defs, OSC_A_BASE, 1.0);
    osc_defs(&mut defs, OSC_B_BASE, 0.0);

    defs.push(def(
        P_FILTER_ENABLE,
        b"Filter Enable\0",
        0.0,
        1.0,
        1.0,
        true,
    ));
    defs.push(def(P_FILTER_TYPE, b"Filter Type\0", 0.0, 3.0, 0.0, true));
    defs.push(def(
        P_FILTER_CUTOFF,
        b"Cutoff\0",
        20.0,
        20_000.0,
        20_000.0,
        false,
    ));
    defs.push(def(P_FILTER_RESO, b"Resonance\0", 0.0, 1.0, 0.15, false));
    defs.push(def(P_FILTER_DRIVE, b"Drive\0", 0.0, 1.0, 0.0, false));
    defs.push(def(P_FILTER_KEYTRACK, b"Keytrack\0", 0.0, 1.0, 0.0, false));
    defs.push(def(P_FILTER_MIX, b"Filter Mix\0", 0.0, 1.0, 1.0, false));
    defs.push(def(P_FILTER_ROUTE_A, b"A To Filter\0", 0.0, 1.0, 1.0, true));
    defs.push(def(P_FILTER_ROUTE_B, b"B To Filter\0", 0.0, 1.0, 1.0, true));

    env_defs(&mut defs, ENV1_BASE, 0.7);
    env_defs(&mut defs, ENV2_BASE, 0.5);
    lfo_defs(&mut defs, LFO1_BASE);
    lfo_defs(&mut defs, LFO2_BASE);

    const SLOT_NAMES: [[&[u8]; 3]; 8] = [
        [b"Mod1 Source\0", b"Mod1 Dest\0", b"Mod1 Amount\0"],
        [b"Mod2 Source\0", b"Mod2 Dest\0", b"Mod2 Amount\0"],
        [b"Mod3 Source\0", b"Mod3 Dest\0", b"Mod3 Amount\0"],
        [b"Mod4 Source\0", b"Mod4 Dest\0", b"Mod4 Amount\0"],
        [b"Mod5 Source\0", b"Mod5 Dest\0", b"Mod5 Amount\0"],
        [b"Mod6 Source\0", b"Mod6 Dest\0", b"Mod6 Amount\0"],
        [b"Mod7 Source\0", b"Mod7 Dest\0", b"Mod7 Amount\0"],
        [b"Mod8 Source\0", b"Mod8 Dest\0", b"Mod8 Amount\0"],
    ];
    for slot in 0..MOD_SLOTS {
        let base = MOD_BASE + slot * MOD_FIELDS;
        let names = &SLOT_NAMES[slot as usize];
        defs.push(def(
            base + MOD_SOURCE,
            names[0],
            0.0,
            (SRC_COUNT - 1) as f64,
            0.0,
            true,
        ));
        defs.push(def(
            base + MOD_DEST,
            names[1],
            0.0,
            (DST_COUNT - 1) as f64,
            0.0,
            true,
        ));
        defs.push(def(base + MOD_AMOUNT, names[2], -1.0, 1.0, 0.0, false));
    }
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_blocks_are_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 4 + 13 * 2 + 9 + 5 * 2 + 4 * 2 + 24);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!((500..=603).contains(&def.id), "id {} out of block", def.id);
            assert!(seen.insert(def.id), "duplicate id {}", def.id);
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }
}
