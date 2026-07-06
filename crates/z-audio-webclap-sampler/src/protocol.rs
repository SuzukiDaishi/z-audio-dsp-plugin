//! Binary UI <-> plugin message protocol for the sampler.
//!
//! Every message starts with the 4-byte magic `b"ZSMP"` followed by a
//! one-byte opcode. Multi-byte fields are little-endian and read
//! unaligned, so the JS side can build packets with a plain `DataView`.
//!
//! UI -> plugin:
//!
//! ```text
//! 0x01 BeginSample   f32 sample_rate · u8 channels · u32 frames
//! 0x02 SampleChunk   u32 float_offset · payload (f32le PCM, interleaved)
//! 0x03 CommitZones   u16 zone_count · zone_count × 40-byte zone record
//! 0x04 NotePreview   u8 on · u8 key · u8 velocity(0-127)
//! 0x05 ClearSample   (no payload)
//! ```
//!
//! Zone record (40 bytes):
//!
//! ```text
//!  0 u8  lokey          1 u8  hikey          2 u8  root
//!  3 u8  flags (bit0 = one-shot)
//!  4 u8  loop_mode (0 off · 1 infinite · 2 sustain · 3 ping-pong · 4 reverse)
//!  5     3 pad bytes
//!  8 u32 start_frame   12 u32 end_frame (exclusive, in source frames)
//! 16 u32 loop_start    20 u32 loop_end (relative to start_frame)
//! 24 f32 gain_db       28 f32 tune_cents
//! 32 f32 pan           36 f32 loop_xfade_s
//! ```
//!
//! Plugin -> UI:
//!
//! ```text
//! 0x81 Status  u8 has_sample · u8 channels · u16 zone_count
//!              · u32 frames · f32 sample_rate
//! ```

pub const MAGIC: &[u8; 4] = b"ZSMP";

pub const OP_BEGIN_SAMPLE: u8 = 0x01;
pub const OP_SAMPLE_CHUNK: u8 = 0x02;
pub const OP_COMMIT_ZONES: u8 = 0x03;
pub const OP_NOTE_PREVIEW: u8 = 0x04;
pub const OP_CLEAR_SAMPLE: u8 = 0x05;
pub const OP_STATUS: u8 = 0x81;

pub const ZONE_RECORD_BYTES: usize = 40;
/// Upper bound on zones per commit; also bounds slice count in the UI.
pub const MAX_ZONES: usize = 128;
/// Upper bound on one uploaded sample: 60s of stereo 48kHz f32 PCM.
pub const MAX_SAMPLE_FLOATS: usize = 48_000 * 60 * 2;

pub const ZONE_FLAG_ONE_SHOT: u8 = 1;

/// One key-mapped slice of the shared source sample, as sent by the UI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneDef {
    pub lokey: u8,
    pub hikey: u8,
    pub root: u8,
    pub one_shot: bool,
    /// 0 off · 1 infinite · 2 sustain · 3 ping-pong · 4 reverse.
    pub loop_mode: u8,
    pub start_frame: u32,
    /// Exclusive; clamped to the source length when the zone is cut.
    pub end_frame: u32,
    /// Relative to `start_frame`.
    pub loop_start: u32,
    pub loop_end: u32,
    pub gain_db: f32,
    pub tune_cents: f32,
    pub pan: f32,
    pub loop_xfade_s: f32,
}

/// A parsed UI -> plugin message. Payload slices borrow from the receive
/// buffer; nothing is copied until the plugin decides to keep it.
#[derive(Debug, PartialEq)]
pub enum UiMessage<'a> {
    BeginSample {
        sample_rate: f32,
        channels: u8,
        frames: u32,
    },
    SampleChunk {
        float_offset: u32,
        /// Raw little-endian f32 bytes (length is a multiple of 4).
        pcm_bytes: &'a [u8],
    },
    CommitZones(alloc_vec::Vec<ZoneDef>),
    NotePreview {
        on: bool,
        key: u8,
        velocity: u8,
    },
    ClearSample,
}

// The plugin crate builds with std; keep an alias so this module could
// move into a no_std context untouched.
mod alloc_vec {
    pub use std::vec::Vec;
}

fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Parses one UI message. `None` means "not ours / malformed" — the caller
/// simply ignores those (the transport also carries the generic
/// `ready`/`{set:…}` messages, which don't start with our magic).
pub fn parse_ui_message(bytes: &[u8]) -> Option<UiMessage<'_>> {
    if bytes.len() < 5 || &bytes[..4] != MAGIC {
        return None;
    }
    let body = &bytes[5..];
    match bytes[4] {
        OP_BEGIN_SAMPLE => {
            let sample_rate = f32_at(body, 0)?;
            let channels = *body.get(4)?;
            let frames = u32_at(body, 5)?;
            if !(8_000.0..=384_000.0).contains(&sample_rate)
                || !(1..=2).contains(&channels)
                || frames == 0
                || frames as usize * channels as usize > MAX_SAMPLE_FLOATS
            {
                return None;
            }
            Some(UiMessage::BeginSample {
                sample_rate,
                channels,
                frames,
            })
        }
        OP_SAMPLE_CHUNK => {
            let float_offset = u32_at(body, 0)?;
            let pcm_bytes = body.get(4..)?;
            if pcm_bytes.is_empty() || pcm_bytes.len() % 4 != 0 {
                return None;
            }
            Some(UiMessage::SampleChunk {
                float_offset,
                pcm_bytes,
            })
        }
        OP_COMMIT_ZONES => {
            let count = u16::from_le_bytes(body.get(..2)?.try_into().ok()?) as usize;
            if count > MAX_ZONES || body.len() < 2 + count * ZONE_RECORD_BYTES {
                return None;
            }
            let mut zones = Vec::with_capacity(count);
            for i in 0..count {
                let r = &body[2 + i * ZONE_RECORD_BYTES..2 + (i + 1) * ZONE_RECORD_BYTES];
                zones.push(ZoneDef {
                    lokey: r[0].min(127),
                    hikey: r[1].min(127),
                    root: r[2].min(127),
                    one_shot: r[3] & ZONE_FLAG_ONE_SHOT != 0,
                    loop_mode: r[4].min(4),
                    start_frame: u32_at(r, 8)?,
                    end_frame: u32_at(r, 12)?,
                    loop_start: u32_at(r, 16)?,
                    loop_end: u32_at(r, 20)?,
                    gain_db: f32_at(r, 24)?.clamp(-48.0, 24.0),
                    tune_cents: f32_at(r, 28)?.clamp(-1_200.0, 1_200.0),
                    pan: f32_at(r, 32)?.clamp(-1.0, 1.0),
                    loop_xfade_s: f32_at(r, 36)?.clamp(0.0, 2.0),
                });
            }
            Some(UiMessage::CommitZones(zones))
        }
        OP_NOTE_PREVIEW => Some(UiMessage::NotePreview {
            on: *body.first()? != 0,
            key: (*body.get(1)?).min(127),
            velocity: (*body.get(2)?).min(127),
        }),
        OP_CLEAR_SAMPLE => Some(UiMessage::ClearSample),
        _ => None,
    }
}

/// Builds the plugin -> UI status packet.
pub fn encode_status(
    has_sample: bool,
    channels: u8,
    zone_count: u16,
    frames: u32,
    sample_rate: f32,
) -> [u8; 17] {
    let mut out = [0u8; 17];
    out[..4].copy_from_slice(MAGIC);
    out[4] = OP_STATUS;
    out[5] = has_sample as u8;
    out[6] = channels;
    out[7..9].copy_from_slice(&zone_count.to_le_bytes());
    out[9..13].copy_from_slice(&frames.to_le_bytes());
    out[13..17].copy_from_slice(&sample_rate.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(op: u8, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(op);
        out.extend_from_slice(body);
        out
    }

    fn zone_record(z: &ZoneDef) -> [u8; ZONE_RECORD_BYTES] {
        let mut r = [0u8; ZONE_RECORD_BYTES];
        r[0] = z.lokey;
        r[1] = z.hikey;
        r[2] = z.root;
        r[3] = if z.one_shot { ZONE_FLAG_ONE_SHOT } else { 0 };
        r[4] = z.loop_mode;
        r[8..12].copy_from_slice(&z.start_frame.to_le_bytes());
        r[12..16].copy_from_slice(&z.end_frame.to_le_bytes());
        r[16..20].copy_from_slice(&z.loop_start.to_le_bytes());
        r[20..24].copy_from_slice(&z.loop_end.to_le_bytes());
        r[24..28].copy_from_slice(&z.gain_db.to_le_bytes());
        r[28..32].copy_from_slice(&z.tune_cents.to_le_bytes());
        r[32..36].copy_from_slice(&z.pan.to_le_bytes());
        r[36..40].copy_from_slice(&z.loop_xfade_s.to_le_bytes());
        r
    }

    pub(crate) fn encode_begin(sample_rate: f32, channels: u8, frames: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&sample_rate.to_le_bytes());
        body.push(channels);
        body.extend_from_slice(&frames.to_le_bytes());
        packet(OP_BEGIN_SAMPLE, &body)
    }

    pub(crate) fn encode_chunk(float_offset: u32, pcm: &[f32]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&float_offset.to_le_bytes());
        for s in pcm {
            body.extend_from_slice(&s.to_le_bytes());
        }
        packet(OP_SAMPLE_CHUNK, &body)
    }

    pub(crate) fn encode_commit(zones: &[ZoneDef]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(zones.len() as u16).to_le_bytes());
        for z in zones {
            body.extend_from_slice(&zone_record(z));
        }
        packet(OP_COMMIT_ZONES, &body)
    }

    fn test_zone() -> ZoneDef {
        ZoneDef {
            lokey: 36,
            hikey: 36,
            root: 36,
            one_shot: true,
            loop_mode: 0,
            start_frame: 100,
            end_frame: 4_100,
            loop_start: 0,
            loop_end: 0,
            gain_db: -3.0,
            tune_cents: 12.5,
            pan: -0.25,
            loop_xfade_s: 0.01,
        }
    }

    #[test]
    fn begin_sample_round_trips() {
        let bytes = encode_begin(44_100.0, 2, 1_234);
        assert_eq!(
            parse_ui_message(&bytes),
            Some(UiMessage::BeginSample {
                sample_rate: 44_100.0,
                channels: 2,
                frames: 1_234,
            })
        );
    }

    #[test]
    fn begin_sample_rejects_bad_shapes() {
        assert_eq!(parse_ui_message(&encode_begin(44_100.0, 3, 100)), None);
        assert_eq!(parse_ui_message(&encode_begin(44_100.0, 0, 100)), None);
        assert_eq!(parse_ui_message(&encode_begin(1.0, 1, 100)), None);
        assert_eq!(parse_ui_message(&encode_begin(44_100.0, 1, 0)), None);
        // Too large to allocate.
        assert_eq!(
            parse_ui_message(&encode_begin(44_100.0, 2, MAX_SAMPLE_FLOATS as u32)),
            None
        );
    }

    #[test]
    fn chunk_round_trips_payload_bytes() {
        let bytes = encode_chunk(8, &[0.5, -0.5]);
        match parse_ui_message(&bytes) {
            Some(UiMessage::SampleChunk {
                float_offset,
                pcm_bytes,
            }) => {
                assert_eq!(float_offset, 8);
                assert_eq!(pcm_bytes.len(), 8);
                assert_eq!(f32::from_le_bytes(pcm_bytes[..4].try_into().unwrap()), 0.5);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn chunk_rejects_ragged_payloads() {
        let mut bytes = encode_chunk(0, &[0.5]);
        bytes.pop();
        assert_eq!(parse_ui_message(&bytes), None);
        // Empty payload.
        assert_eq!(parse_ui_message(&encode_chunk(0, &[])), None);
    }

    #[test]
    fn commit_zones_round_trips() {
        let zone = test_zone();
        let bytes = encode_commit(&[zone]);
        assert_eq!(
            parse_ui_message(&bytes),
            Some(UiMessage::CommitZones(vec![zone]))
        );
    }

    #[test]
    fn commit_zones_rejects_truncated_records_and_too_many_zones() {
        let mut bytes = encode_commit(&[test_zone()]);
        bytes.truncate(bytes.len() - 1);
        assert_eq!(parse_ui_message(&bytes), None);

        let too_many = vec![test_zone(); MAX_ZONES + 1];
        assert_eq!(parse_ui_message(&encode_commit(&too_many)), None);
    }

    #[test]
    fn commit_zones_clamps_out_of_range_fields() {
        let mut zone = test_zone();
        zone.lokey = 200;
        zone.pan = 9.0;
        zone.loop_mode = 99;
        let bytes = encode_commit(&[zone]);
        let Some(UiMessage::CommitZones(zones)) = parse_ui_message(&bytes) else {
            panic!("should parse");
        };
        assert_eq!(zones[0].lokey, 127);
        assert_eq!(zones[0].pan, 1.0);
        assert_eq!(zones[0].loop_mode, 4);
    }

    #[test]
    fn note_preview_parses() {
        let bytes = packet(OP_NOTE_PREVIEW, &[1, 60, 100]);
        assert_eq!(
            parse_ui_message(&bytes),
            Some(UiMessage::NotePreview {
                on: true,
                key: 60,
                velocity: 100,
            })
        );
    }

    #[test]
    fn foreign_messages_are_ignored() {
        assert_eq!(parse_ui_message(b"ready"), None);
        assert_eq!(parse_ui_message(&[0xa1, 0x63]), None);
        assert_eq!(parse_ui_message(&packet(0x7f, &[])), None);
        assert_eq!(parse_ui_message(b"ZSMP"), None);
    }

    #[test]
    fn status_packet_layout_is_stable() {
        let s = encode_status(true, 2, 3, 48_000, 44_100.0);
        assert_eq!(&s[..4], MAGIC);
        assert_eq!(s[4], OP_STATUS);
        assert_eq!(s[5], 1);
        assert_eq!(s[6], 2);
        assert_eq!(u16::from_le_bytes(s[7..9].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(s[9..13].try_into().unwrap()), 48_000);
        assert_eq!(f32::from_le_bytes(s[13..17].try_into().unwrap()), 44_100.0);
    }
}
