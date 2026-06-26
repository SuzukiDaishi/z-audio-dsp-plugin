//! `cargo xtask prepare-vcsl-piano`: offline SFZ + FLAC -> VCSL sampler bank
//! conversion. See `docs/VCSLサンプラーピアノ実装計画.md` for the full plan.
//!
//! This only runs at build/asset-prep time; none of it touches the audio
//! thread. It decodes every referenced FLAC sample to f32 PCM and writes a
//! self-contained bank file via `z_audio_synth::build_bank_bytes`.

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use claxon::FlacReader;
use z_audio_dsp::TriggerKind;
use z_audio_synth::{VcslRegionSource, build_bank_bytes};
use zip::ZipArchive;

const SUPPORTED_OPCODES: &[&str] = &[
    "sample",
    "lokey",
    "hikey",
    "pitch_keycenter",
    "lovel",
    "hivel",
    "volume",
    "global_volume",
    "tune",
    "offset",
    "amp_veltrack",
    "ampeg_attack",
    "ampeg_decay",
    "ampeg_sustain",
    "ampeg_release",
    "trigger",
    "rt_decay",
];

/// Notes (MIDI numbers) kept in the small "dev bank" used to embed a piano
/// in WebCLAP without shipping the full instrument. Only the loudest
/// (highest hivel) velocity layer per note is kept, and samples are
/// downmixed to mono and truncated, to keep the embedded bank small.
const DEV_NOTES: &[u8] = &[36, 48, 60, 67, 72, 84];
const DEV_MAX_SECONDS: f32 = 4.0;

pub fn run(args: &[String]) -> Result<()> {
    nih_plug_xtask::chdir_workspace_root().context("could not chdir to workspace root")?;

    let source = arg_value(args, "--source")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/VCSL_Keys.zip"));
    let instrument = arg_value(args, "--instrument")
        .unwrap_or_else(|| "Grand Piano, K".to_string());
    let out = arg_value(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/vcsl-piano/grand-piano-k.bank"));
    let dev_out = arg_value(args, "--dev-out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/vcsl-piano/grand-piano-k-dev.bank"));

    if !source.is_file() {
        bail!("source zip not found: '{}'", source.display());
    }

    eprintln!(
        "Preparing VCSL piano bank: instrument='{instrument}' source='{}'",
        source.display()
    );

    let file = fs::File::open(&source)
        .with_context(|| format!("could not open '{}'", source.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("could not read zip '{}'", source.display()))?;

    let sfz_name = format!("{instrument}.sfz");
    let sfz_text = read_zip_text(&mut archive, &sfz_name)
        .with_context(|| format!("could not read '{sfz_name}' from zip"))?;

    let mut unsupported: HashMap<String, usize> = HashMap::new();
    let raw_regions = parse_sfz(&sfz_text, &mut unsupported);
    eprintln!("Parsed {} region(s) from '{sfz_name}'", raw_regions.len());
    for (opcode, count) in &unsupported {
        eprintln!("  note: opcode '{opcode}' seen {count} time(s) but is not applied by the MVP sampler");
    }

    let mut decoded_cache: HashMap<String, (f32, u8, std::sync::Arc<Vec<f32>>)> = HashMap::new();
    let mut sources = Vec::with_capacity(raw_regions.len());
    for region in &raw_regions {
        let sample_path = region
            .get("sample")
            .ok_or_else(|| anyhow::anyhow!("region missing required 'sample' opcode"))?;

        let (sample_rate, channels, pcm) = if let Some(cached) = decoded_cache.get(sample_path) {
            cached.clone()
        } else {
            let bytes = read_zip_bytes(&mut archive, sample_path)
                .with_context(|| format!("missing sample referenced by SFZ: '{sample_path}'"))?;
            let (sample_rate, channels, pcm) = decode_flac(&bytes)
                .with_context(|| format!("could not decode FLAC '{sample_path}'"))?;
            let pcm = std::sync::Arc::new(pcm);
            decoded_cache.insert(sample_path.clone(), (sample_rate, channels, pcm.clone()));
            (sample_rate, channels, pcm)
        };

        sources.push(VcslRegionSource {
            lokey: parse_u8(region, "lokey", 0),
            hikey: parse_u8(region, "hikey", 127),
            lovel: parse_u8(region, "lovel", 0),
            hivel: parse_u8(region, "hivel", 127),
            pitch_keycenter: parse_u8(region, "pitch_keycenter", 60),
            tune_cents: parse_f32(region, "tune", 0.0),
            volume_db: parse_f32(region, "volume", 0.0) + parse_f32(region, "global_volume", 0.0),
            amp_veltrack: parse_f32(region, "amp_veltrack", 100.0) / 100.0,
            offset_frames: parse_f32(region, "offset", 0.0) as u32,
            trigger: if region.get("trigger").map(String::as_str) == Some("release") {
                TriggerKind::Release
            } else {
                TriggerKind::Attack
            },
            ampeg_attack: parse_f32(region, "ampeg_attack", 0.004),
            ampeg_decay: parse_f32(region, "ampeg_decay", 0.0),
            ampeg_sustain: parse_f32(region, "ampeg_sustain", 1.0).clamp(0.0, 1.0),
            ampeg_release: parse_f32(region, "ampeg_release", 0.4),
            sample_rate,
            channels,
            pcm: (*pcm).clone(),
        });
    }

    write_bank(&out, &sources)?;
    eprintln!(
        "Wrote full bank: '{}' ({} regions)",
        out.display(),
        sources.len()
    );

    let dev_sources = build_dev_bank(&sources);
    write_bank(&dev_out, &dev_sources)?;
    eprintln!(
        "Wrote dev bank: '{}' ({} regions, notes={:?})",
        dev_out.display(),
        dev_sources.len(),
        DEV_NOTES
    );

    Ok(())
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn parse_u8(region: &HashMap<String, String>, key: &str, default: u8) -> u8 {
    region
        .get(key)
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| v.round().clamp(0.0, 127.0) as u8)
        .unwrap_or(default)
}

fn parse_f32(region: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    region
        .get(key)
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(default)
}

/// Parses the SFZ opcode subset described in
/// `docs/VCSLサンプラーピアノ実装計画.md`. Returns one merged opcode map per
/// `<region>` (global -> group -> region precedence already applied).
fn parse_sfz(text: &str, unsupported: &mut HashMap<String, usize>) -> Vec<HashMap<String, String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Section {
        None,
        Global,
        Group,
        Region,
    }

    let mut section = Section::None;
    let mut global_map: HashMap<String, String> = HashMap::new();
    let mut group_map: HashMap<String, String> = HashMap::new();
    let mut region_map: HashMap<String, String> = HashMap::new();
    let mut have_region = false;
    let mut regions = Vec::new();

    let merge = |global: &HashMap<String, String>,
                 group: &HashMap<String, String>,
                 region: &HashMap<String, String>| {
        let mut merged = global.clone();
        merged.extend(group.clone());
        merged.extend(region.clone());
        merged
    };

    for raw_line in text.lines() {
        let line = match raw_line.split_once("//") {
            Some((before, _)) => before.trim(),
            None => raw_line.trim(),
        };
        if line.is_empty() {
            continue;
        }
        match line {
            "<global>" => {
                section = Section::Global;
                global_map.clear();
                continue;
            }
            "<group>" => {
                if have_region {
                    regions.push(merge(&global_map, &group_map, &region_map));
                    have_region = false;
                    region_map.clear();
                }
                section = Section::Group;
                group_map.clear();
                continue;
            }
            "<region>" => {
                if have_region {
                    regions.push(merge(&global_map, &group_map, &region_map));
                }
                region_map.clear();
                have_region = true;
                section = Section::Region;
                continue;
            }
            _ => {}
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if !SUPPORTED_OPCODES.contains(&key.as_str()) {
                *unsupported.entry(key.clone()).or_insert(0) += 1;
            }
            match section {
                Section::Global => {
                    global_map.insert(key, value);
                }
                Section::Group => {
                    group_map.insert(key, value);
                }
                Section::Region => {
                    region_map.insert(key, value);
                }
                Section::None => {}
            }
        }
    }
    if have_region {
        regions.push(merge(&global_map, &group_map, &region_map));
    }
    regions
}

fn read_zip_text<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<String> {
    let mut file = archive.by_name(name)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn read_zip_bytes<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive.by_name(name)?;
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn decode_flac(bytes: &[u8]) -> Result<(f32, u8, Vec<f32>)> {
    let mut reader = FlacReader::new(Cursor::new(bytes)).context("not a valid FLAC stream")?;
    let info = reader.streaminfo();
    let sample_rate = info.sample_rate as f32;
    let channels = info.channels as u8;
    let scale = 1.0_f32 / (1_i64 << (info.bits_per_sample - 1)) as f32;

    let mut pcm = Vec::with_capacity((info.samples.unwrap_or(0) as usize) * channels.max(1) as usize);
    for sample in reader.samples() {
        let sample = sample.context("FLAC decode error")?;
        pcm.push(sample as f32 * scale);
    }
    Ok((sample_rate, channels, pcm))
}

fn write_bank(path: &Path, sources: &[VcslRegionSource]) -> Result<()> {
    let bytes = build_bank_bytes(sources);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create '{}'", parent.display()))?;
    }
    fs::write(path, &bytes).with_context(|| format!("could not write '{}'", path.display()))?;
    Ok(())
}

/// Builds a small embeddable bank: for each [`DEV_NOTES`] entry, keeps only
/// the highest-velocity attack and release region covering that note,
/// downmixes to mono, widens the key range to fill the gaps between dev
/// notes, and truncates playback to [`DEV_MAX_SECONDS`].
fn build_dev_bank(sources: &[VcslRegionSource]) -> Vec<VcslRegionSource> {
    let mut dev = Vec::new();
    for (i, &note) in DEV_NOTES.iter().enumerate() {
        let lokey = match i {
            0 => 0,
            _ => (DEV_NOTES[i - 1] + note) / 2 + 1,
        };
        let hikey = match DEV_NOTES.get(i + 1) {
            Some(&next) => (note + next) / 2,
            None => 127,
        };
        for trigger in [TriggerKind::Attack, TriggerKind::Release] {
            if let Some(region) = sources
                .iter()
                .filter(|r| r.trigger == trigger && note >= r.lokey && note <= r.hikey)
                .max_by_key(|r| r.hivel)
            {
                let max_frames = (DEV_MAX_SECONDS * region.sample_rate) as usize;
                let mono = downmix_truncate(&region.pcm, region.channels, max_frames);
                dev.push(VcslRegionSource {
                    lokey,
                    hikey,
                    lovel: 0,
                    hivel: 127,
                    pitch_keycenter: region.pitch_keycenter,
                    tune_cents: region.tune_cents,
                    volume_db: region.volume_db,
                    amp_veltrack: region.amp_veltrack,
                    offset_frames: region.offset_frames.min(mono.len() as u32),
                    trigger,
                    ampeg_attack: region.ampeg_attack,
                    ampeg_decay: region.ampeg_decay,
                    ampeg_sustain: region.ampeg_sustain,
                    ampeg_release: region.ampeg_release,
                    sample_rate: region.sample_rate,
                    channels: 1,
                    pcm: mono,
                });
            }
        }
    }
    dev
}

fn downmix_truncate(pcm: &[f32], channels: u8, max_frames: usize) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let frames = (pcm.len() / channels).min(max_frames.max(1));
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * channels;
        let sum: f32 = pcm[base..base + channels].iter().sum();
        mono.push(sum / channels as f32);
    }
    mono
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn region_source(
        lokey: u8,
        hikey: u8,
        hivel: u8,
        trigger: TriggerKind,
        channels: u8,
        pcm: Vec<f32>,
    ) -> VcslRegionSource {
        VcslRegionSource {
            lokey,
            hikey,
            lovel: 0,
            hivel,
            pitch_keycenter: lokey,
            tune_cents: 0.0,
            volume_db: 0.0,
            amp_veltrack: 1.0,
            offset_frames: 0,
            trigger,
            ampeg_attack: 0.004,
            ampeg_decay: 0.0,
            ampeg_sustain: 1.0,
            ampeg_release: 0.4,
            sample_rate: 100.0,
            channels,
            pcm,
        }
    }

    #[test]
    fn arg_value_finds_flag_following_value() {
        let args = vec!["--instrument".to_string(), "Grand Piano, K".to_string()];
        assert_eq!(
            arg_value(&args, "--instrument"),
            Some("Grand Piano, K".to_string())
        );
        assert_eq!(arg_value(&args, "--missing"), None);
    }

    #[test]
    fn arg_value_ignores_flag_with_no_trailing_value() {
        let args = vec!["--instrument".to_string()];
        assert_eq!(arg_value(&args, "--instrument"), None);
    }

    #[test]
    fn parse_u8_falls_back_to_default_when_missing_or_unparsable() {
        let region = region_map(&[("lokey", "garbage")]);
        assert_eq!(parse_u8(&region, "lokey", 9), 9);
        assert_eq!(parse_u8(&region, "hikey", 42), 42);
    }

    #[test]
    fn parse_u8_rounds_and_clamps_to_midi_range() {
        let region = region_map(&[("lokey", "60.6"), ("hikey", "999")]);
        assert_eq!(parse_u8(&region, "lokey", 0), 61);
        assert_eq!(parse_u8(&region, "hikey", 0), 127);
    }

    #[test]
    fn parse_f32_falls_back_to_default_when_missing_or_unparsable() {
        let region = region_map(&[("tune", "nope")]);
        assert_eq!(parse_f32(&region, "tune", 1.5), 1.5);
        assert_eq!(parse_f32(&region, "volume", 2.5), 2.5);
    }

    #[test]
    fn parse_f32_parses_negative_values() {
        let region = region_map(&[("tune", "-11.5")]);
        assert_eq!(parse_f32(&region, "tune", 0.0), -11.5);
    }

    #[test]
    fn parse_sfz_applies_global_and_group_defaults_to_a_region() {
        let text = "\
<global>
global_volume=2
amp_veltrack=88

<group>
trigger=attack
ampeg_attack=0.004

<region>
lokey=21
hikey=23
sample=a.flac
";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert_eq!(regions.len(), 1);
        let region = &regions[0];
        assert_eq!(region.get("global_volume").map(String::as_str), Some("2"));
        assert_eq!(region.get("amp_veltrack").map(String::as_str), Some("88"));
        assert_eq!(region.get("trigger").map(String::as_str), Some("attack"));
        assert_eq!(region.get("lokey").map(String::as_str), Some("21"));
        assert!(unsupported.is_empty());
    }

    #[test]
    fn parse_sfz_region_overrides_group_which_overrides_global() {
        let text = "\
<global>
volume=1

<group>
volume=2
trigger=attack

<region>
volume=3
sample=a.flac

<region>
sample=b.flac
";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert_eq!(regions.len(), 2);
        // First region overrides the group's volume.
        assert_eq!(regions[0].get("volume").map(String::as_str), Some("3"));
        // Second region falls through to the group's volume.
        assert_eq!(regions[1].get("volume").map(String::as_str), Some("2"));
        // Both inherit the group-level trigger.
        assert_eq!(regions[0].get("trigger").map(String::as_str), Some("attack"));
        assert_eq!(regions[1].get("trigger").map(String::as_str), Some("attack"));
    }

    #[test]
    fn parse_sfz_strips_comments_and_blank_lines() {
        let text = "\
// a leading comment

<region>
lokey=21 // inline comment
sample=a.flac
";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].get("lokey").map(String::as_str), Some("21"));
    }

    #[test]
    fn parse_sfz_records_unsupported_opcodes_without_dropping_the_region() {
        let text = "\
<region>
lokey=21
seq_length=2
seq_position=1
sample=a.flac
";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert_eq!(regions.len(), 1);
        assert_eq!(unsupported.get("seq_length"), Some(&1));
        assert_eq!(unsupported.get("seq_position"), Some(&1));
        assert_eq!(regions[0].get("lokey").map(String::as_str), Some("21"));
    }

    #[test]
    fn parse_sfz_resets_group_defaults_at_each_group_header() {
        let text = "\
<group>
trigger=attack

<region>
sample=a.flac

<group>
trigger=release

<region>
sample=b.flac
";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].get("trigger").map(String::as_str), Some("attack"));
        assert_eq!(regions[1].get("trigger").map(String::as_str), Some("release"));
    }

    #[test]
    fn parse_sfz_captures_trailing_region_with_no_following_header() {
        let text = "<region>\nlokey=10\nsample=a.flac\n";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].get("lokey").map(String::as_str), Some("10"));
    }

    #[test]
    fn parse_sfz_with_no_regions_returns_empty() {
        let text = "<global>\nglobal_volume=2\n";
        let mut unsupported = HashMap::new();
        let regions = parse_sfz(text, &mut unsupported);
        assert!(regions.is_empty());
    }

    #[test]
    fn downmix_truncate_passes_mono_through_unchanged() {
        let pcm = vec![0.1, 0.2, 0.3];
        let out = downmix_truncate(&pcm, 1, 10);
        assert_eq!(out, pcm);
    }

    #[test]
    fn downmix_truncate_averages_stereo_channels() {
        let pcm = vec![1.0, -1.0, 0.5, 0.5];
        let out = downmix_truncate(&pcm, 2, 10);
        assert_eq!(out, vec![0.0, 0.5]);
    }

    #[test]
    fn downmix_truncate_caps_to_max_frames() {
        let pcm = vec![1.0; 100];
        let out = downmix_truncate(&pcm, 1, 10);
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn downmix_truncate_does_not_pad_short_input() {
        let pcm = vec![1.0, 2.0];
        let out = downmix_truncate(&pcm, 1, 1000);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn build_dev_bank_keeps_only_the_highest_velocity_layer_per_note() {
        let sources = vec![
            region_source(60, 60, 65, TriggerKind::Attack, 1, vec![1.0; 100]),
            region_source(60, 60, 127, TriggerKind::Attack, 1, vec![2.0; 100]),
        ];
        let dev = build_dev_bank(&sources);
        let attack: Vec<_> = dev.iter().filter(|r| r.trigger == TriggerKind::Attack).collect();
        assert_eq!(attack.len(), 1);
        assert_eq!(attack[0].hivel, 127); // re-widened to full range in the dev bank
        assert!(attack[0].pcm.iter().all(|s| *s == 2.0));
    }

    #[test]
    fn build_dev_bank_downmixes_to_mono_and_truncates() {
        let long_stereo = vec![1.0; (DEV_MAX_SECONDS as usize + 1) * 100 * 2];
        let sources = vec![region_source(
            60,
            60,
            127,
            TriggerKind::Attack,
            2,
            long_stereo,
        )];
        let dev = build_dev_bank(&sources);
        assert_eq!(dev.len(), 1);
        assert_eq!(dev[0].channels, 1);
        let max_frames = (DEV_MAX_SECONDS * dev[0].sample_rate) as usize;
        assert!(dev[0].pcm.len() <= max_frames);
    }

    #[test]
    fn build_dev_bank_widens_key_ranges_to_cover_gaps_between_dev_notes() {
        let sources: Vec<_> = DEV_NOTES
            .iter()
            .map(|&note| region_source(note, note, 127, TriggerKind::Attack, 1, vec![1.0; 10]))
            .collect();
        let dev = build_dev_bank(&sources);
        let attack: Vec<_> = dev.iter().filter(|r| r.trigger == TriggerKind::Attack).collect();
        assert_eq!(attack.len(), DEV_NOTES.len());
        // Full MIDI range should be covered with no gaps and no overlaps.
        assert_eq!(attack[0].lokey, 0);
        assert_eq!(attack.last().unwrap().hikey, 127);
        for i in 1..attack.len() {
            assert_eq!(
                attack[i].lokey,
                attack[i - 1].hikey + 1,
                "gap/overlap between dev note {} and {}",
                i - 1,
                i
            );
        }
    }

    #[test]
    fn build_dev_bank_skips_notes_with_no_covering_source() {
        // No source covers any DEV_NOTES key, so the dev bank should end up
        // empty rather than panicking.
        let sources = vec![region_source(0, 5, 127, TriggerKind::Attack, 1, vec![1.0; 10])];
        let dev = build_dev_bank(&sources);
        assert!(dev.is_empty());
    }
}
