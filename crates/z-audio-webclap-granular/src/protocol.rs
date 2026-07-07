//! Binary UI <-> plugin message protocol for the granular synth.
//!
//! A trimmed cousin of the sampler's `ZSMP` protocol (no zone table — the
//! granular engine plays one whole source sample). Every message starts
//! with the 4-byte magic `b"ZGRN"` followed by a one-byte opcode.
//! Multi-byte fields are little-endian and read unaligned, so the JS side
//! can build packets with a plain `DataView`.
//!
//! UI -> plugin:
//!
//! ```text
//! 0x01 BeginSample   f32 sample_rate · u8 channels · u32 frames
//! 0x02 SampleChunk   u32 float_offset · payload (f32le PCM, interleaved)
//! 0x03 CommitSample  (no payload — the pending upload becomes the source)
//! 0x04 NotePreview   u8 on · u8 key · u8 velocity(0-127)
//! 0x05 ClearSample   (no payload)
//! 0x06 PollActivity  (no payload — native webview polls; WebCLAP pushes)
//! ```
//!
//! Plugin -> UI:
//!
//! ```text
//! 0x81 Status    u8 has_sample · u8 channels · u32 frames · f32 sample_rate
//! 0x82 Activity  u8 grain_count · grain_count × f32 normalized position
//! ```

pub const MAGIC: &[u8; 4] = b"ZGRN";

pub const OP_BEGIN_SAMPLE: u8 = 0x01;
pub const OP_SAMPLE_CHUNK: u8 = 0x02;
pub const OP_COMMIT_SAMPLE: u8 = 0x03;
pub const OP_NOTE_PREVIEW: u8 = 0x04;
pub const OP_CLEAR_SAMPLE: u8 = 0x05;
pub const OP_POLL_ACTIVITY: u8 = 0x06;
pub const OP_STATUS: u8 = 0x81;
pub const OP_ACTIVITY: u8 = 0x82;

/// Upper bound on one uploaded sample: 60s of stereo 48kHz f32 PCM.
pub const MAX_SAMPLE_FLOATS: usize = 48_000 * 60 * 2;
/// Most grain positions one Activity packet carries.
pub const MAX_ACTIVITY_GRAINS: usize = 32;

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
    CommitSample,
    NotePreview {
        on: bool,
        key: u8,
        velocity: u8,
    },
    ClearSample,
    PollActivity,
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
        OP_COMMIT_SAMPLE => Some(UiMessage::CommitSample),
        OP_NOTE_PREVIEW => Some(UiMessage::NotePreview {
            on: *body.first()? != 0,
            key: (*body.get(1)?).min(127),
            velocity: (*body.get(2)?).min(127),
        }),
        OP_CLEAR_SAMPLE => Some(UiMessage::ClearSample),
        OP_POLL_ACTIVITY => Some(UiMessage::PollActivity),
        _ => None,
    }
}

/// Builds the plugin -> UI status packet.
pub fn encode_status(has_sample: bool, channels: u8, frames: u32, sample_rate: f32) -> [u8; 15] {
    let mut out = [0u8; 15];
    out[..4].copy_from_slice(MAGIC);
    out[4] = OP_STATUS;
    out[5] = has_sample as u8;
    out[6] = channels;
    out[7..11].copy_from_slice(&frames.to_le_bytes());
    out[11..15].copy_from_slice(&sample_rate.to_le_bytes());
    out
}

/// Builds the plugin -> UI grain-activity packet from normalized (0..1)
/// grain positions; anything past [`MAX_ACTIVITY_GRAINS`] is dropped.
pub fn encode_activity(positions: &[f32]) -> Vec<u8> {
    let count = positions.len().min(MAX_ACTIVITY_GRAINS);
    let mut out = Vec::with_capacity(6 + count * 4);
    out.extend_from_slice(MAGIC);
    out.push(OP_ACTIVITY);
    out.push(count as u8);
    for p in &positions[..count] {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out
}

// -- UI -> plugin encoders --------------------------------------------------
//
// The shipping UI encodes these packets in JS (`ui/main.js`); the Rust
// encoders exist for this crate's unit tests and for the native plugin's
// tests, which drive its UI<->audio bridge with real packets.

fn packet(op: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(op);
    out.extend_from_slice(body);
    out
}

pub fn encode_begin(sample_rate: f32, channels: u8, frames: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&sample_rate.to_le_bytes());
    body.push(channels);
    body.extend_from_slice(&frames.to_le_bytes());
    packet(OP_BEGIN_SAMPLE, &body)
}

pub fn encode_chunk(float_offset: u32, pcm: &[f32]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&float_offset.to_le_bytes());
    for s in pcm {
        body.extend_from_slice(&s.to_le_bytes());
    }
    packet(OP_SAMPLE_CHUNK, &body)
}

pub fn encode_commit() -> Vec<u8> {
    packet(OP_COMMIT_SAMPLE, &[])
}

pub fn encode_note_preview(on: bool, key: u8, velocity: u8) -> Vec<u8> {
    packet(OP_NOTE_PREVIEW, &[on as u8, key, velocity])
}

pub fn encode_clear() -> Vec<u8> {
    packet(OP_CLEAR_SAMPLE, &[])
}

pub fn encode_poll_activity() -> Vec<u8> {
    packet(OP_POLL_ACTIVITY, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn payloadless_opcodes_parse() {
        assert_eq!(
            parse_ui_message(&encode_commit()),
            Some(UiMessage::CommitSample)
        );
        assert_eq!(
            parse_ui_message(&encode_clear()),
            Some(UiMessage::ClearSample)
        );
        assert_eq!(
            parse_ui_message(&encode_poll_activity()),
            Some(UiMessage::PollActivity)
        );
    }

    #[test]
    fn note_preview_parses() {
        let bytes = encode_note_preview(true, 60, 100);
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
        assert_eq!(parse_ui_message(b"ZGRN"), None);
        assert_eq!(parse_ui_message(b"ZSMP\x01"), None);
    }

    #[test]
    fn status_packet_layout_is_stable() {
        let s = encode_status(true, 2, 48_000, 44_100.0);
        assert_eq!(&s[..4], MAGIC);
        assert_eq!(s[4], OP_STATUS);
        assert_eq!(s[5], 1);
        assert_eq!(s[6], 2);
        assert_eq!(u32::from_le_bytes(s[7..11].try_into().unwrap()), 48_000);
        assert_eq!(f32::from_le_bytes(s[11..15].try_into().unwrap()), 44_100.0);
    }

    #[test]
    fn activity_packet_caps_and_encodes_positions() {
        let positions: Vec<f32> = (0..40).map(|i| i as f32 / 40.0).collect();
        let pkt = encode_activity(&positions);
        assert_eq!(&pkt[..4], MAGIC);
        assert_eq!(pkt[4], OP_ACTIVITY);
        assert_eq!(pkt[5] as usize, MAX_ACTIVITY_GRAINS);
        assert_eq!(pkt.len(), 6 + MAX_ACTIVITY_GRAINS * 4);
        let first = f32::from_le_bytes(pkt[6..10].try_into().unwrap());
        assert_eq!(first, 0.0);
    }
}
