# 02. VST3 / CLAP via NIH-plug Plan

## 方針

第一弾の VST3 / CLAP は NIH-plug を使う方針にします。

理由:

- RustでVST3とCLAPの両方を扱いやすい
- GUIなしプラグインの作成が速い
- generic editorでパラメータ確認できる
- DSP検証を優先できる

ただし、NIH-plug 本家は maintenance mode と記載されているため、長期的には fork / alternative / native CLAP adapter を検討します。

## Crate layout

```text
crates/
└── z-audio-plugin/
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── params.rs
        ├── plugin.rs
        └── mapping.rs
```

## Plugin declaration

`SimpleSynth::new(config)`はsample_rate/max_block_size/max_polyphonyを要求するため、
NIH-plugの`Default::default()`時点では構築できません。`Option`で保持し、
`initialize()`でhostのconfigを使って構築します。

```rust
pub struct ZAudioSimpleSynth {
    params: Arc<ZAudioParams>,
    synth: Option<SimpleSynth>,
    events: Vec<TimedEvent>,
}
```

`initialize()`:

```text
synth = Some(SimpleSynth::new(SimpleSynthConfig {
    sample_rate: buffer_config.sample_rate,
    max_block_size: buffer_config.max_buffer_size as usize,
    max_polyphony: FIXED_MAX_POLYPHONY, // e.g. 16
}))
-> for id in ParamId::ALL { synth.set_param(id, params.value(id)) }
```

## Parameters

`z_audio_dsp::ParamId::ALL`（27種）と`ParamId::metadata()`を単一の正とし、
NIH-plugの`Params`はこれをループして自動生成します（名前・範囲・defaultを
手書きで重複管理しない）。詳細は[05_parameters_automation_presets.md](05_parameters_automation_presets.md)。

### Synth (group: `Synth`)

- master_gain (Linear, 0.0..=2.0, default 1.0)
- max_polyphony (Linear, 1.0..=64.0, default 16.0) — **読み取り専用**。
  `SimpleSynth::set_param`はこれを無視する。`SimpleSynthConfig::max_polyphony`として
  construction時にのみ反映されるため、NIH-plugでは自動化不可のplain int paramか、
  v1ではホストparamとして公開しない（固定値16）。
- generator_kind (Enum, 5: Sine/Triangle/Saw/Pulse/Noise)

### Generator (group: `Generator`)

- generator_gain (Linear, 0.0..=2.0, default 1.0)
- generator_pulse_width (Linear, 0.05..=0.95, default 0.5)
- generator_phase_offset (Linear, 0.0..=1.0, default 0.0)
- generator_pan (Linear, -1.0..=1.0, default 0.0)

### Envelope (group: `Envelope`)

- env_attack (Seconds, 0.0..=10.0, default 0.01)
- env_decay (Seconds, 0.0..=10.0, default 0.1)
- env_sustain (Linear, 0.0..=1.0, default 0.7)
- env_release (Seconds, 0.0..=10.0, default 0.2)
- env_curve (Enum, 2: Linear/Exponential, default Exponential)

### LFO (group: `LFO`)

- lfo_enabled (Boolean, default true)
- lfo_waveform (Enum, 6: Sine/Triangle/SawUp/SawDown/Square/RandomHold)
- lfo_rate_hz (Hertz, 0.01..=20.0, default 5.0)
- lfo_amount (Linear, 0.0..=12.0, default 0.0)
- lfo_target (Enum, 6: None/Gain/PitchSemitone/EqLowFreq/EqMidFreq/EqHighFreq)
- lfo_retrigger (Boolean, default true)

### EQ (group: `EQ/Low`, `EQ/Mid`, `EQ/High`)

- eq_low_enabled / eq_low_freq (20..=2000 Hz, default 200) / eq_low_type (default LowPass)
- eq_mid_enabled / eq_mid_freq (80..=8000 Hz, default 1000) / eq_mid_type (default BandPass)
- eq_high_enabled / eq_high_freq (1000..=20000 Hz, default 5000) / eq_high_type (default HighPass)

すべて`ButterworthKind` (Enum, 3: LowPass/BandPass/HighPass)。
帯域ごとにrangeとdefaultが異なる点に注意（共通の20Hz-20kHzではない）。

## MIDI events

NIH-plugのprocess内でnote eventsを受け、DSP側の `TimedEvent` に変換します。

```rust
TimedEvent {
    sample_offset,
    kind: EventKind::NoteOn { note, velocity }
}
```

## Build

想定コマンド:

```bash
cargo xtask bundle z_audio_simple_synth --release
```

またはNIH-plugのtemplateに従う形にします。

## Deliverables

```text
target/bundled/Z Audio Simple Synth.vst3
target/bundled/Z Audio Simple Synth.clap
```

## Acceptance Criteria

- REAPERでVST3を読み込める
- REAPERでCLAPを読み込める
- BitwigでCLAPを読み込める
- DAW generic UIでパラメータ操作できる
- MIDI noteで音が鳴る
- EQ周波数変更が反映される
- preset/state save/loadが動く
