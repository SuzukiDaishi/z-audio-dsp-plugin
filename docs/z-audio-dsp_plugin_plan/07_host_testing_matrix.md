# 07. Host Testing Matrix

## Native DAW hosts

### Windows

- REAPER
  - VST3
  - CLAP
- Bitwig Studio
  - CLAP
  - VST3
- Ableton Live
  - VST3

### macOS

- REAPER
  - VST3
  - CLAP
- Bitwig Studio
  - CLAP
  - VST3
- Logic Pro
  - AUは第一弾対象外
  - 参考確認のみ

### Linux

- REAPER
  - VST3
  - CLAP
- Bitwig Studio
  - CLAP
  - VST3

## Web

- Chrome
- Edge
- Firefox
- Safariは後回し

## Test cases

### Load/unload

- plugin scanでクラッシュしない
- insert/removeでクラッシュしない
- DAW終了時にクラッシュしない

### Audio

- note onで発音
- note offでrelease
- chord発音
- rapid note on/off
- silence状態でCPUが暴れない

### Parameters

- generator切替
- envelope変更
- LFO rate変更
- EQ band on/off
- EQ frequency sweep
- master gain

### Automation

- EQ frequency automation
- LFO amount automation
- master gain automation

### State

- save project
- reload project
- parameter復元
- preset保存/読み込み

### Web

- AudioWorklet load
- wasm load
- note trigger
- parameter message
- continuous playback 5min
