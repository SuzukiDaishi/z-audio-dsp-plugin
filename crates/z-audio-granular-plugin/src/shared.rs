//! UI <-> audio-thread bridge.
//!
//! Both editors (the wry webview on Windows/macOS and the egui fallback
//! elsewhere) run on GUI threads, so everything expensive — chunked PCM
//! assembly — happens here, and the audio thread only `try_lock`s
//! prepared results at the top of each block.
//!
//! The webview editor forwards the exact WebCLAP `ZGRN` packets its UI
//! emits (see `z_audio_webclap_granular::protocol`), so [`on_ui_message`]
//! mirrors the wasm build's `on_ui_message` dispatch one-to-one; the egui
//! editor skips the packet layer and calls [`queue_commit`]/[`queue_clear`]
//! directly.
//!
//! Grain activity flows the other way: the audio thread `try_lock`-writes
//! a snapshot each block, and the UI polls it with `OP_POLL_ACTIVITY`
//! (the wry bridge is request/response — the plugin cannot push).
//!
//! [`on_ui_message`]: GranularShared::on_ui_message
//! [`queue_commit`]: GranularShared::queue_commit
//! [`queue_clear`]: GranularShared::queue_clear

use std::sync::{Mutex, MutexGuard};

use z_audio_webclap_granular::engine::{GranularEngine, SourceSample};
use z_audio_webclap_granular::protocol::{self, UiMessage, MAX_ACTIVITY_GRAINS};

/// The WebCLAP scaffold's `ready` packet (CBOR text "ready"); the webview
/// bridge forwards it when the UI (re)opens, and the reply is a status
/// push — the same contract as the wasm build.
// The packet path is only reachable from the Windows/macOS webview editor
// (and the unit tests); the egui fallback calls queue_commit/queue_clear
// directly, hence the target-gated dead_code allowances below.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub const READY_PACKET: &[u8] = b"\x65ready";

/// Bounded queue of editor keyboard preview notes; extras are dropped
/// (previews are cosmetic, and an unread queue means no audio thread is
/// running anyway).
const MAX_PENDING_NOTES: usize = 64;

/// A prepared engine update, applied by the audio thread.
pub enum GranularUpdate {
    /// New source PCM from a completed chunk upload or a native decode.
    Commit(SourceSample),
    Clear,
}

/// One editor keyboard preview note.
pub struct NotePreview {
    pub on: bool,
    pub key: u8,
    pub velocity: u8,
}

/// Editor-side mirror of the engine status, kept in step with the queued
/// updates (i.e. it reports what the engine will play once the pending
/// update is applied) so status replies never touch the audio thread.
#[derive(Clone, Copy)]
pub struct SharedStatus {
    pub has_sample: bool,
    pub channels: u8,
    pub frames: u32,
    pub sample_rate: f32,
}

/// Latest grain positions, written by the audio thread each block.
#[derive(Clone, Copy)]
struct ActivitySnapshot {
    count: usize,
    positions: [f32; MAX_ACTIVITY_GRAINS],
}

/// An in-progress chunked PCM upload (protocol `BeginSample`/`SampleChunk`),
/// assembled on the editor thread.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
struct Upload {
    sample_rate: f32,
    channels: u8,
    data: Vec<f32>,
}

pub struct GranularShared {
    /// Latest not-yet-applied update; the audio thread takes it via
    /// `try_lock` at the top of each block.
    update: Mutex<Option<GranularUpdate>>,
    notes: Mutex<Vec<NotePreview>>,
    staging: Mutex<Option<Upload>>,
    status: Mutex<SharedStatus>,
    activity: Mutex<ActivitySnapshot>,
}

impl GranularShared {
    /// Builds the bridge with its status mirror seeded from the engine's
    /// current (startup) state.
    pub fn mirroring(engine: &GranularEngine) -> Self {
        let (frames, sample_rate, channels) = engine.source_info();
        Self {
            update: Mutex::new(None),
            notes: Mutex::new(Vec::with_capacity(MAX_PENDING_NOTES)),
            staging: Mutex::new(None),
            status: Mutex::new(SharedStatus {
                has_sample: engine.has_sample(),
                channels,
                frames,
                sample_rate,
            }),
            activity: Mutex::new(ActivitySnapshot {
                count: 0,
                positions: [0.0; MAX_ACTIVITY_GRAINS],
            }),
        }
    }

    pub fn status(&self) -> SharedStatus {
        *lock(&self.status)
    }

    /// Handles one UI -> plugin packet on the editor thread; `reply` sends
    /// plugin -> UI packets back over the same transport. Same dispatch as
    /// the WebCLAP build's `on_ui_message`.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    pub fn on_ui_message(&self, bytes: &[u8], reply: &mut dyn FnMut(&[u8])) {
        if bytes == READY_PACKET {
            self.push_status(reply);
            return;
        }
        let Some(msg) = protocol::parse_ui_message(bytes) else {
            return;
        };
        match msg {
            UiMessage::BeginSample {
                sample_rate,
                channels,
                frames,
            } => {
                *lock(&self.staging) = Some(Upload {
                    sample_rate,
                    channels,
                    data: vec![0.0; frames as usize * channels.max(1) as usize],
                });
            }
            UiMessage::SampleChunk {
                float_offset,
                pcm_bytes,
            } => self.stage_chunk(float_offset, pcm_bytes),
            UiMessage::CommitSample => {
                if let Some(upload) = lock(&self.staging).take() {
                    self.queue_commit(SourceSample {
                        sample_rate: upload.sample_rate,
                        channels: upload.channels,
                        data: upload.data,
                    });
                }
                self.push_status(reply);
            }
            UiMessage::NotePreview { on, key, velocity } => {
                let mut notes = lock(&self.notes);
                if notes.len() < MAX_PENDING_NOTES {
                    notes.push(NotePreview { on, key, velocity });
                }
            }
            UiMessage::ClearSample => {
                self.queue_clear();
                self.push_status(reply);
            }
            UiMessage::PollActivity => {
                let snap = *lock(&self.activity);
                reply(&protocol::encode_activity(&snap.positions[..snap.count]));
            }
        }
    }

    /// Same bounds/finite-clamp rules as `GranularEngine::upload_chunk`,
    /// duplicated here because assembly must happen off the audio thread.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    fn stage_chunk(&self, float_offset: u32, pcm_bytes: &[u8]) {
        let mut staging = lock(&self.staging);
        let Some(upload) = staging.as_mut() else {
            return;
        };
        let offset = float_offset as usize;
        let count = pcm_bytes.len() / 4;
        let Some(dst) = upload.data.get_mut(offset..offset.saturating_add(count)) else {
            // Out-of-bounds chunk: drop the whole transfer rather than
            // committing a sample with silent holes.
            *staging = None;
            return;
        };
        for (i, out) in dst.iter_mut().enumerate() {
            let b = &pcm_bytes[i * 4..i * 4 + 4];
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            *out = if v.is_finite() {
                v.clamp(-4.0, 4.0)
            } else {
                0.0
            };
        }
    }

    /// Queues new source PCM and updates the status mirror.
    pub fn queue_commit(&self, source: SourceSample) {
        {
            let mut status = lock(&self.status);
            status.has_sample = true;
            status.channels = source.channels;
            status.frames = source.frames() as u32;
            status.sample_rate = source.sample_rate;
        }
        *lock(&self.update) = Some(GranularUpdate::Commit(source));
    }

    pub fn queue_clear(&self) {
        *lock(&self.staging) = None;
        *lock(&self.status) = SharedStatus {
            has_sample: false,
            channels: 0,
            frames: 0,
            sample_rate: 0.0,
        };
        *lock(&self.update) = Some(GranularUpdate::Clear);
    }

    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    fn push_status(&self, reply: &mut dyn FnMut(&[u8])) {
        let s = self.status();
        reply(&protocol::encode_status(
            s.has_sample,
            s.channels,
            s.frames,
            s.sample_rate,
        ));
    }

    // -- audio-thread side --------------------------------------------------

    /// Non-blocking: a held lock just means the editor is mid-update; the
    /// next block picks it up.
    pub fn take_update(&self) -> Option<GranularUpdate> {
        self.update.try_lock().ok().and_then(|mut slot| slot.take())
    }

    /// Non-blocking; drains queued keyboard previews into `into`.
    pub fn drain_notes(&self, into: &mut Vec<NotePreview>) {
        if let Ok(mut notes) = self.notes.try_lock() {
            into.append(&mut notes);
        }
    }

    /// Non-blocking; the audio thread publishes this block's grain
    /// positions for the UI's activity display.
    pub fn store_activity(&self, positions: &[f32]) {
        if let Ok(mut snap) = self.activity.try_lock() {
            let count = positions.len().min(MAX_ACTIVITY_GRAINS);
            snap.positions[..count].copy_from_slice(&positions[..count]);
            snap.count = count;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use z_audio_webclap_granular::protocol::{
        encode_begin, encode_chunk, encode_clear, encode_commit, encode_note_preview,
        encode_poll_activity, encode_status, OP_ACTIVITY, OP_STATUS,
    };

    fn fresh() -> GranularShared {
        GranularShared::mirroring(&GranularEngine::new(48_000.0))
    }

    fn no_reply() -> impl FnMut(&[u8]) {
        |_bytes: &[u8]| {}
    }

    #[test]
    fn ready_replies_with_the_mirrored_status() {
        let mut engine = GranularEngine::new(48_000.0);
        engine.set_source(SourceSample {
            sample_rate: 44_100.0,
            channels: 1,
            data: vec![0.5; 1_000],
        });
        let shared = GranularShared::mirroring(&engine);

        let mut replies: Vec<Vec<u8>> = Vec::new();
        shared.on_ui_message(READY_PACKET, &mut |bytes| replies.push(bytes.to_vec()));
        assert_eq!(
            replies,
            vec![encode_status(true, 1, 1_000, 44_100.0).to_vec()]
        );
    }

    #[test]
    fn upload_and_commit_produce_a_source_update() {
        let shared = fresh();
        let pcm: Vec<f32> = (0..1_000).map(|i| (i as f32 / 1_000.0) - 0.5).collect();
        shared.on_ui_message(&encode_begin(44_100.0, 1, 1_000), &mut no_reply());
        for (index, chunk) in pcm.chunks(256).enumerate() {
            shared.on_ui_message(&encode_chunk((index * 256) as u32, chunk), &mut no_reply());
        }
        let mut replies: Vec<Vec<u8>> = Vec::new();
        shared.on_ui_message(&encode_commit(), &mut |bytes| replies.push(bytes.to_vec()));

        let Some(GranularUpdate::Commit(source)) = shared.take_update() else {
            panic!("expected a commit with source PCM");
        };
        assert_eq!(source.frames(), 1_000);
        assert_eq!(source.sample_rate, 44_100.0);
        assert_eq!(source.data, pcm);
        assert_eq!(
            replies,
            vec![encode_status(true, 1, 1_000, 44_100.0).to_vec()]
        );
        assert!(shared.take_update().is_none(), "update is consumed once");
    }

    #[test]
    fn out_of_bounds_chunk_aborts_the_upload() {
        let shared = fresh();
        shared.on_ui_message(&encode_begin(44_100.0, 1, 100), &mut no_reply());
        shared.on_ui_message(&encode_chunk(90, &[0.5; 16]), &mut no_reply());
        let mut replies: Vec<Vec<u8>> = Vec::new();
        shared.on_ui_message(&encode_commit(), &mut |bytes| replies.push(bytes.to_vec()));
        assert!(
            shared.take_update().is_none(),
            "poisoned upload must not become a source"
        );
        assert!(!shared.status().has_sample);
        assert_eq!(replies[0][4], OP_STATUS);
    }

    #[test]
    fn note_previews_queue_and_drain() {
        let shared = fresh();
        shared.on_ui_message(&encode_note_preview(true, 60, 100), &mut no_reply());
        shared.on_ui_message(&encode_note_preview(false, 60, 0), &mut no_reply());
        let mut notes = Vec::new();
        shared.drain_notes(&mut notes);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].on && notes[0].key == 60 && notes[0].velocity == 100);
        assert!(!notes[1].on);
        shared.drain_notes(&mut notes);
        assert_eq!(notes.len(), 2, "queue drains once");
    }

    #[test]
    fn clear_zeroes_the_status_and_queues_a_clear() {
        let shared = fresh();
        shared.queue_commit(SourceSample {
            sample_rate: 48_000.0,
            channels: 2,
            data: vec![0.0; 256],
        });
        let mut replies: Vec<Vec<u8>> = Vec::new();
        shared.on_ui_message(&encode_clear(), &mut |bytes| replies.push(bytes.to_vec()));
        assert!(matches!(shared.take_update(), Some(GranularUpdate::Clear)));
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0][4], OP_STATUS);
        assert!(!shared.status().has_sample);
    }

    #[test]
    fn activity_poll_replies_with_the_stored_snapshot() {
        let shared = fresh();
        shared.store_activity(&[0.25, 0.5, 0.75]);
        let mut replies: Vec<Vec<u8>> = Vec::new();
        shared.on_ui_message(&encode_poll_activity(), &mut |bytes| {
            replies.push(bytes.to_vec())
        });
        assert_eq!(replies.len(), 1);
        let pkt = &replies[0];
        assert_eq!(pkt[4], OP_ACTIVITY);
        assert_eq!(pkt[5], 3);
        assert_eq!(f32::from_le_bytes(pkt[6..10].try_into().unwrap()), 0.25);
        assert_eq!(f32::from_le_bytes(pkt[10..14].try_into().unwrap()), 0.5);
    }
}
