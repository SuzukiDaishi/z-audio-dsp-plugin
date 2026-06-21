# 03. 数式シンセシスを組み合わせたピアノ設計

## ゴール

`z-audio-dsp` の数式ベースシンセシス、modal resonator、delay line、filter を組み合わせて、sample playback ではないピアノ音源を作る。

plugin としては `z-audio-dsp-plugin` に `Z Audio Formula Piano` を追加する。

## 方針

完全な物理モデルピアノは大きすぎるため、最初は以下のハイブリッドにする。

```text
MIDI Note
  -> Hammer Excitation
  -> String / Modal Resonator Bank
  -> Damper / Release Model
  -> Body Resonance
  -> Optional Sympathetic Resonance
  -> Output EQ / Limiter
```

## Piano voice architecture

```text
PianoVoice
  ├─ note, velocity, frequency
  ├─ HammerModel
  ├─ StringModel
  │   ├─ ModalBank low/mid
  │   └─ WaveguideString optional high
  ├─ DamperModel
  ├─ BodyResonator
  ├─ stereo panner
  └─ state: Active / Releasing / Idle
```

## 実装 module

`z-audio-dsp` 側:

```text
crates/z-audio-dsp/src/resonators/
  mod.rs
  modal_bank.rs
  biquad_resonator.rs
  string_dispersion.rs
  body.rs

crates/z-audio-dsp/src/excitation/
  mod.rs
  hammer.rs
```

`z-audio-synth` 側:

```text
crates/z-audio-synth/src/piano/
  mod.rs
  voice.rs
  voice_manager.rs
  params.rs
  model.rs
  preset.rs
```

`z-audio-dsp-plugin` 側:

```text
crates/z-audio-piano-plugin/
  Cargo.toml
  src/lib.rs
  src/params.rs
  src/editor.rs
```

## Hammer model

Hammer は短い excitation signal を作る。

### MVP hammer

```text
hammer(t) = velocity_gain * exp(-t / tau) * (sin(2π f_hammer t) + noise * noise_amount)
```

parameters:

- hardness
- velocity curve
- noise amount
- attack time
- hammer tone

実装:

```rust
pub struct HammerExciter {
    sample_rate: f32,
    phase: f32,
    time_samples: u32,
    duration_samples: u32,
    velocity: f32,
    hardness: f32,
    noise_amount: f32,
}

impl HammerExciter {
    pub fn trigger(&mut self, velocity: f32, frequency_hz: f32);
    pub fn next_sample(&mut self) -> f32;
    pub fn is_done(&self) -> bool;
}
```

## String / modal model

### MVP: modal bank

各 note に対して partial を複数持つ。

```text
y[n] = Σ_i g_i * resonator_i(excitation)
```

各 partial:

```text
f_i = fundamental * i * sqrt(1 + B * i^2)
```

- `B`: inharmonicity coefficient
- low note ほど B が大きめ
- high note は partial 数を少なくする

### resonator 実装

2-pole resonator を使う。

```text
r = exp(-π * bandwidth / sample_rate)
theta = 2π * frequency / sample_rate
y[n] = b0*x[n] + 2*r*cos(theta)*y[n-1] - r^2*y[n-2]
```

decay time から bandwidth を求める。

```text
r = exp(-1 / (decay_seconds * sample_rate))
```

### partial table

MVP は procedural table。

```rust
pub struct PianoPartial {
    pub ratio: f32,
    pub gain: f32,
    pub decay_sec: f32,
    pub pan: f32,
}

pub struct PianoNoteModel {
    pub frequency_hz: f32,
    pub inharmonicity: f32,
    pub partials: [PianoPartial; MAX_PIANO_PARTIALS],
    pub partial_count: usize,
}
```

## Body resonance

ピアノらしさは string だけでは出ないため、body resonator を後段に置く。

MVP:

- low shelf / high shelf
- 3〜6本の broad resonator
- stereo width

```text
string sum -> BodyResonator -> stereo output
```

body resonator candidate:

- 120 Hz: soundboard low resonance
- 240 Hz: warm body
- 500 Hz: box tone
- 1.2 kHz: presence
- 3.5 kHz: hammer attack

## Damper / release model

NoteOff で即停止すると不自然なので、release damper を入れる。

- sustain pedal off: release short
- sustain pedal on: release long
- high notes: damper の影響を弱くする

```rust
pub enum PianoVoiceState {
    Idle,
    Attack,
    Sustain,
    Release,
}
```

release では modal resonator の input は止めるが、resonator state は decay させる。

## Sympathetic resonance

Phase 2 以降。

簡易版:

- pedal down 中のみ global resonator bank を鳴らす
- input は全 voice の dry string sum
- note に近い harmonic のみ反応

```text
sum(strings) -> SympatheticBank -> small wet mix
```

MVP では off にしておく。

## 数式シンセとの接続

ピアノ内で formula synthesis を使う場所:

1. Hammer excitation shape
2. partial gain morph
3. body tone nonlinear shaping
4. attack noise color

例えば hammer excitation を FormulaProgram で定義する。

```text
hammer = exp(-t * decay) * (
  sin(phase * hammer_ratio)
  + macro_noise * noise_hash(t)
  + macro_click * exp(-t * click_decay)
)
```

## Parameters

### Piano global

- Master Gain
- Polyphony
- Tone
- Brightness
- Hammer Hardness
- Hammer Noise
- Inharmonicity
- Decay
- Release
- Body Amount
- Stereo Width
- Sympathetic Amount
- Pedal Resonance

### MIDI

- NoteOn velocity
- NoteOff velocity optional
- Sustain pedal CC64
- Soft pedal CC67 optional

## Voice allocation

既存 `VoiceManager` を流用できるなら流用する。
ただし piano は long release が多いため、voice stealing policy を調整する。

推奨 policy:

1. Idle voice
2. Releasing かつ envelope が小さい voice
3. 同 note の古い voice
4. 最も音量が小さい voice
5. 最古 voice

## Plugin UI

最初は 1画面でよい。

```text
[Character]
  Tone
  Brightness
  Hammer Hardness
  Body

[Physical]
  Inharmonicity
  Decay
  Release
  Stereo Width

[Pedal]
  Sustain Resonance
  Sympathetic Amount

[Output]
  Gain
  Safety Limiter On/Off
```

## 実装ステップ

### Step 1: ModalBank を作る

- single resonator
- multiple resonator bank
- impulse input test
- decay time test

### Step 2: PianoVoice MVP

- NoteOn で hammer を鳴らす
- hammer output を modal bank に入力
- output を stereo に pan
- NoteOff で release state

### Step 3: PianoSynth runtime

- polyphonic voice manager
- MIDI event handling
- global output gain
- sustain pedal

### Step 4: Native plugin

- `z-audio-piano-plugin`
- nih-plug params
- MIDI input
- stereo output
- basic egui editor

### Step 5: Quality improvements

- partial table tuning
- velocity mapping
- body resonator
- sympathetic resonance
- internal limiter

## テスト

- NoteOn で非ゼロ出力
- NoteOff 後に自然 decay
- 128 voices でも panic なし
- output NaN/Inf なし
- velocity 0.1 と 1.0 の loudness 差
- sustain pedal on/off
- sample rate 44.1 / 48 / 96 kHz

## 注意点

ピアノ音色はパラメータの調整が非常に重要。
最初からリアルピアノを目指しすぎるより、以下の順序がよい。

1. toy piano / bell piano 的に鳴る
2. attack と decay が自然
3. low note の濁りを減らす
4. body resonance を足す
5. sympathetic resonance を足す
6. velocity layer の表現力を上げる
