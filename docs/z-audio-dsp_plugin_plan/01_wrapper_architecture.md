# 01. Wrapper Architecture

## 重要方針

DSP crate と plugin crate は分離します。

```text
z-audio-dsp      # pure DSP
z-audio-synth    # SimpleSynth runtime
z-audio-plugin   # VST3/CLAP wrapper
z-audio-webclap  # WebCLAP/WASM wrapper
```

Plugin crateはDSPを所有するだけです。

```rust
pub struct ZAudioSimpleSynthPlugin {
    synth: SimpleSynth,
    params: Arc<PluginParams>,
}
```

## Adapter responsibilities

Plugin Adapterが行うこと:

- sample_rate / block_size をDSPに伝える
- host MIDI/event を `InputEvent` に変換
- host automation を `Param` に変換
- audio bufferをDSPに渡す
- parameter metadataをhostに公開
- state save/load

Plugin Adapterがやらないこと:

- DSP計算
- oscillator実装
- filter実装
- voice allocation本体
- asset loading
- GUI

## Parameter bridge

```text
Host Parameter
  -> normalized value
  -> plugin parameter object
  -> z-audio-synth ParamId
  -> smoothed DSP param
```

## Event bridge

実際の `z-audio-synth` にはイベント専用の`process_events()`はありません。
イベントは `ProcessContext::events: &[TimedEvent]` として渡し、
`SimpleSynth::process_with_context(&ctx, left, right)` がブロック内の
sample_offsetでまとめて処理します。

```text
VST3/CLAP event
  -> TimedEvent { sample_offset, kind: EventKind }
  -> events.sort_by_key(|e| e.sample_offset)  // 必須: sample_offset昇順
  -> ProcessContext::new(sample_rate, block_size, tempo_bpm, &events)
  -> SimpleSynth::process_with_context(&ctx, left, right)
```

```rust
pub enum EventKind {
    NoteOn  { note: u8, velocity: f32 },
    NoteOff { note: u8, velocity: f32 },
    Param   { id: ParamId, value: f32 },
}
```

- `NoteOn`/`NoteOff` -> `synth.note_on(note, velocity)` / `synth.note_off(note)`
- `Param` -> `synth.set_param(id, value)`（`process_with_context`内で自動dispatchされる）
- 全`sample_offset`は`< block_size`である必要があります。

## Process lifecycle

`SimpleSynth::new(config)` は内部で`prepare()`まで完了した状態を返すため、
host activate時に別途`prepare()`を呼ぶAPIはありません。
sample_rate / max_block_size / max_polyphony が変わる場合は
`SimpleSynth`を**再構築**します。

```text
Plugin init
  -> create params (ParamId::ALL + metadata)
  -> SimpleSynth は未構築 (host activateまで保留)、
     またはデフォルトconfigで仮構築

Host activates (sample_rate, max_block_size確定)
  -> synth = SimpleSynth::new(SimpleSynthConfig {
       sample_rate,
       max_block_size,
       max_polyphony, // 固定値 or プラグイン設定
     })
  -> 現在のparam値をsynth.set_param(id, value)で復元

Process block
  -> host event/automationをTimedEventへ変換 (sample_offset昇順)
  -> ProcessContext構築
  -> synth.process_with_context(&ctx, left, right)
```

## No allocation during process

Plugin adapterもDSPと同じく、process中のallocationを避けます。

必要な一時buffer:

```rust
event_buffer: Vec<TimedEvent>
left: temporary borrowed host buffer
right: temporary borrowed host buffer
```

`event_buffer` は `prepare()` 時に容量確保します。
