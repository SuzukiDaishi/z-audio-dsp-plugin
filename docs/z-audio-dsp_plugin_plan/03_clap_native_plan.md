# 03. Native CLAP Adapter Plan

## なぜnative CLAPも検討するか

第一弾はNIH-plugでよいですが、長期的にはnative CLAP adapterを持つ価値があります。

理由:

- CLAPはstable ABIを重視する設計
- 拡張機能がC interfaceとして定義される
- Phase Plant風の将来機能、特にmodulation/parameter周りと相性がよい
- NIH-plugのmaintenance状態に依存しすぎない

## 位置づけ

native CLAP adapterは第一弾では実装しません。  
ただし、設計だけ先に決めます。

```text
z-audio-dsp/z-audio-synth
  -> z-audio-clap-adapter
  -> .clap
```

## 実装方針

Rustで直接CLAP ABIを扱う場合は、以下のいずれか。

1. `clap-sys` 等のbindingを使う
2. C wrapperを薄く書いてRust FFIへ渡す
3. 既存のRust CLAP crateを調査して選定

## Adapter responsibilities

- entry point
- plugin factory
- plugin descriptor
- activate/deactivate
- process
- params extension
- state extension
- note ports/event handling
- audio ports

## 初期対応するCLAP extensions

第一弾native化するなら最低限:

- params
- state
- audio-ports
- note-ports
- log
- thread-check

将来的:

- latency
- tail
- gui
- remote controls
- preset discovery
- note expression / modulation extensions

## Process callback

```text
clap_process
  -> input events scan
  -> output buffer mapping
  -> z_audio_synth::SimpleSynth::process()
  -> return status
```

## Risks

- ABIミスでhostがクラッシュしやすい
- VST3より資料は読みやすいが、実装検証に時間がかかる
- Windows/macOS/Linuxのbundle/install差異
- hostごとの拡張対応差

## Decision

第一弾:

```text
NIH-plug経由CLAP
```

第二弾以降:

```text
native CLAP adapter prototype
```
