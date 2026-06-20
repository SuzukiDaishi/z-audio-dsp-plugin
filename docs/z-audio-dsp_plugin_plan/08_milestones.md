# 08. Plugin/Web Milestones

## Phase P0: DSP API freeze for plugin — 完了済み (確認 2026-06-14)

z-audio-dsp本家 (https://github.com/SuzukiDaishi/z-audio-dsp.git) 側で
以下が実装・テスト済みであることを確認しました。

- `SimpleSynth::new(SimpleSynthConfig)` （内部でprepare完了、再prepare APIはなし）
- `SimpleSynth::process(left, right)` / `process_with_context(&ctx, left, right)`
- `SimpleSynth::note_on/note_off/set_param/param_value`
- `ParamId::ALL` (27種) + `ParamId::metadata()`
- voice stealing (`VoiceStealPolicy`)
- automation smoothing (EQ周波数/master gain)
- no allocation during process（DSP側のreal-time safety方針として明記済み）

残作業: pluginクレート側でこのAPIに対するthin adapterを書くこと
（[01_wrapper_architecture.md](01_wrapper_architecture.md)）。

Deliverable:

```text
z-audio-synth callable from plugin crate
```

## Implementation status (2026-06-14)

`thirdparty/z-audio-dsp`（git submodule）を path dependency として、以下のレイアウトで
P1-P5を実装済みです。

```text
crates/
├── z-audio-plugin/    # P1-P4: NIH-plug VST3+CLAP (nih_export_clap!/nih_export_vst3!)
├── xtask/             # cargo xtask bundle (nih_plug_xtask)
└── z-audio-webclap/   # P5: WASM + AudioWorklet MVP (wasm-bindgen, web/ harness)
```

- P1-P3: `ZAudioSimpleSynth`（`crates/z-audio-plugin/src/lib.rs`）が`Plugin`/`ClapPlugin`/`Vst3Plugin`を実装。
  `cargo xtask bundle z-audio-plugin --release` で `target/bundled/Z Audio Simple Synth.{vst3,clap}` を生成。
- P4: `#[derive(Params)]`によりstate save/loadはNIH-plug側で自動化
  （[05_parameters_automation_presets.md](05_parameters_automation_presets.md)参照）。
- P5: `crates/z-audio-webclap`が`SynthProcessor`(wasm-bindgen)を公開し、
  `crates/z-audio-webclap/web/`のAudioWorkletハーネスから駆動できる。

## Phase P1: NIH-plug skeleton

- plugin crate作成
- params定義
- process skeleton
- silent output

Deliverable:

```text
empty VST3/CLAP loads in DAW
```

## Phase P2: MIDI -> DSP

- MIDI note event mapping
- SimpleSynth発音
- stereo output

Deliverable:

```text
DAW keyboard/MIDI clipで音が鳴る
```

## Phase P3: Parameters

- generator type
- envelope
- LFO
- EQ
- master gain
- generic UIで操作

Deliverable:

```text
DAW generic UIから音色変更
```

## Phase P4: State save/load

- serialize params
- restore state
- DAW project reload test

Deliverable:

```text
project reloadで音色復元
```

## Phase P5: Web AudioWorklet MVP

- wasm build
- AudioWorklet bridge
- JS note trigger
- JS param message

Deliverable:

```text
browser demoで音が鳴る
```

## Phase P6: WebCLAP investigation/prototype

- minimal WCLAP load test
- browser-test-host compatibility
- z-audio-synth adapter検討

Deliverable:

```text
minimal WebCLAP proof of concept
```

### Findings (2026-06-14): deferred

実WebCLAP（CLAP ABIをWASM上で動かし `browser-test-host` と連携させる Stage2/3）を調査した結果、
以下の理由で現時点では非現実的と判断し、deferredとしました。

- `browser-test-host`はEmscripten前提のC++ホストであり、Rust製pluginをそのまま組み込めない。
- RustのCLAP実装crateである`clack`は`wasm32-unknown-unknown`ターゲットに対応していない。

詳細な調査経緯は[04_webclap_plan.md](04_webclap_plan.md)の「Stage 2/3 Deferral Findings」節を参照。
代わりに、`crates/z-audio-webclap`（pure WASM + AudioWorklet, Stage1 MVP）を実質的なWeb向け
成果物として実装済みです（[Implementation status](#implementation-status-2026-06-14)参照）。

## Phase P7: Release packaging

- Windows/macOS/Linux bundles
- Web demo zip
- install docs
- host test report

Deliverable:

```text
Z Audio Simple Synth v0.1 plugin release candidate
```
