# 01. 全体アーキテクチャとロードマップ

## 目的

`z-audio-dsp` を、ゲームランタイム / DAW plugin / WebCLAP で共通利用できる pure Rust DSP core に育てる。

今回のゴールは、以下を「独立して実装可能な粒度」まで設計すること。

- 数式ベースのシンセシス
- 物理・数式ハイブリッドピアノ
- パラメトリックリバーブ
- リミッター
- コンプレッサー

## 基本レイヤー

```text
┌──────────────────────────────────────────────┐
│ z-audio-dsp-plugin                            │
│  - VST3 / CLAP native wrapper                 │
│  - WebCLAP wrapper                            │
│  - GUI / parameters / preset I/O              │
└──────────────────────────────────────────────┘
                    │
                    ▼
┌──────────────────────────────────────────────┐
│ z-audio-synth / z-audio-instruments           │
│  - voice management                           │
│  - MIDI note handling                         │
│  - formula synth voice                        │
│  - piano voice                                │
└──────────────────────────────────────────────┘
                    │
                    ▼
┌──────────────────────────────────────────────┐
│ z-audio-dsp                                   │
│  - Generator / Modulator / Effect             │
│  - formula VM / opcodes                       │
│  - filters / delay lines / dynamics           │
│  - reverb / compressor / limiter              │
└──────────────────────────────────────────────┘
```

## crate 追加方針

### z-audio-dsp 側

既存の `crates/z-audio-dsp` に module を追加する。

```text
crates/z-audio-dsp/src/
  formula/
    mod.rs
    opcode.rs
    program.rs
    compiler.rs
    runtime.rs
    generator.rs
    builtins.rs
  resonators/
    mod.rs
    modal_bank.rs
    string.rs
    body.rs
  delay/
    mod.rs
    delay_line.rs
    allpass.rs
    modulated_delay.rs
  dynamics/
    mod.rs
    envelope_follower.rs
    compressor.rs
    limiter.rs
    ballistics.rs
    oversampling.rs
  reverb/
    mod.rs
    early_reflections.rs
    fdn.rs
    parametric_reverb.rs
  smoothing/
    mod.rs
    linear_smoother.rs
    exp_smoother.rs
```

最初から全 module を作る必要はないが、namespace は早めに切る。

### z-audio-synth 側

既存 `SimpleSynth` を壊さずに、新しい runtime を追加する。

```text
crates/z-audio-synth/src/
  formula_synth.rs
  formula_voice.rs
  piano/
    mod.rs
    voice.rs
    model.rs
    params.rs
    preset.rs
```

### z-audio-dsp-plugin 側

native plugin は nih-plug ベース、WebCLAP は既存 `wclap-plugin` runtime を使う。

```text
crates/
  z-audio-formula-plugin/
  z-audio-piano-plugin/
  z-audio-reverb-plugin/
  z-audio-limiter-plugin/
  z-audio-compressor-plugin/
```

WebCLAP は後追いで追加。

```text
crates/
  z-audio-webclap-formula/
  z-audio-webclap-piano/
  z-audio-webclap-reverb/
  z-audio-webclap-limiter/
  z-audio-webclap-compressor/
```

## 共有すべき utility

以下は全機能で使うので `z-audio-dsp` に作る。

### Parameter smoothing

- `LinearSmoother`
- `ExponentialSmoother`
- `SmoothedValue`

用途:

- reverb size / damping / mix
- compressor threshold / ratio / makeup
- limiter ceiling / release
- formula macro parameters
- piano brightness / hammer hardness

### Delay line

- fractional delay
- integer delay
- modulated delay
- allpass delay

用途:

- reverb
- piano string
- chorus-like detune
- sympathetic resonance

### Envelope follower / ballistics

- peak follower
- RMS follower
- attack/release smoothing
- hold

用途:

- compressor
- limiter
- piano hammer envelope optional

### Parameter metadata

現状の `ParamId` へ追加する。

各 plugin 側で param を独自定義すると破綻するので、DSP core 側に stable ID と metadata を置く。

## 実装順

### Phase 0: safety foundation

- smoothing utility
- delay line utility
- envelope follower utility
- test helper
- no-denormal helper の徹底

### Phase 1: formula synthesis MVP

- `FormulaProgram`
- opcode runtime
- curated built-in formula list
- `FormulaGenerator`
- mono output only
- no user text parser yet

### Phase 2: dynamics processors

- compressor core
- limiter core
- unit tests
- native plugin wrappers

### Phase 3: parametric reverb

- early reflections
- 8-line FDN
- damping filters
- diffusion allpass
- wet/dry/mix
- native plugin wrapper

### Phase 4: formula piano prototype

- modal piano voice
- hammer excitation
- damper envelope
- body resonator
- velocity mapping
- native piano plugin

### Phase 5: WebCLAP

- Formula plugin WebCLAP
- Dynamics WebCLAP
- Reverb WebCLAP
- Piano WebCLAP

## 重要な設計判断

### ユーザー入力の数式文字列を最初から audio thread で評価しない

数式ベースシンセという名前でも、最初は以下にする。

- preset / builtin formula を Rust code として定義
- もしくは UI thread で文字列を compile して `FormulaProgram` にする
- audio thread は stack VM を回すだけ

これにより、リアルタイム安全性を保ちつつ将来の文字列式 editor に拡張できる。

### Piano は sample playback ではなく physical / modal 寄りにする

今回の目的が「数式ベースの組み合わせ」なので、ピアノは以下のハイブリッドにする。

- hammer impulse / noise excitation
- stiff string partials / modal bank
- body resonance
- damper / release model
- sympathetic resonance optional

### Compressor と Limiter は同じ dynamics 基盤から作る

共通部品:

- detector
- ballistics
- gain computer
- gain smoother
- lookahead delay

Compressor と Limiter の違いは gain computer と lookahead / ceiling の扱い。

## 成果物の最小構成

最初に merge する PR は小さくする。

```text
PR 1: dsp utilities
PR 2: formula core MVP
PR 3: compressor core + tests
PR 4: limiter core + tests
PR 5: compressor/limiter native plugin
PR 6: reverb core
PR 7: reverb native plugin
PR 8: piano core prototype
PR 9: piano native plugin
PR 10: WebCLAP wrappers
```
