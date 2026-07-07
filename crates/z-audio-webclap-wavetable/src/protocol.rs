//! Plugin → UI visualization packets, layered on the `clap.webview/3`
//! binary channel (the parameter snapshot/set traffic is handled by the
//! `wclap-plugin` scaffold; these packets only feed the UI's canvases).
//!
//! All packets are little-endian and start with a 4-byte magic:
//!
//! - `ZWTW` — waveform preview: magic · u8 osc (0=A, 1=B) · u16 sample
//!   count `n` · n × f32 samples of the current morphed single cycle.
//! - `ZWTM` — meter: magic · u8 active voices · f32 env1 · f32 env2 ·
//!   f32 lfo1 · f32 lfo2.
//!
//! The UI never sends binary messages back; parameter edits ride the
//! scaffold's standard `{set:[id,value]}` path.

pub const MAGIC_WAVE: &[u8; 4] = b"ZWTW";
pub const MAGIC_METER: &[u8; 4] = b"ZWTM";

/// Samples per waveform preview packet.
pub const PREVIEW_LEN: usize = 256;

pub fn encode_wave(osc_b: bool, samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 2 + samples.len() * 4);
    out.extend_from_slice(MAGIC_WAVE);
    out.push(osc_b as u8);
    out.extend_from_slice(&(samples.len() as u16).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn encode_meter(voices: u8, env1: f32, env2: f32, lfo1: f32, lfo2: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 16);
    out.extend_from_slice(MAGIC_METER);
    out.push(voices);
    out.extend_from_slice(&env1.to_le_bytes());
    out.extend_from_slice(&env2.to_le_bytes());
    out.extend_from_slice(&lfo1.to_le_bytes());
    out.extend_from_slice(&lfo2.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_packet_round_trips() {
        let samples: Vec<f32> = (0..PREVIEW_LEN).map(|i| (i as f32) / 256.0).collect();
        let bytes = encode_wave(true, &samples);
        assert_eq!(&bytes[..4], MAGIC_WAVE);
        assert_eq!(bytes[4], 1);
        let n = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
        assert_eq!(n, PREVIEW_LEN);
        let s0 = f32::from_le_bytes(bytes[7..11].try_into().unwrap());
        assert_eq!(s0, 0.0);
        assert_eq!(bytes.len(), 7 + PREVIEW_LEN * 4);
    }

    #[test]
    fn meter_packet_layout() {
        let bytes = encode_meter(3, 0.5, 0.25, -1.0, 1.0);
        assert_eq!(&bytes[..4], MAGIC_METER);
        assert_eq!(bytes[4], 3);
        assert_eq!(bytes.len(), 21);
        let env1 = f32::from_le_bytes(bytes[5..9].try_into().unwrap());
        assert_eq!(env1, 0.5);
    }
}
