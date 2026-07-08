//! Multi-zone sampler instrument: one shared source sample cut into
//! key-mapped zones (Logic-style Classic / One-Shot / Slice mapping is
//! decided by the UI; this engine just plays whatever zone table it's
//! given).
//!
//! Built on `z_audio_dsp::SamplerEngine`. Each committed [`ZoneDef`] is cut
//! into its own [`SampleBuffer`] (so non-looped playback naturally stops at
//! the zone's end, with the engine's end-of-sample declick fade), and
//! per-note triggering matches zones by key range. Global ADSR / gain /
//! tune / velocity parameters are baked into cached [`SampleRegion`]s that
//! are rebuilt lazily when a parameter actually changes.

use std::sync::Arc;

use z_audio_dsp::{
    db_to_linear, flush_denormal, LoopMode, SampleBuffer, SampleRegion, SamplerEngine,
    SamplerEngineConfig, TriggerKind,
};

use crate::protocol::ZoneDef;

/// Simultaneous notes; each note may trigger several overlapping zones.
const MAX_VOICES: usize = 32;
/// Fixed floor for the attack so the very first samples never click.
const MIN_ATTACK_S: f32 = 0.001;
/// Baseline release baked into regions; the audible release is this times
/// the `release_time_scale` passed at trigger time (i.e. the scale IS the
/// seconds value).
const BASE_AMPEG_RELEASE_S: f32 = 1.0;

/// Global (non-zone) parameters, mirrored 1:1 by the CLAP param surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalParams {
    pub master_gain_db: f32,
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
    pub tune_cents: f32,
    pub transpose_semitones: f32,
    pub velocity_amount: f32,
    pub stereo_width: f32,
}

impl Default for GlobalParams {
    fn default() -> Self {
        Self {
            master_gain_db: 0.0,
            attack_s: 0.002,
            decay_s: 0.0,
            sustain: 1.0,
            release_s: 0.25,
            tune_cents: 0.0,
            transpose_semitones: 0.0,
            velocity_amount: 1.0,
            stereo_width: 1.0,
        }
    }
}

/// The uploaded (or embedded) source sample, kept so zones can be re-cut
/// when the UI edits markers without re-uploading PCM.
pub struct SourceSample {
    pub sample_rate: f32,
    pub channels: u8,
    /// Interleaved when stereo.
    pub data: Vec<f32>,
}

impl SourceSample {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
}

/// An in-progress chunked PCM upload (protocol `BeginSample`/`SampleChunk`).
struct Upload {
    sample_rate: f32,
    channels: u8,
    data: Vec<f32>,
}

/// One zone after cutting: the def plus its own PCM slice.
struct CutZone {
    def: ZoneDef,
    buffer: SampleBuffer,
}

pub struct ZoneSampler {
    sample_rate: f32,
    engine: SamplerEngine,
    params: GlobalParams,
    source: Option<SourceSample>,
    zone_defs: Vec<ZoneDef>,
    cut_zones: Vec<CutZone>,
    /// Cached trigger regions, one per cut zone; rebuilt when params change.
    regions: Vec<Arc<SampleRegion>>,
    regions_dirty: bool,
    upload: Option<Upload>,
}

impl ZoneSampler {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            engine: SamplerEngine::new(SamplerEngineConfig {
                sample_rate: sample_rate.max(1.0),
                max_voices: MAX_VOICES,
            }),
            params: GlobalParams::default(),
            source: None,
            zone_defs: Vec::new(),
            cut_zones: Vec::new(),
            regions: Vec::new(),
            regions_dirty: true,
            upload: None,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.reset_voices();
    }

    /// Drops all active voices (host reset), keeping the sample and zones.
    pub fn reset_voices(&mut self) {
        self.engine = SamplerEngine::new(SamplerEngineConfig {
            sample_rate: self.sample_rate,
            max_voices: MAX_VOICES,
        });
        self.regions_dirty = true;
    }

    pub fn params(&self) -> &GlobalParams {
        &self.params
    }

    pub fn set_params(&mut self, params: GlobalParams) {
        if params != self.params {
            self.params = params;
            self.regions_dirty = true;
        }
    }

    pub fn has_sample(&self) -> bool {
        self.source.is_some()
    }

    pub fn source_info(&self) -> (u32, f32, u8) {
        match &self.source {
            Some(s) => (s.frames() as u32, s.sample_rate, s.channels),
            None => (0, 0.0, 0),
        }
    }

    pub fn zone_count(&self) -> usize {
        self.zone_defs.len()
    }

    #[cfg_attr(not(test), allow(dead_code))] // exercised by unit tests
    pub fn active_voice_count(&self) -> usize {
        self.engine.active_voice_count()
    }

    /// Installs a new source sample and cuts the current zone table from
    /// it. Replaces any previous source.
    pub fn set_source(&mut self, source: SourceSample, zones: Vec<ZoneDef>) {
        self.source = Some(source);
        self.zone_defs = zones;
        self.recut();
    }

    /// Replaces the zone table, re-cutting from the existing source.
    pub fn set_zones(&mut self, zones: Vec<ZoneDef>) {
        self.zone_defs = zones;
        self.recut();
    }

    pub fn clear(&mut self) {
        self.source = None;
        self.upload = None;
        self.zone_defs.clear();
        self.cut_zones.clear();
        self.regions.clear();
        self.regions_dirty = true;
    }

    // -- chunked upload ---------------------------------------------------

    pub fn begin_upload(&mut self, sample_rate: f32, channels: u8, frames: u32) {
        self.upload = Some(Upload {
            sample_rate,
            channels,
            data: vec![0.0; frames as usize * channels.max(1) as usize],
        });
    }

    pub fn upload_chunk(&mut self, float_offset: u32, pcm_bytes: &[u8]) {
        let Some(upload) = self.upload.as_mut() else {
            return;
        };
        let offset = float_offset as usize;
        let count = pcm_bytes.len() / 4;
        let Some(dst) = upload.data.get_mut(offset..offset.saturating_add(count)) else {
            // Out-of-bounds chunk: drop the whole transfer rather than
            // committing a sample with silent holes.
            self.upload = None;
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

    /// Commits a zone table. If a chunked upload is pending it becomes the
    /// new source; otherwise the zones re-cut the existing source.
    pub fn commit_zones(&mut self, zones: Vec<ZoneDef>) {
        if let Some(upload) = self.upload.take() {
            self.set_source(
                SourceSample {
                    sample_rate: upload.sample_rate,
                    channels: upload.channels,
                    data: upload.data,
                },
                zones,
            );
        } else {
            self.set_zones(zones);
        }
    }

    // -- zone cutting / region building ------------------------------------

    fn recut(&mut self) {
        self.cut_zones.clear();
        let Some(source) = &self.source else {
            self.regions.clear();
            self.regions_dirty = true;
            return;
        };
        let channels = source.channels.max(1) as usize;
        let source_frames = source.frames();
        for def in &self.zone_defs {
            let start = (def.start_frame as usize).min(source_frames);
            let end = (def.end_frame as usize).clamp(start, source_frames);
            if end - start < 2 {
                continue;
            }
            let data = source.data[start * channels..end * channels].to_vec();
            self.cut_zones.push(CutZone {
                def: *def,
                buffer: SampleBuffer::new(source.sample_rate, source.channels, data),
            });
        }
        self.regions_dirty = true;
    }

    fn rebuild_regions(&mut self) {
        let p = self.params;
        self.regions = self
            .cut_zones
            .iter()
            .map(|zone| {
                let def = &zone.def;
                let frames = zone.buffer.frames();
                let loop_start = (def.loop_start as usize).min(frames.saturating_sub(2));
                let loop_end = (def.loop_end as usize).clamp(loop_start + 1, frames.max(1));
                let loop_xfade_frames =
                    (def.loop_xfade_s.max(0.0) * zone.buffer.sample_rate()) as usize;
                Arc::new(SampleRegion {
                    lokey: def.lokey.min(def.hikey),
                    hikey: def.hikey.max(def.lokey),
                    lovel: 0,
                    hivel: 127,
                    pitch_keycenter: def.root,
                    tune_cents: def.tune_cents
                        + p.tune_cents
                        + p.transpose_semitones.round() * 100.0,
                    volume_db: def.gain_db,
                    amp_veltrack: p.velocity_amount.clamp(0.0, 1.0),
                    offset_frames: 0,
                    trigger: if def.one_shot {
                        TriggerKind::Release
                    } else {
                        TriggerKind::Attack
                    },
                    ampeg_attack: p.attack_s.max(MIN_ATTACK_S),
                    ampeg_decay: p.decay_s.max(0.0),
                    ampeg_sustain: p.sustain.clamp(0.0, 1.0),
                    ampeg_release: BASE_AMPEG_RELEASE_S,
                    sample: zone.buffer.clone(),
                    loop_mode: loop_mode_from_u8(def.loop_mode),
                    loop_start_frames: loop_start,
                    loop_end_frames: loop_end,
                    loop_xfade_frames,
                    pan: def.pan,
                })
            })
            .collect();
        self.regions_dirty = false;
    }

    // -- playback ----------------------------------------------------------

    pub fn note_on(&mut self, note: u8, velocity01: f32) {
        if self.regions_dirty {
            self.rebuild_regions();
        }
        let note = note & 0x7f;
        let velocity01 = velocity01.clamp(0.0, 1.0);
        let release_scale = self.params.release_s.max(0.01);
        for i in 0..self.regions.len() {
            if self.regions[i].matches(note, 127.min((velocity01 * 127.0) as u8)) {
                let region = self.regions[i].clone();
                self.engine
                    .trigger(region, note, velocity01, 1.0, release_scale);
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        self.engine.note_off(note & 0x7f);
    }

    /// Renders one block, applying stereo width and master gain. `left`
    /// and `right` must be the same length.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.engine.process(left, right);
        let width = self.params.stereo_width.clamp(0.0, 1.0);
        let master = db_to_linear(self.params.master_gain_db);
        for i in 0..left.len() {
            let l = flush_denormal(left[i]);
            let r = flush_denormal(right[i]);
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5 * width;
            left[i] = (mid + side) * master;
            right[i] = (mid - side) * master;
        }
    }
}

fn loop_mode_from_u8(mode: u8) -> LoopMode {
    match mode {
        1 => LoopMode::Infinite,
        2 => LoopMode::Sustain,
        3 => LoopMode::PingPong,
        4 => LoopMode::Reverse,
        _ => LoopMode::Off,
    }
}

/// A single full-range chromatic zone over the whole source — what the UI
/// calls Classic mode, and what the embedded startup bank uses.
pub fn classic_zone(root: u8, frames: u32) -> ZoneDef {
    ZoneDef {
        lokey: 0,
        hikey: 127,
        root,
        one_shot: false,
        loop_mode: 0,
        start_frame: 0,
        end_frame: frames,
        loop_start: 0,
        loop_end: frames,
        gain_db: 0.0,
        tune_cents: 0.0,
        pan: 0.0,
        loop_xfade_s: 0.01,
    }
}

// ---------------------------------------------------------------------------
// Embedded dev-bank parsing (same blob format as z-audio-synth's
// `build_bank_bytes`: magic ZSMPLBNK · u32 version · f32 rate · u8 channels
// · u8 root · u32 frames · i16le PCM). Parsed here directly so this crate
// can own the PCM as a mutable Vec (SampleBuffer doesn't expose its data).
// ---------------------------------------------------------------------------

pub fn parse_dev_bank(bytes: &[u8]) -> Option<(SourceSample, u8)> {
    let magic = bytes.get(..8)?;
    if magic != b"ZSMPLBNK" {
        return None;
    }
    let version = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    if version != 1 {
        return None;
    }
    let sample_rate = f32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?);
    let channels = *bytes.get(16)?;
    let root = *bytes.get(17)?;
    let frames = u32::from_le_bytes(bytes.get(18..22)?.try_into().ok()?) as usize;
    let total = frames.checked_mul(channels.max(1) as usize)?;
    let pcm_bytes = bytes.get(22..22 + total * 2)?;
    let mut data = Vec::with_capacity(total);
    for i in 0..total {
        let v = i16::from_le_bytes([pcm_bytes[i * 2], pcm_bytes[i * 2 + 1]]);
        data.push(v as f32 / i16::MAX as f32);
    }
    Some((
        SourceSample {
            sample_rate,
            channels: channels.max(1),
            data,
        },
        root.min(127),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_source(frames: usize) -> SourceSample {
        SourceSample {
            sample_rate: 48_000.0,
            channels: 1,
            data: (0..frames).map(|i| i as f32 / frames as f32).collect(),
        }
    }

    fn sine_source(frames: usize) -> SourceSample {
        SourceSample {
            sample_rate: 48_000.0,
            channels: 1,
            data: (0..frames)
                .map(|i| (core::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
                .collect(),
        }
    }

    fn slice_zone(key: u8, start: u32, end: u32) -> ZoneDef {
        ZoneDef {
            lokey: key,
            hikey: key,
            root: key,
            one_shot: true,
            loop_mode: 0,
            start_frame: start,
            end_frame: end,
            loop_start: 0,
            loop_end: 0,
            gain_db: 0.0,
            tune_cents: 0.0,
            pan: 0.0,
            loop_xfade_s: 0.0,
        }
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn silent_with_no_sample() {
        let mut s = ZoneSampler::new(48_000.0);
        s.note_on(60, 1.0);
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        s.render(&mut l, &mut r);
        assert!(l.iter().chain(r.iter()).all(|v| *v == 0.0));
    }

    #[test]
    fn classic_zone_plays_across_the_keyboard() {
        let mut s = ZoneSampler::new(48_000.0);
        let src = sine_source(48_000);
        let frames = src.frames() as u32;
        s.set_source(src, vec![classic_zone(60, frames)]);
        for note in [36, 60, 84] {
            s.note_on(note, 1.0);
        }
        assert_eq!(s.active_voice_count(), 3);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        s.render(&mut l, &mut r);
        assert!(l.iter().any(|v| v.abs() > 0.0));
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
    }

    #[test]
    fn slice_zones_only_respond_to_their_key() {
        let mut s = ZoneSampler::new(48_000.0);
        s.set_source(
            sine_source(8_000),
            vec![slice_zone(36, 0, 4_000), slice_zone(37, 4_000, 8_000)],
        );
        s.note_on(36, 1.0);
        assert_eq!(s.active_voice_count(), 1);
        s.note_on(38, 1.0); // unmapped key
        assert_eq!(s.active_voice_count(), 1);
        s.note_on(37, 1.0);
        assert_eq!(s.active_voice_count(), 2);
    }

    #[test]
    fn slice_zone_stops_at_its_end_frame() {
        let mut s = ZoneSampler::new(48_000.0);
        // 4000-frame slice out of a long ramp: playback must stop after
        // ~4000 frames even though the source is much longer.
        s.set_source(ramp_source(48_000), vec![slice_zone(36, 0, 4_000)]);
        s.note_on(36, 1.0);
        let mut l = [0.0f32; 8_192];
        let mut r = [0.0f32; 8_192];
        s.render(&mut l, &mut r);
        assert_eq!(
            s.active_voice_count(),
            0,
            "slice should end at its cut point"
        );
        assert!(rms(&l[..2_000]) > 0.0);
        assert!(l[6_000..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn one_shot_zone_ignores_note_off() {
        let mut s = ZoneSampler::new(48_000.0);
        s.set_source(sine_source(48_000), vec![slice_zone(36, 0, 48_000)]);
        s.note_on(36, 1.0);
        s.note_off(36);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        s.render(&mut l, &mut r);
        assert_eq!(
            s.active_voice_count(),
            1,
            "one-shot slices keep playing after note-off"
        );
    }

    #[test]
    fn sustained_zone_releases_on_note_off() {
        let mut s = ZoneSampler::new(48_000.0);
        let src = sine_source(480_000);
        let frames = src.frames() as u32;
        s.set_source(src, vec![classic_zone(60, frames)]);
        let mut p = *s.params();
        p.release_s = 0.02;
        s.set_params(p);
        s.note_on(60, 1.0);
        s.note_off(60);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        for _ in 0..20 {
            s.render(&mut l, &mut r);
        }
        assert_eq!(s.active_voice_count(), 0);
    }

    #[test]
    fn master_gain_scales_output() {
        let make = |gain_db: f32| {
            let mut s = ZoneSampler::new(48_000.0);
            let src = sine_source(48_000);
            let frames = src.frames() as u32;
            s.set_source(src, vec![classic_zone(69, frames)]);
            let mut p = *s.params();
            p.master_gain_db = gain_db;
            s.set_params(p);
            s.note_on(69, 1.0);
            let mut l = [0.0f32; 2_048];
            let mut r = [0.0f32; 2_048];
            s.render(&mut l, &mut r);
            rms(&l)
        };
        assert!(make(0.0) > make(-24.0) * 4.0);
    }

    #[test]
    fn chunked_upload_assembles_and_commits() {
        let mut s = ZoneSampler::new(48_000.0);
        s.begin_upload(44_100.0, 1, 1_000);
        let pcm: Vec<f32> = (0..1_000)
            .map(|i| (core::f32::consts::TAU * 100.0 * i as f32 / 44_100.0).sin())
            .collect();
        for (chunk_index, chunk) in pcm.chunks(256).enumerate() {
            let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
            s.upload_chunk((chunk_index * 256) as u32, &bytes);
        }
        s.commit_zones(vec![classic_zone(60, 1_000)]);
        assert!(s.has_sample());
        let (frames, rate, channels) = s.source_info();
        assert_eq!(frames, 1_000);
        assert_eq!(rate, 44_100.0);
        assert_eq!(channels, 1);
        s.note_on(60, 1.0);
        let mut l = [0.0f32; 256];
        let mut r = [0.0f32; 256];
        s.render(&mut l, &mut r);
        assert!(l.iter().any(|v| v.abs() > 0.0));
    }

    #[test]
    fn out_of_bounds_chunk_aborts_the_upload() {
        let mut s = ZoneSampler::new(48_000.0);
        s.begin_upload(44_100.0, 1, 100);
        let bytes: Vec<u8> = (0..64u32).flat_map(|_| 0.5f32.to_le_bytes()).collect();
        s.upload_chunk(90, &bytes); // 90 + 16 > 100
        s.commit_zones(vec![classic_zone(60, 100)]);
        assert!(
            !s.has_sample(),
            "poisoned upload must not become the source"
        );
    }

    #[test]
    fn recommit_recuts_zones_without_reupload() {
        let mut s = ZoneSampler::new(48_000.0);
        s.set_source(sine_source(8_000), vec![slice_zone(36, 0, 8_000)]);
        assert_eq!(s.zone_count(), 1);
        s.commit_zones(vec![slice_zone(36, 0, 4_000), slice_zone(37, 4_000, 8_000)]);
        assert_eq!(s.zone_count(), 2);
        s.note_on(37, 1.0);
        assert_eq!(s.active_voice_count(), 1);
    }

    #[test]
    fn degenerate_zones_are_skipped() {
        let mut s = ZoneSampler::new(48_000.0);
        s.set_source(
            sine_source(1_000),
            vec![
                slice_zone(36, 500, 500),   // empty
                slice_zone(37, 999, 5_000), // end past source, 1 frame after clamp
                slice_zone(38, 0, 1_000),   // fine
            ],
        );
        s.note_on(36, 1.0);
        s.note_on(37, 1.0);
        assert_eq!(s.active_voice_count(), 0);
        s.note_on(38, 1.0);
        assert_eq!(s.active_voice_count(), 1);
    }

    #[test]
    fn clear_drops_sample_and_zones() {
        let mut s = ZoneSampler::new(48_000.0);
        s.set_source(sine_source(1_000), vec![classic_zone(60, 1_000)]);
        s.clear();
        assert!(!s.has_sample());
        assert_eq!(s.zone_count(), 0);
        s.note_on(60, 1.0);
        assert_eq!(s.active_voice_count(), 0);
    }

    #[test]
    fn param_changes_apply_to_new_notes() {
        let mut s = ZoneSampler::new(48_000.0);
        let src = sine_source(480_000);
        let frames = src.frames() as u32;
        s.set_source(src, vec![classic_zone(60, frames)]);
        // Zero velocity with full velocity tracking: silent.
        s.note_on(60, 0.0);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        s.render(&mut l, &mut r);
        let silent = rms(&l);
        // Velocity amount 0: velocity no longer matters.
        let mut p = *s.params();
        p.velocity_amount = 0.0;
        s.set_params(p);
        s.note_on(60, 0.0);
        let mut l2 = [0.0f32; 2_048];
        let mut r2 = [0.0f32; 2_048];
        s.render(&mut l2, &mut r2);
        assert!(rms(&l2) > silent);
    }

    #[test]
    fn dev_bank_parses() {
        let bytes = include_bytes!("../../../assets/sampler/piano-dev.bank");
        let (source, root) = parse_dev_bank(bytes).expect("embedded bank parses");
        assert!(source.frames() > 1_000);
        assert!(root <= 127);
        assert!(source.sample_rate > 8_000.0);
    }

    #[test]
    fn dev_bank_note_renders_audible_output_immediately() {
        // End-to-end: default state (dev bank + classic zone), one note on,
        // the first 100 ms of output must carry real signal.
        let bytes = include_bytes!("../../../assets/sampler/piano-dev.bank");
        let (source, root) = parse_dev_bank(bytes).expect("embedded bank parses");
        let frames = source.frames() as u32;
        let mut s = ZoneSampler::new(48_000.0);
        s.set_source(source, vec![classic_zone(root, frames)]);
        s.note_on(60, 0.9);
        let mut l = [0.0f32; 4_800];
        let mut r = [0.0f32; 4_800];
        s.render(&mut l, &mut r);
        assert!(rms(&l) > 0.01, "dev bank note is inaudible (rms {})", rms(&l));
    }

    #[test]
    fn dev_bank_is_audible_from_the_start() {
        // Regression: the embedded bank used to lead with 1.5 s of digital
        // silence and peak at -42 dBFS, making the plugin appear silent.
        let bytes = include_bytes!("../../../assets/sampler/piano-dev.bank");
        let (source, _) = parse_dev_bank(bytes).expect("embedded bank parses");
        let peak = source.data.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "dev bank is too quiet (peak {peak})");
        let early = ((source.sample_rate * 0.25) as usize * source.channels.max(1) as usize)
            .min(source.data.len());
        let early_peak = source.data[..early]
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            early_peak > 0.05,
            "dev bank leads with silence (first 250 ms peak {early_peak})"
        );
    }

    #[test]
    fn looped_classic_zone_sustains_past_sample_end() {
        let mut s = ZoneSampler::new(48_000.0);
        let mut zone = classic_zone(60, 2_000);
        zone.loop_mode = 1; // infinite
        zone.loop_start = 0;
        zone.loop_end = 2_000;
        zone.loop_xfade_s = 0.001;
        s.set_source(sine_source(2_000), vec![zone]);
        s.note_on(60, 1.0);
        let mut l = [0.0f32; 20_000];
        let mut r = [0.0f32; 20_000];
        s.render(&mut l, &mut r);
        assert_eq!(s.active_voice_count(), 1);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
    }
}
