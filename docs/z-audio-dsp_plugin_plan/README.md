# z-audio-dsp Plugin / WebCLAP 実装計画

作成日: 2026-06-14

このZIPは、`z-audio-dsp` DSPコアを使った **VST3 / CLAP / WebCLAP** ラッパー実装計画です。  
DSP本体のcargo library計画は別ZIPに分離しています。

DSPコアの実体は https://github.com/SuzukiDaishi/z-audio-dsp.git
（`z-audio-dsp` / `z-audio-synth` crate、`docs/z-audio-dsp_dsp_plan/`に本家側計画あり）。
2026-06-14時点で本家のSimpleSynth実装は本ZIPが前提とするAPI
（`SimpleSynth::new` / `process_with_context` / `ParamId::ALL` 27種 + `metadata()`）と
一致していることを確認済みです（詳細は[REFERENCES.md](REFERENCES.md)）。

## 第一弾の成果物

GUIなしのシンプルなシンセを、以下の形式でビルドできるようにします。

- VST3
- CLAP
- WebCLAP

## 基本方針

- DSP本体は `z-audio-dsp` / `z-audio-synth` に完全分離
- プラグイン層は薄いadapterにする
- GUIなし
- DAWのgeneric parameter UIを使う
- WebCLAPは実験的扱いにして、VST3/CLAPより段階を分ける

## このZIP内のファイル

- `00_plugin_overview.md`
- `01_wrapper_architecture.md`
- `02_vst3_clap_nih_plug_plan.md`
- `03_clap_native_plan.md`
- `04_webclap_plan.md`
- `05_parameters_automation_presets.md`
- `06_build_ci_release.md`
- `07_host_testing_matrix.md`
- `08_milestones.md`
- `09_risks_and_decisions.md`
- `REFERENCES.md`
