//! UI ↔ plugin binary packets, layered on the `clap.webview/3` channel
//! (parameter snapshot/set traffic is handled by the `wclap-plugin`
//! scaffold; these packets feed the band-envelope meter and the preview
//! keyboard). Same framing conventions as the wavetable synth.
//!
//! All packets are little-endian and start with a 4-byte magic.
//!
//! Plugin → UI:
//!
//! - `ZVCM` — meter (~30 Hz): magic · u8 active voices · u8 band count
//!   `n` · n × f32 modulator band envelopes (pre-shift, linear).
//!
//! UI → plugin:
//!
//! - `ZVCN` — note preview from the on-screen keyboard: magic ·
//!   u8 on (0/1) · u8 key · u8 velocity (0-127).

pub const MAGIC_METER: &[u8; 4] = b"ZVCM";
pub const MAGIC_NOTE: &[u8; 4] = b"ZVCN";

pub fn encode_meter(voices: u8, env: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + env.len() * 4);
    out.extend_from_slice(MAGIC_METER);
    out.push(voices);
    out.push(env.len() as u8);
    for v in env {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a `ZVCN` keyboard packet into `(on, key, velocity)`.
pub fn parse_note_preview(bytes: &[u8]) -> Option<(bool, u8, u8)> {
    if bytes.len() != 7 || &bytes[..4] != MAGIC_NOTE {
        return None;
    }
    Some((bytes[4] != 0, bytes[5].min(127), bytes[6].min(127)))
}

/// Encode a `ZVCN` packet (used by tests; the UI builds it in JS).
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
    fn meter_packet_layout() {
        let env = [0.5f32, 0.25, 0.0, 1.0];
        let bytes = encode_meter(3, &env);
        assert_eq!(&bytes[..4], MAGIC_METER);
        assert_eq!(bytes[4], 3);
        assert_eq!(bytes[5], 4);
        assert_eq!(bytes.len(), 6 + 4 * 4);
        let read = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(read(6), 0.5);
        assert_eq!(read(10), 0.25);
        assert_eq!(read(18), 1.0);
    }

    #[test]
    fn note_packet_round_trips_and_rejects_junk() {
        let on = encode_note_preview(true, 60, 100);
        assert_eq!(parse_note_preview(&on), Some((true, 60, 100)));
        let off = encode_note_preview(false, 200, 255);
        assert_eq!(parse_note_preview(&off), Some((false, 127, 127)));
        assert_eq!(parse_note_preview(b"ZVCN"), None);
        assert_eq!(parse_note_preview(b"ZWTNxxx"), None);
        assert_eq!(parse_note_preview(&encode_meter(0, &[0.0; 4])), None);
    }
}
