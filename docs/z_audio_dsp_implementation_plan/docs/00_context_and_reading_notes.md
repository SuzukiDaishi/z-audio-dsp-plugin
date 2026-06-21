# 00. 現状 repo 読解メモと設計前提

## 対象 repo

- `https://github.com/SuzukiDaishi/z-audio-dsp`
- `https://github.com/SuzukiDaishi/z-audio-dsp-plugin`

## 現状構成の前提

`z-audio-dsp` は DSP core 側の workspace として扱う。
現在の workspace は以下の3 crate 構成。

```text
z-audio-dsp/
  Cargo.toml
  crates/
    z-audio-dsp/        # pure DSP library
    z-audio-synth/      # voice / synth runtime
    z-audio-examples/   # examples
```

`z-audio-dsp-plugin` は plugin wrapper 側の workspace として扱う。
現在の workspace は以下の crate 構成。

```text
z-audio-dsp-plugin/
  Cargo.toml
  crates/
    z-audio-plugin/          # native synth plugin
    z-audio-eq-plugin/       # native EQ plugin
    z-audio-webclap/         # WebCLAP synth wrapper
    z-audio-webclap-eq/      # WebCLAP EQ wrapper
    wclap-plugin/            # WebCLAP runtime glue
    xtask/                   # build/package helper
  thirdparty/
    z-audio-dsp/             # submodule, workspace から exclude
```

## 現状設計の良い点

- DSP core と plugin wrapper が分離されている。
- DSP 側は `Generator`, `Modulator`, `Effect` の trait に分かれている。
- plugin 側は native VST3/CLAP と WebCLAP を分けられる構成になっている。
- `ParamId` によって DSP core 側の安定パラメータ ID を持たせられる。
- 既存の `SimpleSynth` は固定チェーンだが、最初の製品化に必要な最小構成としては扱いやすい。

## 今回追加するものの分類

今回の追加物は、既存構造に合わせて以下の3階層に分ける。

### 1. pure DSP module

`crates/z-audio-dsp/src/` に入れる。

- formula synthesis engine
- modal resonator / string model utilities
- dynamics processors
- reverb processors
- envelope followers
- delay lines / ring buffers
- smoothing / parameter mapping utilities

### 2. musical runtime / instrument layer

`crates/z-audio-synth/src/` に入れるか、新 crate `crates/z-audio-instruments/` を追加する。

最初は既存構成を壊さないため、以下を推奨する。

```text
crates/z-audio-synth/src/
  formula_voice.rs
  formula_synth.rs
  piano/
    mod.rs
    voice.rs
    preset.rs
    model.rs
```

ただしピアノが大きくなるなら次段階で独立 crate にする。

```text
crates/z-audio-instruments/
  src/
    piano/
    formula_synth/
```

### 3. plugin wrapper layer

`z-audio-dsp-plugin/crates/` に追加する。

```text
crates/
  z-audio-piano-plugin/
  z-audio-reverb-plugin/
  z-audio-limiter-plugin/
  z-audio-compressor-plugin/
  z-audio-webclap-piano/
  z-audio-webclap-reverb/
  z-audio-webclap-limiter/
  z-audio-webclap-compressor/
```

最初から全部を作ると workspace が肥大化するため、実装順は以下を推奨する。

1. DSP core only の FormulaGenerator
2. DSP core only の Compressor / Limiter
3. native plugin only の Compressor / Limiter
4. DSP core only の Reverb
5. native plugin only の Reverb
6. Piano runtime
7. native Piano plugin
8. WebCLAP 対応

## リアルタイム安全性の共通方針

`process_*` 中は以下を禁止する。

- heap allocation
- lock / mutex wait
- file I/O
- logging
- panic
- dynamic dispatch の多用
- String parse / expression parse
- Vec push など capacity 変更の可能性がある処理

式のコンパイル、プリセット読み込み、buffer resize、IR / modal table 生成などはすべて `prepare()` または UI thread / non-audio thread で行う。

## ドキュメント全体の読み方

- `01_architecture_roadmap.md`: 全体ロードマップ
- `02_formula_synthesis_core.md`: 数式シンセシス core
- `03_formula_piano_design.md`: ピアノ instrument
- `04_parametric_reverb.md`: パラメトリックリバーブ
- `05_limiter.md`: リミッター
- `06_compressor.md`: コンプレッサー
- `07_plugin_integration.md`: VST3/CLAP/WebCLAP 統合
- `08_parameters_and_ui.md`: パラメータ・UI・automation
- `09_tests_benchmarks_validation.md`: テスト・検証
- `10_milestones_tasks.md`: 実装タスク分解
- `11_code_templates.md`: 実装テンプレート
