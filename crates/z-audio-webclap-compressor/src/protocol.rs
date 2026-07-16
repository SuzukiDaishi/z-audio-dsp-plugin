//! UI ↔ plugin binary packets on the `clap.webview/3` channel (param
//! snapshot/set traffic is handled by the `wclap-plugin` scaffold; this
//! feeds the live gain-reduction meter). Little-endian, 4-byte magic —
//! same framing conventions as the vocoder/wavetable plugins.
//!
//! Plugin → UI:
//!
//! - `ZCGR` — meter (~30 Hz): magic · f32 gain reduction (positive dB) ·
//!   f32 input peak (dBFS) · f32 output peak (dBFS).

pub const MAGIC_METER: &[u8; 4] = b"ZCGR";

pub fn encode_meter(gr_db: f32, in_peak_db: f32, out_peak_db: f32) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(MAGIC_METER);
    out[4..8].copy_from_slice(&gr_db.to_le_bytes());
    out[8..12].copy_from_slice(&in_peak_db.to_le_bytes());
    out[12..16].copy_from_slice(&out_peak_db.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_packet_layout() {
        let bytes = encode_meter(12.5, -6.0, -18.5);
        assert_eq!(&bytes[..4], MAGIC_METER);
        let read = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(read(4), 12.5);
        assert_eq!(read(8), -6.0);
        assert_eq!(read(12), -18.5);
    }
}
