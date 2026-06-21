# 実装計画と完了範囲

## 今回の実装方針

`docs/z_audio_dsp_implementation_plan` と `docs/楽器を数式で実装.md` を前提に、既存構造を壊さない MVP として以下を完了範囲にした。

1. `thirdparty/z-audio-dsp/crates/z-audio-dsp` に realtime-safe な DSP core を追加する。
2. `thirdparty/z-audio-dsp/crates/z-audio-synth` に MIDI で鳴る Formula Synth / Formula Piano runtime を追加する。
3. root `z-audio-dsp-plugin` workspace に native VST3/CLAP crate と WebCLAP crate を追加する。
4. 低層 DSP と runtime は unit test で検証し、plugin workspace は `cargo check --workspace` / `cargo test --workspace` / WebCLAP wasm build で検証する。

GUI editor の詳細実装は、native / WebCLAP の UI 共通化が必要なため次フェーズに分離した。各 plugin のパラメータ公開と音声処理 wrapper は実装済み。

## 実装ステップ

### Phase 0: DSP共通基盤

- `delay::DelayLine`
- `delay::AllpassDelay`
- `math::db_to_linear`
- `math::linear_to_db`
- `dynamics::BallisticsFilter`
- `dynamics::LevelDetector`

### Phase 1: 数式シンセシス core

- `formula::FormulaOpcode`
- `formula::FormulaProgram`
- `formula::FormulaRuntime`
- built-in 6 preset
  - Pure Phase
  - Bright Fold
  - FM Bell
  - Additive Odd
  - PD Syncish
  - Chaotic Soft
- `FormulaGenerator`

### Phase 2: dynamics core

- feed-forward `Compressor`
- lookahead peak `Limiter`
- hard/soft knee gain computer
- ceiling clamp
- release smoothing

### Phase 3: parametric reverb core

- early reflection taps
- allpass diffusion chain
- 8-line Hadamard FDN
- damping lowpass
- room size / decay / mix / width / output gain

### Phase 4: Formula Piano core

参照ドキュメントのピアノ式をMVP化した。

- hammer excitation
- stiff-string inharmonicity:
  - `f_n = n f_0 sqrt(1 + B n^2)`
- velocity dependent partial gains
- modal string bank
- release damper
- stereo key panning
- body/soundboard resonator

### Phase 5: native plugin

追加 crate:

- `crates/z-audio-formula-plugin`
- `crates/z-audio-piano-plugin`
- `crates/z-audio-reverb-plugin`
- `crates/z-audio-limiter-plugin`
- `crates/z-audio-compressor-plugin`

各 crate は nih-plug の VST3/CLAP export を持ち、パラメータは DSP core の `ParamId::metadata()` を参照する。

### Phase 6: WebCLAP plugin

追加 crate:

- `crates/z-audio-webclap-formula`
- `crates/z-audio-webclap-piano`
- `crates/z-audio-webclap-reverb`
- `crates/z-audio-webclap-limiter`
- `crates/z-audio-webclap-compressor`

既存 `wclap-plugin` runtime に合わせて、CLAP ABI / params / process wrapper と `plugin.json` / `xtask bundle-webclap` の配布経路を実装した。UI path は未設定で、ホスト側の標準パラメータ表示を使う。

## 検証コマンド

```powershell
cd C:\Users\zukky\Desktop\z-audio-dsp-plugin\thirdparty\z-audio-dsp
cargo test -p z-audio-dsp
cargo test -p z-audio-synth

cd C:\Users\zukky\Desktop\z-audio-dsp-plugin
cargo check --workspace
cargo test --workspace
cargo check --target wasm32-unknown-unknown `
  -p z-audio-webclap `
  -p z-audio-webclap-eq `
  -p z-audio-webclap-formula `
  -p z-audio-webclap-piano `
  -p z-audio-webclap-reverb `
  -p z-audio-webclap-limiter `
  -p z-audio-webclap-compressor
cargo xtask bundle-webclap --release
```

## 次フェーズ

- egui editor の共通部品化
- WebCLAP UI asset の共通部品化
- compressor / limiter meter atomics
- limiter latency compensation
- true peak detector
- reverb modulation depth/rate の実処理
- piano sympathetic resonance
