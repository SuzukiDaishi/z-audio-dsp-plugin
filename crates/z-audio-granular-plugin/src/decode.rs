//! WAV/FLAC decoding for the "Load Sample..." file picker.
//!
//! Only ever called from the editor (UI) thread or `Plugin::initialize`/
//! `reset` (both non-realtime setup callbacks); never from `process`.

use std::io::Cursor;
use std::path::Path;

use claxon::FlacReader;

/// Decodes `path` (`.wav`/`.flac`) to `(sample_rate, channels, interleaved f32 pcm)`.
pub fn decode_audio_file(path: &Path) -> Result<(f32, u8, Vec<f32>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read file: {e}"))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("flac") => decode_flac(&bytes),
        _ => decode_wav(&bytes),
    }
}

fn decode_flac(bytes: &[u8]) -> Result<(f32, u8, Vec<f32>), String> {
    let mut reader =
        FlacReader::new(Cursor::new(bytes)).map_err(|e| format!("not a valid FLAC stream: {e}"))?;
    let info = reader.streaminfo();
    let sample_rate = info.sample_rate as f32;
    let channels = info.channels as u8;
    let scale = 1.0_f32 / (1_i64 << (info.bits_per_sample - 1)) as f32;

    let mut pcm =
        Vec::with_capacity((info.samples.unwrap_or(0) as usize) * channels.max(1) as usize);
    for sample in reader.samples() {
        let sample = sample.map_err(|e| format!("FLAC decode error: {e}"))?;
        pcm.push(sample as f32 * scale);
    }
    Ok((sample_rate, channels, pcm))
}

fn decode_wav(bytes: &[u8]) -> Result<(f32, u8, Vec<f32>), String> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|e| format!("not a valid WAV stream: {e}"))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate as f32;
    let channels = spec.channels as u8;

    let pcm: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("WAV float decode error: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = 1.0_f32 / (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("WAV int decode error: {e}"))?
        }
    };
    Ok((sample_rate, channels, pcm))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn decode_audio_file_rejects_missing_path() {
        let result = decode_audio_file(Path::new("does/not/exist.wav"));
        assert!(result.is_err());
    }
}
