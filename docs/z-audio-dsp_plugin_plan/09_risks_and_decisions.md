# 09. Risks and Decisions

## Decision 1: DSP and plugin are separate

決定:

```text
DSPはcargo libraryとして独立
Pluginはadapter
```

理由:

- ゲームランタイムでも使える
- Webでも使える
- テストしやすい
- plugin format変更に強い

## Decision 2: Public term is Generator

決定:

```text
OscillatorではなくGenerator
```

理由:

- Phase Plant準拠
- noise/sample/audio input/mathを含めやすい
- 将来のgranular/additive/wavetableに自然

## Decision 3: First VST3/CLAP path is NIH-plug

決定:

```text
VST3/CLAPはまずNIH-plugで作る
```

理由:

- 初速が速い
- Rustで書きやすい
- GUIなしなら十分

リスク:

- 本家がmaintenance mode
- 長期的にはforkまたはnative CLAP adapterが必要かもしれない

## Decision 4: WebCLAP is staged

決定:

```text
いきなりWebCLAPではなく、まずAudioWorklet/WASMでDSP確認
```

理由:

- WebCLAP周辺はまだ実験的
- AudioWorklet上でDSPが安定することが先
- ブラウザのblock sizeやmessage bridgeを先に確認したい

## Decision 5: ParamId::metadata() を単一の正とする

決定:

```text
parameter名/range/default/enum段数は z_audio_dsp::ParamId::metadata() から取得し、
plugin側で手書きの定義表を持たない
```

理由:

- DSP本家のparamsとplugin側定義のズレ（今回見つかったような乖離）を防ぐ
- ParamId::ALL (27種) をループしてNIH-plug Paramsを生成できる
- 範囲/defaultの変更がDSP側で起きてもplugin側は追従するだけでよい

## Risk: VST3/CLAP/WebCLAPの共通化

同じDSPを使えても、host event/parameter/stateの仕様が違うため完全共通化はできません。

対策:

```text
DSP core
  <- common ParamId/Event
Plugin adapter
  <- format-specific mapping
```

## Risk: GUIなしでもパラメータが多い

第一弾はgeneric editorを使うため、パラメータ名とgroupingが重要です。

対策:

```text
Generator/*
Envelope/*
LFO/*
EQ/Low/*
EQ/Mid/*
EQ/High/*
Master/*
```

## Risk: EQ仕様の曖昧さ — 解決済み (2026-06-14確認)

`lowpass, bandpass, lowpass` と指定されていましたが、EQとしては highpass が自然という
懸念は、z-audio-dsp本家の`ThreeBandButterworthEq::new()`実装で解決済みです。

```text
Bandごとに ButterworthKind (LowPass/BandPass/HighPass) を持つ
Default: Low=LowPass / Mid=BandPass / High=HighPass
```

## Risk: SimpleSynthの再構築コスト

`SimpleSynth`はsample_rate/max_block_size/max_polyphonyを変更する
公開API（再prepare）を持たず、`SimpleSynth::new()`での再構築が必要です。

対策:

```text
host activate / sample-rate変更時にのみ再構築
再構築時は現在のparam値をset_paramで復元
processブロック単位での再構築は行わない（real-time safety違反）
```

## Risk: max_polyphonyはconstruction-time専用

`ParamId::MaxPolyphony`は`SimpleSynth::set_param`で無視されるread-onlyな
automation IDで、実際のvoice pool sizeは`SimpleSynthConfig::max_polyphony`として
`new()`時に固定されます。

対策:

```text
v1: ホストにautomatable paramとして公開しない。固定値（例: 16）。
将来: 変更時はSimpleSynth再構築（上記リスクと同じ制約）。
```

## Risk: Audio thread allocation

Plugin wrapper側でevent Vecをprocess中に伸ばすと危険。

対策:

- prepare時にevent buffer reserve
- max events per blockを決める
- overflow時は安全にdrop/logなし

## Risk: Host differences

DAWごとにVST3/CLAP実装の差があります。

対策:

- REAPERを基準host
- BitwigでCLAP確認
- AbletonでVST3確認
- host testing matrixを維持
