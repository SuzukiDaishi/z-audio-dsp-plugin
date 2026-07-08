//! The pro EQ's parameter surface.
//!
//! Web ids 700-773 — a fresh block (ring mod 620s … bitcrusher 680s).
//! 700 is the output trim; the 8 bands live at 710 + band*8 with the
//! field offsets below. The original 3-band EQ ids (40-57, mirrored from
//! the z-audio-dsp submodule) are retired with the 0.2 rewrite; the
//! native VST3/CLAP EQ still uses them with its own UI snapshot.

use wclap_plugin::{ParamDef, PARAM_IS_AUTOMATABLE, PARAM_IS_STEPPED};

pub const P_OUTPUT: u32 = 700;

pub const BAND_BASE: u32 = 710;
pub const BAND_COUNT: u32 = 8;
pub const BAND_FIELDS: u32 = 8;

pub const F_ENABLE: u32 = 0;
pub const F_TYPE: u32 = 1;
pub const F_FREQ: u32 = 2;
pub const F_GAIN: u32 = 3;
pub const F_Q: u32 = 4;
pub const F_SLOPE: u32 = 5;
pub const F_PLACEMENT: u32 = 6;
pub const F_SOLO: u32 = 7;

/// Band filter types, in stepped-parameter order.
pub const TYPE_BELL: u8 = 0;
pub const TYPE_LOW_SHELF: u8 = 1;
pub const TYPE_HIGH_SHELF: u8 = 2;
pub const TYPE_LOW_CUT: u8 = 3;
pub const TYPE_HIGH_CUT: u8 = 4;
pub const TYPE_NOTCH: u8 = 5;
pub const TYPE_COUNT: u8 = 6;

/// Cut slopes, in stepped-parameter order: 6 / 12 / 24 / 48 dB per octave.
pub const SLOPE_6: u8 = 0;
pub const SLOPE_12: u8 = 1;
pub const SLOPE_24: u8 = 2;
pub const SLOPE_48: u8 = 3;
pub const SLOPE_COUNT: u8 = 4;

/// Per-band channel placement, in stepped-parameter order.
pub const PLACE_STEREO: u8 = 0;
pub const PLACE_MID: u8 = 1;
pub const PLACE_SIDE: u8 = 2;
pub const PLACE_LEFT: u8 = 3;
pub const PLACE_RIGHT: u8 = 4;
pub const PLACE_COUNT: u8 = 5;

pub const FREQ_MIN: f32 = 10.0;
pub const FREQ_MAX: f32 = 24_000.0;
pub const GAIN_MIN: f32 = -30.0;
pub const GAIN_MAX: f32 = 30.0;
pub const Q_MIN: f32 = 0.1;
pub const Q_MAX: f32 = 30.0;

/// Staggered default frequencies so freshly enabled bands land usefully.
pub const DEFAULT_FREQS: [f32; BAND_COUNT as usize] =
    [30.0, 80.0, 200.0, 500.0, 1_200.0, 3_000.0, 8_000.0, 16_000.0];

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

pub fn param_defs() -> Vec<ParamDef> {
    const NAMES: [[&[u8]; 8]; 8] = [
        [
            b"B1 Enable\0",
            b"B1 Type\0",
            b"B1 Freq\0",
            b"B1 Gain\0",
            b"B1 Q\0",
            b"B1 Slope\0",
            b"B1 Placement\0",
            b"B1 Solo\0",
        ],
        [
            b"B2 Enable\0",
            b"B2 Type\0",
            b"B2 Freq\0",
            b"B2 Gain\0",
            b"B2 Q\0",
            b"B2 Slope\0",
            b"B2 Placement\0",
            b"B2 Solo\0",
        ],
        [
            b"B3 Enable\0",
            b"B3 Type\0",
            b"B3 Freq\0",
            b"B3 Gain\0",
            b"B3 Q\0",
            b"B3 Slope\0",
            b"B3 Placement\0",
            b"B3 Solo\0",
        ],
        [
            b"B4 Enable\0",
            b"B4 Type\0",
            b"B4 Freq\0",
            b"B4 Gain\0",
            b"B4 Q\0",
            b"B4 Slope\0",
            b"B4 Placement\0",
            b"B4 Solo\0",
        ],
        [
            b"B5 Enable\0",
            b"B5 Type\0",
            b"B5 Freq\0",
            b"B5 Gain\0",
            b"B5 Q\0",
            b"B5 Slope\0",
            b"B5 Placement\0",
            b"B5 Solo\0",
        ],
        [
            b"B6 Enable\0",
            b"B6 Type\0",
            b"B6 Freq\0",
            b"B6 Gain\0",
            b"B6 Q\0",
            b"B6 Slope\0",
            b"B6 Placement\0",
            b"B6 Solo\0",
        ],
        [
            b"B7 Enable\0",
            b"B7 Type\0",
            b"B7 Freq\0",
            b"B7 Gain\0",
            b"B7 Q\0",
            b"B7 Slope\0",
            b"B7 Placement\0",
            b"B7 Solo\0",
        ],
        [
            b"B8 Enable\0",
            b"B8 Type\0",
            b"B8 Freq\0",
            b"B8 Gain\0",
            b"B8 Q\0",
            b"B8 Slope\0",
            b"B8 Placement\0",
            b"B8 Solo\0",
        ],
    ];

    let mut defs = Vec::with_capacity(1 + (BAND_COUNT * BAND_FIELDS) as usize);
    defs.push(def(P_OUTPUT, b"Output\0", -24.0, 24.0, 0.0, false));
    for band in 0..BAND_COUNT {
        let base = BAND_BASE + band * BAND_FIELDS;
        let names = &NAMES[band as usize];
        defs.push(def(base + F_ENABLE, names[0], 0.0, 1.0, 0.0, true));
        defs.push(def(
            base + F_TYPE,
            names[1],
            0.0,
            (TYPE_COUNT - 1) as f64,
            0.0,
            true,
        ));
        defs.push(def(
            base + F_FREQ,
            names[2],
            FREQ_MIN as f64,
            FREQ_MAX as f64,
            DEFAULT_FREQS[band as usize] as f64,
            false,
        ));
        defs.push(def(
            base + F_GAIN,
            names[3],
            GAIN_MIN as f64,
            GAIN_MAX as f64,
            0.0,
            false,
        ));
        defs.push(def(
            base + F_Q,
            names[4],
            Q_MIN as f64,
            Q_MAX as f64,
            0.71,
            false,
        ));
        defs.push(def(
            base + F_SLOPE,
            names[5],
            0.0,
            (SLOPE_COUNT - 1) as f64,
            SLOPE_12 as f64,
            true,
        ));
        defs.push(def(
            base + F_PLACEMENT,
            names[6],
            0.0,
            (PLACE_COUNT - 1) as f64,
            0.0,
            true,
        ));
        defs.push(def(base + F_SOLO, names[7], 0.0, 1.0, 0.0, true));
    }
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_block_is_well_formed() {
        let defs = param_defs();
        assert_eq!(defs.len(), 1 + 64);
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!(
                def.id == P_OUTPUT || (710..=773).contains(&def.id),
                "id {} out of block",
                def.id
            );
            assert!(seen.insert(def.id), "duplicate id {}", def.id);
            assert!(def.min < def.max);
            assert!(def.default >= def.min && def.default <= def.max);
            assert!(def.name.ends_with(b"\0"));
        }
    }
}
