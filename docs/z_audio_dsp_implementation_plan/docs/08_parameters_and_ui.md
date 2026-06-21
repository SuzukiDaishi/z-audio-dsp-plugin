# 08. パラメータ・UI・automation 設計

## ゴール

DSP core と plugin wrapper のパラメータ不一致を避ける。

- DSP core 側に stable `ParamId` を追加する
- plugin 側は `ParamMetadata` から nih-plug params を構築する方針に近づける
- UI 表示名、単位、範囲、default を統一する

## ParamId 拡張案

既存 `ParamId` に以下を追加する。

### Formula

```rust
FormulaProgram,
FormulaMacro1,
FormulaMacro2,
FormulaMacro3,
FormulaMacro4,
FormulaMacro5,
FormulaMacro6,
FormulaMacro7,
FormulaMacro8,
FormulaOutputGain,
FormulaOversampling,
FormulaDcBlock,
```

### Piano

```rust
PianoTone,
PianoBrightness,
PianoHammerHardness,
PianoHammerNoise,
PianoInharmonicity,
PianoDecay,
PianoRelease,
PianoBodyAmount,
PianoStereoWidth,
PianoSympatheticAmount,
PianoPedalResonance,
PianoMasterGain,
```

### Reverb

```rust
ReverbMix,
ReverbRoomSize,
ReverbDecay,
ReverbPreDelay,
ReverbDiffusion,
ReverbDamping,
ReverbLowCut,
ReverbHighCut,
ReverbModRate,
ReverbModDepth,
ReverbWidth,
ReverbEarlyLateMix,
ReverbOutputGain,
```

### Limiter

```rust
LimiterInputGain,
LimiterThreshold,
LimiterCeiling,
LimiterRelease,
LimiterLookahead,
LimiterStereoLink,
LimiterTruePeak,
LimiterOutputGain,
```

### Compressor

```rust
CompressorInputGain,
CompressorThreshold,
CompressorRatio,
CompressorKnee,
CompressorAttack,
CompressorRelease,
CompressorMakeupGain,
CompressorMix,
CompressorDetectorMode,
CompressorStereoLink,
CompressorSidechainHpf,
```

## ParamMetadata

各 param に以下を持たせる。

```rust
pub struct ParamMetadata {
    pub id: ParamId,
    pub name: &'static str,
    pub unit: ParamUnit,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub automatable: bool,
    pub logarithmic: bool,
}
```

必要なら `display_transform` は plugin 側で持つ。

## 単位

```rust
pub enum ParamUnit {
    Unitless,
    Percent,
    Hertz,
    Decibels,
    Milliseconds,
    Seconds,
    Ratio,
    Enum,
    Bool,
}
```

## smoothing 必須 param

### Formula

- macros
- output gain

### Piano

- tone
- brightness
- body amount
- stereo width
- master gain

### Reverb

- mix
- room size
- decay
- damping
- high cut
- low cut
- width
- output gain

### Dynamics

- threshold
- ratio
- attack/release
- makeup/output gain
- mix

## UI 方針

### 共通

- Advanced を折りたたむ
- Meter は audio thread から atomic read
- param grouping を明確にする
- default に戻す操作を用意

### Formula Synth UI

```text
[Formula]
  Program selector
  Macro 1..8 knobs

[Voice]
  Polyphony
  Glide optional

[Output]
  Gain
  Oversampling
```

### Piano UI

```text
[Character]
  Tone / Brightness / Hammer

[Physical]
  Inharmonicity / Decay / Release

[Body]
  Body Amount / Stereo Width / Sympathetic

[Output]
  Master Gain / Safety Limiter
```

### Reverb UI

```text
[Space]
  Room Size / Decay / PreDelay

[Texture]
  Diffusion / Damping / Mod Rate / Mod Depth

[Tone]
  Low Cut / High Cut

[Mix]
  Mix / Early-Late / Width / Output
```

### Limiter UI

```text
[Main]
  Threshold / Ceiling / Release

[Lookahead]
  Lookahead / True Peak

[Meter]
  In / GR / Out
```

### Compressor UI

```text
[Main]
  Threshold / Ratio / Knee

[Timing]
  Attack / Release

[Gain]
  Makeup / Mix

[Detector]
  Peak/RMS / Sidechain HPF / Stereo Link
```

## automation 対応

### MVP

- block 先頭で params を読む
- DSP core 側 smoothing で zipper noise を抑える

### Phase 2

- sample accurate automation event を使う
- `TimedEvent` の sample_offset を尊重
- parameter event queue を block 内で処理

## preset format

将来的に plugin とゲームランタイムで同じ preset を使いたい。

提案:

```json
{
  "schema": "dev.zaudio.plugin-preset.v1",
  "plugin": "z-audio-parametric-reverb",
  "version": "0.1.0",
  "params": {
    "ReverbMix": 0.35,
    "ReverbDecay": 2.4
  }
}
```

Formula program を保存する場合:

```json
{
  "formula_program": {
    "kind": "builtin",
    "id": "bright_fold"
  }
}
```

user formula は Phase 2 以降。
