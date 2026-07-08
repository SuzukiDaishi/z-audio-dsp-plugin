//! `cargo xtask prepare-sampler-bank`: offline WAV/FLAC -> generic sampler
//! bank conversion. See `docs/汎用サンプラー実装計画.md` for the full plan.
//!
//! This only runs at build/asset-prep time; none of it touches the audio
//! thread. It decodes the source file to f32 PCM and writes a self-
//! contained bank file via `z_audio_synth::build_sampler_bank_bytes`.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use claxon::FlacReader;
use z_audio_synth::build_sampler_bank_bytes;

/// Default length of the embeddable "dev bank" (downmixed to mono and
/// truncated), used by the WebCLAP build so the wasm module stays small.
const DEFAULT_DEV_MAX_SECONDS: f32 = 4.0;

/// Frames whose peak stays below this are considered leading silence
/// (~-60 dBFS). The VCSL piano source leads with ~1.5 s of digital silence,
/// which made the out-of-the-box sampler/granular sound appear broken.
const SILENCE_THRESHOLD: f32 = 1.0e-3;

/// Pre-roll kept before the first audible frame so attacks stay intact.
const PRE_ROLL_SECONDS: f32 = 0.01;

/// Peak-normalization target (-1 dBFS). The raw source peaks around
/// -42 dBFS, which is inaudible at default plugin gains.
const PEAK_TARGET: f32 = 0.891;

pub fn run(args: &[String]) -> Result<()> {
    nih_plug_xtask::chdir_workspace_root().context("could not chdir to workspace root")?;

    let source = arg_value(args, "--source")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/samples/piano.wav"));
    let out = arg_value(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/sampler/piano.bank"));
    let dev_out = arg_value(args, "--dev-out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/sampler/piano-dev.bank"));
    let dev_max_seconds = arg_value(args, "--dev-max-seconds")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(DEFAULT_DEV_MAX_SECONDS);
    let root_note = arg_value(args, "--root-note")
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(60);

    if !source.is_file() {
        bail!("source audio file not found: '{}'", source.display());
    }

    eprintln!(
        "Preparing generic sampler bank: source='{}' root_note={root_note}",
        source.display()
    );

    let bytes =
        fs::read(&source).with_context(|| format!("could not read '{}'", source.display()))?;
    let (sample_rate, channels, pcm) = decode_audio(&source, &bytes)?;
    eprintln!(
        "Decoded {} Hz, {} channel(s), {} frames",
        sample_rate,
        channels,
        pcm.len() / channels.max(1) as usize
    );

    let pre_roll = (PRE_ROLL_SECONDS * sample_rate) as usize;
    let pcm = trim_leading_silence(&pcm, channels, SILENCE_THRESHOLD, pre_roll);

    let bank_bytes = build_sampler_bank_bytes(
        sample_rate,
        channels,
        &normalize_peak(pcm.clone(), PEAK_TARGET),
        root_note,
    );
    write_bank(&out, &bank_bytes)?;
    eprintln!(
        "Wrote sampler bank: '{}' ({} bytes)",
        out.display(),
        bank_bytes.len()
    );

    let max_frames = (dev_max_seconds.max(0.01) * sample_rate) as usize;
    let dev_pcm = normalize_peak(downmix_truncate(&pcm, channels, max_frames), PEAK_TARGET);
    let dev_bank_bytes = build_sampler_bank_bytes(sample_rate, 1, &dev_pcm, root_note);
    write_bank(&dev_out, &dev_bank_bytes)?;
    eprintln!(
        "Wrote dev bank: '{}' ({} bytes, {dev_max_seconds}s mono)",
        dev_out.display(),
        dev_bank_bytes.len()
    );

    Ok(())
}

fn write_bank(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create '{}'", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("could not write '{}'", path.display()))?;
    Ok(())
}

/// Drops leading frames whose per-frame peak stays below `threshold`,
/// keeping `pre_roll_frames` before the first audible frame. Returns the
/// input unchanged when it never crosses the threshold.
fn trim_leading_silence(
    pcm: &[f32],
    channels: u8,
    threshold: f32,
    pre_roll_frames: usize,
) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let frames = pcm.len() / channels;
    let first = (0..frames).find(|&frame| {
        pcm[frame * channels..(frame + 1) * channels]
            .iter()
            .any(|s| s.abs() >= threshold)
    });
    let Some(first) = first else {
        return pcm.to_vec();
    };
    let start = first.saturating_sub(pre_roll_frames);
    pcm[start * channels..].to_vec()
}

/// Scales `pcm` so its peak lands on `target` (skipped for near-silent
/// input, where the gain would explode).
fn normalize_peak(mut pcm: Vec<f32>, target: f32) -> Vec<f32> {
    let peak = pcm.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    if peak > 1.0e-6 {
        let gain = target / peak;
        for s in &mut pcm {
            *s *= gain;
        }
    }
    pcm
}

/// Downmixes `pcm` (interleaved, `channels` wide) to mono and truncates to
/// at most `max_frames` frames.
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

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn decode_audio(path: &std::path::Path, bytes: &[u8]) -> Result<(f32, u8, Vec<f32>)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("flac") => decode_flac(bytes).context("could not decode FLAC"),
        _ => decode_wav(bytes).context("could not decode WAV/AIFF"),
    }
}

fn decode_flac(bytes: &[u8]) -> Result<(f32, u8, Vec<f32>)> {
    let mut reader = FlacReader::new(Cursor::new(bytes)).context("not a valid FLAC stream")?;
    let info = reader.streaminfo();
    let sample_rate = info.sample_rate as f32;
    let channels = info.channels as u8;
    let scale = 1.0_f32 / (1_i64 << (info.bits_per_sample - 1)) as f32;

    let mut pcm =
        Vec::with_capacity((info.samples.unwrap_or(0) as usize) * channels.max(1) as usize);
    for sample in reader.samples() {
        let sample = sample.context("FLAC decode error")?;
        pcm.push(sample as f32 * scale);
    }
    Ok((sample_rate, channels, pcm))
}

fn decode_wav(bytes: &[u8]) -> Result<(f32, u8, Vec<f32>)> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).context("not a valid WAV stream")?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate as f32;
    let channels = spec.channels as u8;

    let pcm: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .context("WAV float decode error")?,
        hound::SampleFormat::Int => {
            let scale = 1.0_f32 / (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<_>, _>>()
                .context("WAV int decode error")?
        }
    };
    Ok((sample_rate, channels, pcm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_value_finds_flag_following_value() {
        let args = vec!["--root-note".to_string(), "67".to_string()];
        assert_eq!(arg_value(&args, "--root-note"), Some("67".to_string()));
        assert_eq!(arg_value(&args, "--missing"), None);
    }

    #[test]
    fn decode_wav_round_trips_a_simple_mono_16bit_file() {
        let mut buf = Vec::new();
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 8_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(Cursor::new(&mut buf), spec).unwrap();
            for s in [0_i16, i16::MAX, i16::MIN, -1000, 1000] {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        let (sample_rate, channels, pcm) = decode_wav(&buf).expect("should decode");
        assert_eq!(sample_rate, 8_000.0);
        assert_eq!(channels, 1);
        assert_eq!(pcm.len(), 5);
        assert!((pcm[1] - 1.0).abs() < 1.0e-3);
        assert!((pcm[2] - (-1.0)).abs() < 1.0e-3);
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
    fn trim_leading_silence_keeps_pre_roll() {
        let mut pcm = vec![0.0; 100];
        pcm[50] = 0.5;
        let out = trim_leading_silence(&pcm, 1, 1.0e-3, 10);
        assert_eq!(out.len(), 60); // starts at frame 40
        assert_eq!(out[10], 0.5);
    }

    #[test]
    fn trim_leading_silence_passes_silence_through() {
        let pcm = vec![0.0; 16];
        assert_eq!(trim_leading_silence(&pcm, 1, 1.0e-3, 4), pcm);
    }

    #[test]
    fn trim_leading_silence_checks_all_channels() {
        // Signal only on the right channel must still stop the trim.
        let pcm = vec![0.0, 0.0, 0.0, 0.4, 0.2, 0.2];
        let out = trim_leading_silence(&pcm, 2, 1.0e-3, 0);
        assert_eq!(out, vec![0.0, 0.4, 0.2, 0.2]);
    }

    #[test]
    fn normalize_peak_scales_to_target() {
        let out = normalize_peak(vec![0.004, -0.008], 0.891);
        assert!((out[1] + 0.891).abs() < 1.0e-6);
        assert!((out[0] - 0.4455).abs() < 1.0e-6);
    }

    #[test]
    fn normalize_peak_leaves_silence_alone() {
        assert_eq!(normalize_peak(vec![0.0; 8], 0.891), vec![0.0; 8]);
    }
}
