# 00. Plugin Overview

## 依存DSPコア

DSPコアは https://github.com/SuzukiDaishi/z-audio-dsp.git の以下のcrateを使います。

```text
z-audio-dsp     # pure DSP (generators / modulators / effects / params)
z-audio-synth   # SimpleSynth runtime
```

このリポジトリのroadmap (`docs/z-audio-dsp_dsp_plan/11_final_goal_roadmap.md`) で言う
`v0.2: Plugin-ready Synth Core`（parameter metadata / 安定process API / voice stealing /
automation smoothing）は2026-06-14時点で実装済みのため、本plugin計画はそのAPIを前提に進めます。

## 目的

`z-audio-synth::SimpleSynth` をプラグインとして公開します。

第一弾プラグイン名:

```text
Z Audio Simple Synth
```

内部DSP:

```text
z-audio-dsp
z-audio-synth
```

出力形式:

```text
VST3
CLAP
WebCLAP
```

## GUI

第一弾では GUI なし。

理由:

- DSPコアとplugin wrapperを先に安定化させたい
- DAWのgeneric editorでパラメータ操作できる
- WebCLAPではUI仕様がまだ実験的になりやすい
- GUIを入れるとVST3/CLAP/WebCLAP間で差異が大きくなる

## Plugin Signal Flow

```text
Host Events / MIDI
  -> Plugin Adapter
  -> z_audio_synth::SimpleSynth
  -> Audio Output
```

## Audio I/O

シンセ第一弾:

```text
MIDI/Event in
Stereo audio out
```

将来的には Effect版も作ります。

```text
Stereo audio in
Stereo audio out
```

## Formats

### VST3

- DAW互換性のため必須
- 第一候補は NIH-plug 経由
- GUIなしgeneric UI

### CLAP

- Phase Plant的な近代的パラメータ/イベント設計と相性がよい
- 第一候補は NIH-plug 経由
- 将来的にはnative CLAP adapterも検討

### WebCLAP

- ブラウザ上でCLAP compiled to WASMを動かす
- 実験的ターゲット
- AudioWorklet / WASM / JS host bridge が必要
- 第一弾では動作確認用の最小構成を目指す
