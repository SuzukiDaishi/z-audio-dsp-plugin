# 10. マイルストーンとタスク分解

## Milestone 0: 共通基盤

### z-audio-dsp

- [ ] `smoothing` module を作る
- [ ] `delay` module を作る
- [ ] `dynamics/envelope_follower.rs` を作る
- [ ] dB utility を確認・不足分を追加
- [ ] no-denormal helper を全 DSP に使う
- [ ] common tests を追加

完了条件:

- `cargo test -p z-audio-dsp` が通る
- process 内 allocation がない

## Milestone 1: Formula synthesis MVP

### z-audio-dsp

- [ ] `formula/opcode.rs`
- [ ] `formula/program.rs`
- [ ] `formula/runtime.rs`
- [ ] `formula/builtins.rs`
- [ ] `formula/generator.rs`
- [ ] `FormulaGenerator: Generator`
- [ ] built-in 6 preset
- [ ] tests

### z-audio-synth

- [ ] `formula_voice.rs`
- [ ] `formula_synth.rs`
- [ ] MIDI note on/off
- [ ] polyphony

### z-audio-dsp-plugin

- [ ] `z-audio-formula-plugin`
- [ ] params
- [ ] basic editor
- [ ] native VST3/CLAP export

完了条件:

- MIDI keyboard で鳴る
- program selector と macro knobs が動く
- DAW で scan できる

## Milestone 2: Compressor / Limiter core

### Compressor

- [ ] `dynamics/ballistics.rs`
- [ ] `dynamics/detector.rs`
- [ ] `dynamics/compressor.rs`
- [ ] hard knee
- [ ] attack/release
- [ ] makeup/mix
- [ ] tests

### Limiter

- [ ] `dynamics/limiter.rs`
- [ ] lookahead delay
- [ ] release smoothing
- [ ] ceiling
- [ ] tests

完了条件:

- compressor の threshold/ratio が効く
- limiter が ceiling を超えない
- unit tests が通る

## Milestone 3: Dynamics plugins

### z-audio-dsp-plugin

- [ ] `z-audio-compressor-plugin`
- [ ] `z-audio-limiter-plugin`
- [ ] nih-plug params
- [ ] editor
- [ ] meter atomics
- [ ] xtask bundle 対応

完了条件:

- REAPER/Bitwig で読み込める
- gain reduction meter が動く
- automation できる

## Milestone 4: Parametric reverb core

### z-audio-dsp

- [ ] `delay/delay_line.rs`
- [ ] `delay/allpass.rs`
- [ ] `reverb/early_reflections.rs`
- [ ] `reverb/fdn.rs`
- [ ] `reverb/parametric_reverb.rs`
- [ ] damping filter
- [ ] mix/width/tone
- [ ] tests

完了条件:

- impulse response が出る
- decay/room size/damping が効く
- reset で tail が消える

## Milestone 5: Reverb plugin

### z-audio-dsp-plugin

- [ ] `z-audio-reverb-plugin`
- [ ] params
- [ ] editor
- [ ] mono/stereo layout
- [ ] xtask bundle

完了条件:

- DAW で reverb として使える
- mix 0/100 が期待通り
- automation しても破綻しない

## Milestone 6: Formula Piano core

### z-audio-dsp

- [ ] `resonators/biquad_resonator.rs`
- [ ] `resonators/modal_bank.rs`
- [ ] `excitation/hammer.rs`
- [ ] `resonators/body.rs`

### z-audio-synth

- [ ] `piano/voice.rs`
- [ ] `piano/model.rs`
- [ ] `piano/voice_manager.rs`
- [ ] sustain pedal
- [ ] velocity mapping

完了条件:

- 88鍵で鳴る
- velocity が効く
- release が自然
- CPU が許容範囲

## Milestone 7: Piano plugin

### z-audio-dsp-plugin

- [ ] `z-audio-piano-plugin`
- [ ] MIDI input
- [ ] piano params
- [ ] editor
- [ ] internal safety limiter optional

完了条件:

- DAW で instrument として鳴る
- preset 保存できる
- sustain pedal が効く

## Milestone 8: WebCLAP

- [ ] compressor WebCLAP
- [ ] limiter WebCLAP
- [ ] reverb WebCLAP
- [ ] formula synth WebCLAP
- [ ] piano WebCLAP
- [ ] demo page 更新

完了条件:

- browser host でロードできる
- audio callback が安定
- parameter change が反映

## 推奨 PR サイズ

1 PR は 500〜1500 lines 程度を目安にする。
大きくなる場合は DSP core と plugin wrapper を分ける。

## リスク

### Formula

- aliasing
- expression runtime の安全性
- UI thread からの program swap

### Piano

- 音作りの tuning が重い
- CPU が増えやすい
- voice stealing が難しい

### Reverb

- feedback 暴走
- metallic ringing
- parameter smoothing 不足

### Dynamics

- limiter latency compensation
- compressor meter/UI sync
- true peak 対応の複雑化
