# 05. Parameters / Automation / Presets

## Parameter identity

DSP側の `ParamId`（`z_audio_dsp::params::ParamId`、`z_audio_synth`からも re-export）を
そのままplugin側のparameter IDとして使います。手書きで再定義せず、
`ParamId::ALL`（27要素）と`ParamId::metadata()`をループしてplugin paramを生成します。

```rust
#[repr(u32)]
pub enum ParamId {
    // --- Synth (0-9) ---
    MasterGain = 0,
    MaxPolyphony = 1,
    GeneratorKind = 2,

    // --- Generator (10-19) ---
    GeneratorGain = 10,
    GeneratorPulseWidth = 11,
    GeneratorPhaseOffset = 12,
    GeneratorPan = 13,

    // --- Amp envelope (20-29) ---
    EnvAttack = 20,
    EnvDecay = 21,
    EnvSustain = 22,
    EnvRelease = 23,
    EnvCurve = 24,

    // --- LFO (30-39) ---
    LfoEnabled = 30,
    LfoWaveform = 31,
    LfoRateHz = 32,
    LfoAmount = 33,
    LfoTarget = 34,
    LfoRetrigger = 35,

    // --- 3-band EQ (40-59) ---
    EqLowEnabled = 40,
    EqLowFreq = 41,
    EqLowType = 42,
    EqMidEnabled = 43,
    EqMidFreq = 44,
    EqMidType = 45,
    EqHighEnabled = 46,
    EqHighFreq = 47,
    EqHighType = 48,
}
```

グループ間に隙間(10刻み)があるのは将来の追加用。`ParamId::ALL`の順序は上記宣言順。

## ParamMetadata = single source of truth

```rust
pub enum ParamUnit { Linear, Hertz, Seconds, Boolean, Enum }

pub struct ParamMetadata {
    pub id: ParamId,
    pub name: &'static str,   // "master_gain", "eq_low_freq", ... (snake_case)
    pub unit: ParamUnit,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step_count: Option<u32>, // Enum/Boolean: Some(n), 連続値: None
}
```

NIH-plug側のparam構築方針:

- `ParamUnit::Linear` / `Hertz` / `Seconds` -> `FloatParam::new(name, default, FloatRange::Linear { min, max })`
- `ParamUnit::Boolean` -> `BoolParam::new(name, default >= 0.5)`
- `ParamUnit::Enum` -> `EnumParam` または `IntParam`（`0..=step_count-1`）。
  `from_param_value`/`to_param_value`で`f32`値と相互変換。

手書きのparam一覧やmapping tableをDSP側と二重管理しないこと
（02_vst3_clap_nih_plug_plan.mdのParameters節は本ファイルの要約）。

## Normalized <-> plain mapping

ホストは0..1 normalized値、`SimpleSynth::set_param`/`param_value`は
metadataと同じ単位の"plain"値(f32)を使います。連続値の変換は
`metadata().min..=max`からの線形変換でよく、専用traitは不要です。

```rust
fn normalized_to_plain(m: ParamMetadata, normalized: f32) -> f32 {
    m.min + normalized * (m.max - m.min)
}

fn plain_to_normalized(m: ParamMetadata, plain: f32) -> f32 {
    (plain - m.min) / (m.max - m.min)
}
```

`Enum`/`Boolean`はNIH-plugの`EnumParam`/`BoolParam`/`IntParam`が
normalized<->plainを内部で処理するため、上記は連続値のみに使います。

## Frequency parameters (Hertz)

帯域ごとにrange/defaultが異なります（共通の20Hz-20kHzログマッピングではない）。

| ParamId      | range (Hz)     | default |
|--------------|----------------|---------|
| EqLowFreq    | 20 .. 2,000    | 200     |
| EqMidFreq    | 80 .. 8,000    | 1,000   |
| EqHighFreq   | 1,000 .. 20,000| 5,000   |
| LfoRateHz    | 0.01 .. 20     | 5.0     |

レンジは`ParamId::metadata()`から取得するため、ここでハードコードせず
metadata参照にすること。ホストUIでの聴感上の使いやすさのため、
NIH-plugの`FloatRange::Skewed`（log寄り）で表示することは可能ですが、
`SimpleSynth::set_param`へ渡すplain値は上記の線形レンジ・単位(Hz)のままです。

## Time parameters (Seconds)

Envelopeの時間パラメータは線形秒(0.0..=10.0)です。

| ParamId    | range (s)   | default |
|------------|-------------|---------|
| EnvAttack  | 0.0 .. 10.0 | 0.01    |
| EnvDecay   | 0.0 .. 10.0 | 0.1     |
| EnvRelease | 0.0 .. 10.0 | 0.2     |
| EnvSustain | 0.0 .. 1.0  | 0.7 (Linear level, not seconds) |

UI表示はSkewed rangeで短時間側を見やすくしてよいですが、
DSPへ渡す値は秒単位の線形値（0..10）であることに注意。

## Enum mapping

| ParamId        | 型              | variants (0..) | default |
|----------------|-----------------|-----------------|---------|
| GeneratorKind  | `GeneratorKind` | Sine, Triangle, Saw, Pulse, Noise | Sine |
| EnvCurve       | `EnvelopeCurve` | Linear, Exponential | Exponential |
| LfoWaveform    | `LfoWaveform`   | Sine, Triangle, SawUp, SawDown, Square, RandomHold | Sine |
| LfoTarget      | `LfoTarget`     | None, Gain, PitchSemitone, EqLowFreq, EqMidFreq, EqHighFreq | None |
| Eq{Low,Mid,High}Type | `ButterworthKind` | LowPass, BandPass, HighPass | Low=LowPass, Mid=BandPass, High=HighPass |

各enumは`from_param_value(f32) -> Self`（最近接丸め+clamp）と
`to_param_value(self) -> f32`を持つため、NIH-plugの`EnumParam`の
`index <-> value`変換にそのまま使えます。

## Boolean mapping

| ParamId       | default |
|---------------|---------|
| LfoEnabled    | true (1.0) |
| LfoRetrigger  | true (1.0) |
| EqLowEnabled  | true (1.0) |
| EqMidEnabled  | true (1.0) |
| EqHighEnabled | true (1.0) |

`SimpleSynth::set_param`は`value >= 0.5`で`true`、`param_value`は`0.0`/`1.0`を返します。

## MaxPolyphony (special case)

`ParamId::MaxPolyphony`は`set_param`では**無視**されます
(`SimpleSynth::set_param`内で no-op)。実際の値は`SimpleSynthConfig::max_polyphony`として
`SimpleSynth::new()`時にのみ確定し、voice poolサイズは固定されます。

方針:

- v1: ホストに自動化可能paramとして公開しない。固定値（例: 16）を使う。
- 将来: 変更時に`SimpleSynth`を再構築するnon-automatable設定として扱う。

## Automation

```text
Host automation
  -> EventKind::Param { id, value } with sample_offset
  -> process_with_context()内でsample_offset順にdispatch
  -> SimpleSynth::set_param(id, value)
       - Linear/Hertz/Seconds: metadata().min..=maxへclamp
       - Enum: from_param_value (round + clamp)
       - Boolean: value >= 0.5
  -> EQ周波数/master gainはDSP内部で smoothing済み
       (FREQ_SMOOTHING_TAU=0.02s, gain SMOOTHING_TAU=0.005s)
```

Envelope/Generator/LFOのshapeに関わるパラメータ変更は、
**次にtriggerされるnote**から反映されます（鳴っているvoiceは
trigger時の設定を保持）。EQとmaster gainは即時（smoothed）に反映されます。

## Preset/state

> **Note (2026-06-14)**: nih-plugベースのv1実装（`crates/z-audio-plugin`）では、
> `#[derive(Params)]`を付けた`ZAudioSimpleSynthParams`の全フィールドがhost側で
> 自動的にJSON永続化されるため、以下のカスタム`PluginState`構造体は**不要**です。
> （`MaxPolyphony`はParamsに含めないため、stateにも含まれず常に固定値16）。
> 以下は将来のWeb専用state機構（`crates/z-audio-webclap`等、nih-plugを介さない経路）を
> 検討する際の参考として残します。

```rust
pub struct PluginState {
    pub version: u32,
    pub params: Vec<(u32, f32)>, // (ParamId as u32, plain value)
}
```

- `ParamId`は`#[repr(u32)]`なので`id as u32`で安定したキーになります。
- 保存: `ParamId::ALL`を走査し`(id as u32, synth.param_value(id))`を記録。
- 復元: `for (raw, value) in params { synth.set_param(ParamId::from_u32(raw), value) }`
  （`from_u32`相当のマッピングはplugin側で用意。未知のIDは無視）。
- `MaxPolyphony`もstateに含めてよいが、復元時は値が異なれば
  `SimpleSynth::new()`で再構築が必要（[01_wrapper_architecture.md](01_wrapper_architecture.md)参照）。

serdeを使う場合:

```text
RON or JSON
```

plugin formatによりhost state blobへ保存します。

## Versioning

stateには必ずversionを入れます。

```text
version 1:
  ParamId::ALL (27 params, repr(u32)) + max_polyphony
```

将来Graph IRに移行した場合:

```text
version 2:
  Graph IR + params
```
