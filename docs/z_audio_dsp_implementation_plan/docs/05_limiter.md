# 05. リミッター実装計画

## ゴール

`z-audio-dsp` に stereo limiter を追加し、`z-audio-dsp-plugin` に native / WebCLAP plugin を追加する。

MVP は lookahead peak limiter。
Phase 2 で true peak limiter / oversampling に拡張する。

## Signal flow

```text
Input L/R
  -> optional input gain
  -> detector sidechain
  -> lookahead delay
  -> gain computer
  -> release smoothing
  -> gain apply
  -> ceiling
Output L/R
```

## public API

```rust
pub struct Limiter {
    sample_rate: f32,
    params: LimiterParams,
    lookahead_l: DelayLine,
    lookahead_r: DelayLine,
    gain_smoother: ReleaseSmoother,
    current_gain: f32,
}

pub struct LimiterParams {
    pub input_gain_db: f32,
    pub ceiling_db: f32,
    pub threshold_db: f32,
    pub release_ms: f32,
    pub lookahead_ms: f32,
    pub stereo_link: f32,
    pub true_peak: bool,
    pub output_gain_db: f32,
}
```

`Limiter` は `Effect` trait を実装する。

## level detection

通常 peak:

```text
peak = max(abs(left), abs(right))
```

stereo link:

```text
linked = max(abs(left), abs(right))
unlinked_l = abs(left)
unlinked_r = abs(right)
det_l = mix(unlinked_l, linked, stereo_link)
det_r = mix(unlinked_r, linked, stereo_link)
```

MVP は完全 linked でよい。

## gain computer

threshold/ceiling を dB で扱い、linear gain を返す。

```text
level_db = linear_to_db(peak)
allowed_db = ceiling_db
excess_db = level_db - allowed_db
if excess_db <= 0:
    target_gain_db = 0
else:
    target_gain_db = -excess_db
```

linear:

```text
target_gain = db_to_linear(target_gain_db)
```

## lookahead

lookahead によって transient 前に gain を下げる。

```text
lookahead_samples = round(lookahead_ms * sample_rate / 1000)
```

注意:

- `prepare()` で max lookahead buffer を確保
- `process_stereo()` 内では delay length を変更しても resize しない
- plugin latency を host に報告したいが、nih-plug 側で latency report が必要

MVP では固定 max lookahead を確保し、plugin 側 latency 報告は後回しでもよい。
ただし DAW plugin としては latency compensation が重要なので Phase 2 で対応する。

## release smoothing

gain reduction は attack 即時、release はゆっくり戻す。

```text
if target_gain < current_gain:
    current_gain = target_gain
else:
    current_gain += coeff * (target_gain - current_gain)
```

release coefficient:

```text
coeff = 1 - exp(-1 / (release_sec * sample_rate))
```

## true peak extension

Phase 2 で追加。

方法:

- 2x or 4x oversampling detector
- polyphase FIR or simple halfband IIR/FIR
- detector のみに oversampling をかけ、audio path は通常 sample-rate のまま

MVP は `true_peak=false` 固定でもよい。

## safety ceiling

output は ceiling を超えないように最後に clamp/soft-clip を入れるか議論する。

推奨:

- limiter の数学上は ceiling 以下になる
- safety として debug assert / optional hard clip
- plugin user 向けには hard clip fallback を入れる

```rust
let ceil = db_to_linear(params.ceiling_db);
y = y.clamp(-ceil, ceil);
```

ただし hard clip は音質劣化するため、通常は target gain で抑える。

## parameters

- Input Gain: -24..24 dB
- Threshold: -24..0 dB
- Ceiling: -24..0 dB
- Release: 1..1000 ms
- Lookahead: 0..10 ms
- Stereo Link: 0..100 %
- True Peak: off/on
- Output Gain: -24..24 dB
- Bypass

## UI

```text
[Gain]
  Input Gain
  Threshold
  Ceiling

[Timing]
  Lookahead
  Release

[Stereo]
  Stereo Link

[Meter]
  Input Peak
  Gain Reduction
  Output Peak
```

Meter は audio thread から atomic で UI thread に渡す。

## plugin wrapper

`z-audio-limiter-plugin`

```text
Plugin type: AudioEffect
Category: Dynamics / Limiter
Input layouts: mono, stereo
Output layouts: mono, stereo
```

mono の場合は片 channel だけ処理してもよいが、既存 EQ plugin と同じく scratch right を使って stereo effect を流用してもよい。

## tests

### DSP tests

- input 0.5, ceiling 0dB -> unchanged
- input 2.0, ceiling 0dB -> <= 1.0
- release が短いほど戻りが速い
- lookahead 0 と 5ms で transient 処理が変わる
- silence -> silence
- NaN/Inf なし
- reset clears delay/gain state

### plugin smoke

- VST3 bundle export
- CLAP export
- plugin loads in host
- parameter automation does not panic

## 実装順

1. dB utility 確認/追加
2. `DelayLine` 共通化
3. `ReleaseSmoother`
4. `Limiter` core
5. unit tests
6. native plugin
7. meter atomics
8. latency report
9. true peak detector
10. WebCLAP
