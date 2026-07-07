//! UI ↔ plugin binary packets, layered on the `clap.webview/3` channel
//! (the parameter snapshot/set traffic is handled by the `wclap-plugin`
//! scaffold; these packets feed the UI's canvases and preview keyboard).
//!
//! All packets are little-endian and start with a 4-byte magic.
//!
//! Plugin → UI:
//!
//! - `ZWTW` — waveform preview: magic · u8 osc (0=A, 1=B) · u16 sample
//!   count `n` · n × f32 samples of the current morphed single cycle.
//! - `ZWTM` — meter: magic · u8 active voices · f32 env1 · f32 env2 ·
//!   f32 lfo1 · f32 lfo2.
//! - `ZWTS` — wavetable stack for the pseudo-3D view: magic · u8 osc ·
//!   u8 frame count · u16 samples per frame · frames × samples f32,
//!   frame 0 first. Sent on UI open and when the table selection changes.
//!
//! UI → plugin:
//!
//! - `ZWTN` — note preview from the on-screen keyboard: magic ·
//!   u8 on (0/1) · u8 key · u8 velocity (0-127). Parameter edits still
//!   ride the scaffold's standard `{set:[id,value]}` path.

pub const MAGIC_WAVE: &[u8; 4] = b"ZWTW";
pub const MAGIC_METER: &[u8; 4] = b"ZWTM";
pub const MAGIC_NOTE: &[u8; 4] = b"ZWTN";
pub const MAGIC_STACK: &[u8; 4] = b"ZWTS";

/// Samples per frame in a stack packet (coarse — it only feeds the
/// miniature 3D view).
pub const STACK_FRAME_LEN: usize = 64;

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

/// Encode a `ZWTS` stack packet from `frames` rows of equal length.
pub fn encode_stack(osc_b: bool, frames: &[Vec<f32>]) -> Vec<u8> {
    let frame_len = frames.first().map_or(0, |f| f.len());
    let mut out = Vec::with_capacity(8 + frames.len() * frame_len * 4);
    out.extend_from_slice(MAGIC_STACK);
    out.push(osc_b as u8);
    out.push(frames.len() as u8);
    out.extend_from_slice(&(frame_len as u16).to_le_bytes());
    for frame in frames {
        for s in frame {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

/// Decode a `ZWTN` keyboard packet into `(on, key, velocity)`.
pub fn parse_note_preview(bytes: &[u8]) -> Option<(bool, u8, u8)> {
    if bytes.len() != 7 || &bytes[..4] != MAGIC_NOTE {
        return None;
    }
    Some((bytes[4] != 0, bytes[5].min(127), bytes[6].min(127)))
}

/// Encode a `ZWTN` packet (used by tests; the UI builds it in JS).
pub fn encode_note_preview(on: bool, key: u8, velocity: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(7);
    out.extend_from_slice(MAGIC_NOTE);
    out.push(on as u8);
    out.push(key);
    out.push(velocity);
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

    #[test]
    fn note_packet_round_trips_and_rejects_junk() {
        let on = encode_note_preview(true, 60, 100);
        assert_eq!(parse_note_preview(&on), Some((true, 60, 100)));
        let off = encode_note_preview(false, 200, 255);
        assert_eq!(parse_note_preview(&off), Some((false, 127, 127)));
        assert_eq!(parse_note_preview(b"ZWTN"), None);
        assert_eq!(parse_note_preview(b"ZWTMxxx"), None);
        assert_eq!(
            parse_note_preview(&encode_meter(0, 0.0, 0.0, 0.0, 0.0)),
            None
        );
    }
}
