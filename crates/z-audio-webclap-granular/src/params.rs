//! The granular synth's parameter surface.
//!
//! Web ids 400-429 — a fresh block (the simple synth uses 100s, retired
//! sampler 200s, multi-zone sampler 300s). The native VST3/CLAP build
//! mirrors these ids one-to-one (see `crates/z-audio-granular-plugin`),
//! which its param-mirror unit test pins against [`param_defs`].

use wclap_plugin::{ParamDef, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED};

pub const P_LEVEL: u32 = 400;
pub const P_PITCH: u32 = 401;
pub const P_FINE: u32 = 402;
pub const P_POSITION: u32 = 403;
pub const P_GRAIN_LENGTH: u32 = 404;
pub const P_LENGTH_KEYTRACK: u32 = 405;
pub const P_GRAIN_ATTACK: u32 = 406;
pub const P_GRAIN_DECAY: u32 = 407;
pub const P_ATTACK_CURVE: u32 = 408;
pub const P_DECAY_CURVE: u32 = 409;
pub const P_SPAWN_MODE: u32 = 410;
pub const P_RATE: u32 = 411;
pub const P_SYNC_RATE: u32 = 412;
pub const P_DENSITY: u32 = 413;
pub const P_ROOT_NOTE: u32 = 414;
pub const P_ALIGN_PHASES: u32 = 415;
pub const P_WARM_START: u32 = 416;
pub const P_RANDOM_POSITION: u32 = 417;
pub const P_RANDOM_TIMING: u32 = 418;
pub const P_RANDOM_PITCH: u32 = 419;
pub const P_RANDOM_LEVEL: u32 = 420;
pub const P_RANDOM_PAN: u32 = 421;
pub const P_RANDOM_REVERSE: u32 = 422;
pub const P_CHORD_TYPE: u32 = 423;
pub const P_CHORD_RANGE: u32 = 424;
pub const P_CHORD_PATTERN: u32 = 425;
pub const P_AMP_ATTACK: u32 = 426;
pub const P_AMP_DECAY: u32 = 427;
pub const P_AMP_SUSTAIN: u32 = 428;
pub const P_AMP_RELEASE: u32 = 429;

/// Spawn-rate beat lengths for Sync mode, indexed by `P_SYNC_RATE`
/// (16 bars-ish down to a 64th: 16, 8, 4, 2, 1, 1/2, 1/4, 1/8, 1/16 beats).
pub const SYNC_BEATS: [f64; 9] = [16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125, 0.0625];

pub fn param_defs() -> Vec<ParamDef> {
    fn def(
        id: u32,
        name: &'static [u8],
        min: f64,
        max: f64,
        default: f64,
        stepped: bool,
    ) -> ParamDef {
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
    vec![
        def(P_LEVEL, b"Level\0", 0.0, 2.0, 1.0, false),
        def(P_PITCH, b"Pitch\0", -48.0, 48.0, 0.0, true),
        def(P_FINE, b"Fine\0", -100.0, 100.0, 0.0, false),
        def(P_POSITION, b"Position\0", 0.0, 1.0, 0.0, false),
        def(P_GRAIN_LENGTH, b"Grain Length\0", 2.0, 1000.0, 100.0, false),
        def(P_LENGTH_KEYTRACK, b"Length Keytrack\0", 0.0, 1.0, 0.0, true),
        def(P_GRAIN_ATTACK, b"Grain Attack\0", 0.0, 1.0, 0.5, false),
        def(P_GRAIN_DECAY, b"Grain Decay\0", 0.0, 1.0, 0.5, false),
        def(P_ATTACK_CURVE, b"Attack Curve\0", -1.0, 1.0, 0.0, false),
        def(P_DECAY_CURVE, b"Decay Curve\0", -1.0, 1.0, 0.0, false),
        def(P_SPAWN_MODE, b"Spawn Mode\0", 0.0, 2.0, 0.0, true),
        def(P_RATE, b"Rate\0", 0.1, 400.0, 25.0, false),
        def(P_SYNC_RATE, b"Sync Rate\0", 0.0, 8.0, 4.0, true),
        def(P_DENSITY, b"Density\0", 0.5, 64.0, 8.0, false),
        def(P_ROOT_NOTE, b"Root Note\0", 0.0, 127.0, 60.0, true),
        def(P_ALIGN_PHASES, b"Align Phases\0", 0.0, 1.0, 0.0, true),
        def(P_WARM_START, b"Warm Start\0", 0.0, 1.0, 0.0, true),
        def(
            P_RANDOM_POSITION,
            b"Random Position\0",
            0.0,
            2000.0,
            0.0,
            false,
        ),
        def(P_RANDOM_TIMING, b"Random Timing\0", 0.0, 1.0, 0.0, false),
        def(P_RANDOM_PITCH, b"Random Pitch\0", 0.0, 24.0, 0.0, false),
        def(P_RANDOM_LEVEL, b"Random Level\0", 0.0, 1.0, 0.0, false),
        def(P_RANDOM_PAN, b"Random Pan\0", 0.0, 1.0, 0.0, false),
        def(P_RANDOM_REVERSE, b"Random Reverse\0", 0.0, 1.0, 0.0, false),
        def(P_CHORD_TYPE, b"Chord Type\0", 0.0, 9.0, 0.0, true),
        def(P_CHORD_RANGE, b"Chord Range\0", 1.0, 4.0, 1.0, true),
        def(P_CHORD_PATTERN, b"Chord Pattern\0", 0.0, 3.0, 0.0, true),
        def(P_AMP_ATTACK, b"Amp Attack\0", 0.001, 5.0, 0.002, false),
        def(P_AMP_DECAY, b"Amp Decay\0", 0.0, 5.0, 0.0, false),
        def(P_AMP_SUSTAIN, b"Amp Sustain\0", 0.0, 1.0, 1.0, false),
        def(P_AMP_RELEASE, b"Amp Release\0", 0.01, 10.0, 0.25, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_the_contiguous_400_block() {
        let defs = param_defs();
        assert_eq!(defs.len(), 30);
        for (i, def) in defs.iter().enumerate() {
            assert_eq!(def.id, 400 + i as u32, "web ids must stay contiguous");
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }
}
