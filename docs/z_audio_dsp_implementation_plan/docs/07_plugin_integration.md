# 07. z-audio-dsp-plugin 統合計画

## ゴール

DSP core に追加した Formula / Piano / Reverb / Limiter / Compressor を、native VST3/CLAP と WebCLAP に展開する。

## 既存方針

`z-audio-dsp-plugin` は plugin wrapper repo として扱う。
`thirdparty/z-audio-dsp` を submodule として参照する。

既存 workspace は native synth / EQ / WebCLAP / xtask が分かれているので、同じスタイルで crate を追加する。

## workspace 追加案

`Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/z-audio-plugin",
    "crates/z-audio-eq-plugin",
    "crates/z-audio-formula-plugin",
    "crates/z-audio-piano-plugin",
    "crates/z-audio-reverb-plugin",
    "crates/z-audio-limiter-plugin",
    "crates/z-audio-compressor-plugin",
    "crates/z-audio-webclap",
    "crates/z-audio-webclap-eq",
    "crates/z-audio-webclap-formula",
    "crates/z-audio-webclap-piano",
    "crates/z-audio-webclap-reverb",
    "crates/z-audio-webclap-limiter",
    "crates/z-audio-webclap-compressor",
    "crates/wclap-plugin",
    "crates/xtask",
]
exclude = [
    "thirdparty/z-audio-dsp",
]
```

ただし最初は native only でよい。
WebCLAP crate は Phase 5 で追加する。

## Native plugin 共通構成

各 plugin crate は以下。

```text
src/
  lib.rs
  params.rs
  editor.rs
  engine.rs   # 必要なら
```

`lib.rs` は nih-plug plugin 実装。
`params.rs` は nih-plug params。
`engine.rs` は DSP core との adapter。

## 依存関係

各 plugin の `Cargo.toml` 例。

```toml
[package]
name = "z-audio-reverb-plugin"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib", "lib"]

[dependencies]
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git" }
nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git" }
z-audio-dsp = { path = "../../thirdparty/z-audio-dsp/crates/z-audio-dsp" }
```

Piano / Formula instrument は `z-audio-synth` も参照する。

```toml
z-audio-synth = { path = "../../thirdparty/z-audio-dsp/crates/z-audio-synth" }
```

## Plugin naming

### Formula Synth

- Name: `Z Audio Formula Synth`
- Vendor: `Z Audio`
- Category: Synth
- CLAP ID: `dev.zaudio.formula-synth`
- VST3 Class ID: `ZAudioFormulaSyn`

### Formula Piano

- Name: `Z Audio Formula Piano`
- Category: Instrument
- CLAP ID: `dev.zaudio.formula-piano`
- VST3 Class ID: `ZAudioFormulaPno`

### Parametric Reverb

- Name: `Z Audio Parametric Reverb`
- Category: Reverb
- CLAP ID: `dev.zaudio.parametric-reverb`
- VST3 Class ID: `ZAudioParaReverb`

### Limiter

- Name: `Z Audio Limiter`
- Category: Dynamics
- CLAP ID: `dev.zaudio.limiter`
- VST3 Class ID: `ZAudioLimiter000`

### Compressor

- Name: `Z Audio Compressor`
- Category: Dynamics
- CLAP ID: `dev.zaudio.compressor`
- VST3 Class ID: `ZAudioCompressor`

VST3 Class ID は長さ制限と衝突に注意する。

## Parameter sync 方針

### 現状 adapter の問題

plugin param の変更を block 先頭で DSP event に変換する方式はシンプルだが、sample accurate automation ではない。

### MVP

- block 先頭同期でよい
- DSP core 側に smoothing を入れる
- automation が激しい param だけ smoothing 強め

### Phase 2

- nih-plug の sample accurate automation を調査して adapter を更新
- `TimedEvent { sample_offset, kind: Param }` を活かす

## Effect plugin process adapter

compressor / limiter / reverb は `Effect` trait なのでほぼ共通化できる。

```rust
pub struct EffectEngine<E: Effect> {
    effect: E,
    scratch_r: Vec<f32>,
    sample_rate: f32,
    max_block_size: usize,
}
```

ただし `Vec` は `initialize()` / `prepare` で確保する。

mono handling:

```text
mono input:
  left = channel 0
  right = scratch copy of left or zero
  process_stereo
  output = left or stereo downmix
```

reverb は mono in stereo out が自然。
compressor/limiter は mono in mono out が自然。

## Instrument plugin process adapter

Formula Synth / Piano は MIDI input を受ける。

共通:

- MIDI note on/off
- sustain pedal
- pitch bend optional
- sample offset event
- output stereo

adapter は既存 `z-audio-plugin` の構成を参考にする。

## WebCLAP 方針

WebCLAP crate は native plugin と別にする。

理由:

- wasm32-unknown-unknown target
- JS glue なし
- no_std に近い制限を考慮
- nih-plug は使わない

共通化するなら、plugin metadata と param mapping は別 crate に切る。

```text
crates/z-audio-plugin-common/
  src/
    formula_params.rs
    piano_params.rs
    dynamics_params.rs
    reverb_params.rs
```

ただし最初は重複を許容してよい。

## xtask 更新

`crates/xtask` に build target を追加する。

```text
cargo xtask bundle formula
cargo xtask bundle piano
cargo xtask bundle reverb
cargo xtask bundle limiter
cargo xtask bundle compressor
cargo xtask bundle all
```

WebCLAP:

```text
cargo xtask webclap formula
cargo xtask webclap piano
cargo xtask webclap reverb
cargo xtask webclap limiter
cargo xtask webclap compressor
```

## CI / build check

追加したい CI:

- cargo fmt --check
- cargo clippy --workspace --all-targets
- cargo test --workspace
- cargo build -p z-audio-compressor-plugin
- cargo build -p z-audio-limiter-plugin
- cargo build -p z-audio-reverb-plugin
- wasm32 build smoke test

## 実装順

1. Compressor native plugin
2. Limiter native plugin
3. Reverb native plugin
4. Formula Synth native plugin
5. Piano native plugin
6. WebCLAP compressor / limiter
7. WebCLAP reverb
8. WebCLAP formula / piano
