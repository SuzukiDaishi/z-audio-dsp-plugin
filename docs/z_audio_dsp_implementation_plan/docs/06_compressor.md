# 06. コンプレッサー実装計画

## ゴール

`z-audio-dsp` に compressor core を追加し、`z-audio-dsp-plugin` に native / WebCLAP plugin を作る。

MVP は feed-forward compressor。
Phase 2 で knee, RMS, sidechain EQ, auto makeup, parallel compression を追加する。

## Signal flow

```text
Input L/R
  -> input gain
  -> detector
  -> attack/release ballistics
  -> gain computer
  -> makeup gain
  -> dry/wet mix
Output L/R
```

## public API

```rust
pub struct Compressor {
    sample_rate: f32,
    params: CompressorParams,
    detector: LevelDetector,
    envelope: BallisticsFilter,
    gain_smoother: BallisticsFilter,
    sidechain_filter: Option<SidechainFilter>,
}

pub struct CompressorParams {
    pub input_gain_db: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_gain_db: f32,
    pub mix: f32,
    pub detector_mode: DetectorMode,
    pub stereo_link: f32,
}

pub enum DetectorMode {
    Peak,
    Rms,
}
```

`Compressor` は `Effect` trait を実装する。

## Detector

### peak detector

```text
level = max(abs(l), abs(r))
```

### RMS detector

```text
rms2[n] = a * rms2[n-1] + (1-a) * x^2
level = sqrt(rms2)
```

RMS window は 5〜50 ms。
MVP は peak のみでよい。

## Ballistics

attack/release coefficient:

```text
attack_coeff = exp(-1 / (attack_sec * sample_rate))
release_coeff = exp(-1 / (release_sec * sample_rate))
```

level が上がるとき attack、下がるとき release。

```rust
if input > state {
    coeff = attack_coeff;
} else {
    coeff = release_coeff;
}
state = coeff * state + (1.0 - coeff) * input;
```

## Gain computer

### hard knee

```text
if level_db <= threshold_db:
    gain_db = 0
else:
    compressed_db = threshold_db + (level_db - threshold_db) / ratio
    gain_db = compressed_db - level_db
```

### soft knee

Soft knee は Phase 2。

```text
x = level_db - threshold_db
if x < -knee/2:
    gain = 0
else if x > knee/2:
    gain = threshold + x/ratio - level
else:
    gain = ((1/ratio - 1) * (x + knee/2)^2) / (2*knee)
```

## Auto makeup

MVP では manual makeup のみ。
Phase 2 で auto makeup。

簡易 auto makeup:

```text
auto_makeup_db = -gain_reduction_at(threshold + 6dB) * 0.5
```

## Parallel compression

`mix` parameter で dry/wet。

```text
wet = input * gain * makeup
y = dry * (1 - mix) + wet * mix
```

注意:

- lookahead がない compressor なら latency 差がない
- future lookahead compressor では dry も delay する必要がある

## Sidechain filter

Phase 2。

- highpass sidechain 20..500 Hz
- tilt EQ optional

低域で compressor が過剰反応するのを防ぐ。

## Parameters

- Input Gain: -24..24 dB
- Threshold: -60..0 dB
- Ratio: 1..20
- Knee: 0..24 dB
- Attack: 0.1..200 ms
- Release: 5..2000 ms
- Makeup Gain: -24..24 dB
- Mix: 0..100 %
- Detector: Peak/RMS
- Stereo Link: 0..100 %
- Sidechain HPF: off / 20..500 Hz
- Bypass

## UI

```text
[Main]
  Threshold
  Ratio
  Knee

[Timing]
  Attack
  Release

[Gain]
  Input
  Makeup
  Mix

[Detector]
  Peak/RMS
  Stereo Link
  Sidechain HPF

[Meter]
  Input
  Gain Reduction
  Output
```

## Metering

audio thread から UI thread へ atomic で値を出す。

- input peak
- envelope dB
- gain reduction dB
- output peak

更新は block ごとでよい。

## plugin wrapper

`z-audio-compressor-plugin`

```text
Plugin type: AudioEffect
Category: Dynamics / Compressor
Input layouts: mono, stereo
Output layouts: mono, stereo
```

## tests

### DSP tests

- threshold より小さい入力は変化しない
- threshold より大きい入力は ratio に従う
- ratio 1:1 は変化しない
- attack が長いと transient が残る
- release が長いと gain reduction が戻りにくい
- mix 0 は dry
- mix 1 は wet
- no NaN/Inf

### numerical tests

- db_to_linear / linear_to_db round trip
- threshold edge
- silence input
- extremely low input

## 実装順

1. `LevelDetector`
2. `BallisticsFilter`
3. `CompressorGainComputer`
4. `Compressor` core
5. hard knee tests
6. native plugin
7. meter atomics
8. RMS detector
9. soft knee
10. sidechain HPF
11. WebCLAP
