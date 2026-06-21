# Formula Piano 現状実装メモ

外部の音響/DSP担当者に相談するための、現状のピアノ実装まとめです。

現状の音は「ピアノにかなり遠い」という評価です。この文書は、現在の信号経路、音源モデル、パラメータ、既知の弱点、相談したい論点を共有するためのものです。

## 対象ファイル

- DSP/synth core:
  - `thirdparty/z-audio-dsp/crates/z-audio-synth/src/piano/synth.rs`
  - `thirdparty/z-audio-dsp/crates/z-audio-synth/src/piano/voice.rs`
  - `thirdparty/z-audio-dsp/crates/z-audio-synth/src/piano/params.rs`
- excitation/resonator:
  - `thirdparty/z-audio-dsp/crates/z-audio-dsp/src/excitation/hammer.rs`
  - `thirdparty/z-audio-dsp/crates/z-audio-dsp/src/resonators/modal_bank.rs`
  - `thirdparty/z-audio-dsp/crates/z-audio-dsp/src/resonators/biquad_resonator.rs`
  - `thirdparty/z-audio-dsp/crates/z-audio-dsp/src/resonators/body.rs`
- plugin wrapper:
  - `crates/z-audio-piano-plugin/src/lib.rs`

## 全体の信号経路

```text
MIDI NoteOn/NoteOff
  -> ZAudioFormulaPiano plugin wrapper
  -> FormulaPiano
  -> PianoVoice x max 32
  -> HammerExciter short pulse
  -> ModalBank<64> strings
  -> per-note pan
  -> summed stereo voices
  -> BodyResonator static soundboard/body modes
  -> master gain
  -> stereo output
```

サンプル再生は使っていません。完全に数式/モーダル合成で作っています。

## Plugin wrapper

Native VST3/CLAP plugin は `ZAudioFormulaPiano` です。

- 最大同時発音数: 32
- MIDI input: basic MIDI note input
- audio input: none
- audio output: stereo
- sample accurate automation: false
- note event と param change を `TimedEvent` に変換し、`FormulaPiano::process_with_context()` に渡します。

plugin が公開しているパラメータは 12 個です。

| ParamId | plugin label | range | default |
| --- | --- | ---: | ---: |
| `PianoTone` | Tone | 0.0..1.0 | 0.5 |
| `PianoBrightness` | Brightness | 0.0..1.0 | 0.55 |
| `PianoHammerHardness` | Hammer | 0.0..1.0 | 0.55 |
| `PianoHammerNoise` | Hammer Noise | 0.0..1.0 | 0.08 |
| `PianoInharmonicity` | Inharmonicity | 0.0..1.0 | 0.45 |
| `PianoDecay` | Decay | 0.2..8.0 s | 2.4 |
| `PianoRelease` | Release | 0.05..5.0 s | 0.8 |
| `PianoBodyAmount` | Body | 0.0..1.0 | 0.35 |
| `PianoStereoWidth` | Width | 0.0..1.0 | 0.75 |
| `PianoSympatheticAmount` | Sympathetic | 0.0..1.0 | 0.0 |
| `PianoPedalResonance` | Pedal | 0.0..1.0 | 0.25 |
| `PianoMasterGain` | Master Gain | -24.0..12.0 dB | -6.0 |

## Polyphonic piano engine

`FormulaPiano` は `Vec<PianoVoice>` と `BodyResonator` を持ちます。

処理の概要:

1. block 内の `TimedEvent` を sample offset 順に処理。
2. 各 sample で全 voice の `next_sample()` を合算。
3. 合算した mid 信号を `BodyResonator` に送る。
4. direct voice signal と body output を `body_send` で crossfade。
5. `PianoMasterGain` を dB から linear にして掛ける。

現在の body send:

```text
body_send = clamp(
  body_amount + sympathetic_amount * 0.16 + pedal_resonance * 0.10,
  0.0,
  1.0
)

body_input = (left + right) * 0.5 * (1.0 + pedal_resonance * 0.18)
```

voice stealing は、空き voice 優先、次に releasing voice の古いもの、最後に active voice の古いものを使います。

## Voice model

1 voice は以下を持ちます。

- `HammerExciter`
- `ModalBank<64>`
- note/velocity
- pan
- release envelope scalar

NoteOn 時に、その note 用の modal mode 配列を作り直します。NoteOff 後は `release_gain` を指数減衰させます。

Release 係数:

```text
effective_release = max(release_sec, 0.05) * (1.0 + pedal_resonance * 3.5)
release_coeff = exp(-1.0 / (effective_release * sample_rate))
```

voice は次の条件で idle になります。

```text
hammer_done
AND (
  modal_energy * release_gain < 1.0e-4
  OR (releasing AND release_gain < 2.0e-5)
)
```

## Hammer excitation

`HammerExciter` は物理的な hammer-string contact ではなく、短い force pulse です。

Trigger 時:

```text
duration_ms = 2.9 - 2.1 * hardness + 1.2 * (1.0 - velocity)
duration_samples = clamp(round(duration_ms * sample_rate / 1000), 8, 256)
```

Sample ごと:

```text
x = time / (duration_samples - 1)
force = (0.5 - 0.5 * cos(TAU * x)) ^ (0.65 + hardness * 0.7)
contact_noise = xorshift_noise * noise_amount * (1.0 - x)^2 * (0.2 + hardness * 0.8)
felt_snap = if x < 0.16 {
  (1.0 - x / 0.16)^2 * hardness * 0.22
} else {
  0.0
}

out = (force * (0.62 + hardness * 0.55) + contact_noise + felt_snap)
  * velocity^0.72
  * (0.35 + 0.95 * velocity)
```

重要: note frequency は `trigger()` に渡していますが、現在は使っていません。

## String modal model

`PianoVoice::build_modes()` で最大 64 個の `ModalMode` を生成します。

各 `ModalMode`:

```rust
struct ModalMode {
    frequency_hz: f32,
    gain: f32,
    decay_sec: f32,
}
```

レゾネータは 2-pole resonator です。

```text
r = exp(-1.0 / (decay_sec * sample_rate))
theta = TAU * frequency_hz / sample_rate
coeff = 2.0 * r * cos(theta)
r2 = r * r

y[n] = input * gain + coeff * y[n-1] - r2 * y[n-2]
```

現状、この resonator は振幅正規化されていません。`gain` は手動で小さくしてあります。

### Harmonic count / string count

```text
string_count:
  note < 45       -> 1
  45 <= note < 58 -> 2
  note >= 58      -> 3

max_harmonic:
  note < 40       -> 24
  40 <= note < 72 -> 18
  note >= 72      -> 12
```

### Inharmonicity

```text
note_pos = note / 127
base_b = (0.000035 + (1.0 - note_pos) * 0.00009 + note_pos^2 * 0.00016)
  * (0.35 + inharmonicity * 1.15)

mode_freq = fundamental * n * sqrt(1.0 + base_b * n^2)
```

### Strike position and spectrum

```text
strike = clamp(0.118 + (1.0 - tone) * 0.035 - note_pos * 0.018, 0.085, 0.16)
strike_gain = abs(sin(PI * n * strike))

brightness = 0.45 + brightness_param * 1.55
hammer_cutoff = 7.5 + brightness * 5.5 + velocity * 9.0 + hammer_hardness * 7.0
high_rolloff = exp(-(n / hammer_cutoff)^1.55)
```

### Gain shaping

```text
harmonic_shape:
  n = 1     -> 0.38
  n = 2     -> 1.55
  n = 3     -> 1.45
  n = 4..6  -> 0.98
  otherwise -> 0.82

gain =
  velocity^0.78
  * strike_gain^0.72
  * high_rolloff
  * low_register_weight
  * harmonic_shape
  * (1.0 + hammer_hardness * min(n / 18.0, 1.0) * 0.55)
  / n^(0.74 + (1.0 - brightness_param) * 0.26)
```

実際に各 string mode に入る gain:

```text
mode_gain = gain * string_balance * 0.0065
```

### Decay shaping

```text
register_decay = decay_param * (1.55 - note_pos * 0.92)
sustain_boost = 1.0 + pedal_resonance * 0.95 + sympathetic_amount * 0.45

slow_decay =
  (register_decay * sustain_boost)
  / n^(0.20 + brightness_param * 0.18)
  * (1.0 + (1.0 - note_pos) * 0.25)

fast_decay = (0.055 + 0.42 / sqrt(n)) * (1.0 + pedal_resonance * 0.25)
```

### Multi-string detune

```text
spread_cents =
  (0.7 + harmonic * 0.055)
  * (1.0 + max(key_center, 0.0) * 0.9)
  * (0.75 + sympathetic_amount * 0.65)

1 string: [0.0]
2 string: [-spread, spread * 0.86]
3 string: [-spread * 1.08, 0.0, spread * 0.92]
```

String balance:

```text
1 string: [1.0]
2 string: [0.52, 0.48]
3 string: [0.34, 0.32, 0.34]
```

### Fast transient companion mode

各 harmonic の string modes とは別に、`harmonic <= 10` の場合だけ短い decay の transient mode を追加しています。

```text
transient_freq = frequency * cents_to_ratio(0.35 * key_center * sqrt(n))
transient_gain = gain * (0.005 + sympathetic_amount * 0.0025) * velocity_brightness
transient_decay = fast_decay
```

## Body / soundboard model

`BodyResonator` は固定 7 mode の stereo resonator です。ピアノ響板の物理モデルではなく、軽量な body coloration です。

Current modes:

| Frequency | Gain | Decay |
| ---: | ---: | ---: |
| 96 Hz | 0.0055 | 0.62 s |
| 142 Hz | 0.0060 | 0.72 s |
| 248 Hz | 0.0052 | 0.58 s |
| 430 Hz | 0.0040 | 0.46 s |
| 770 Hz | 0.0028 | 0.34 s |
| 1480 Hz | 0.0018 | 0.22 s |
| 3100 Hz | 0.0012 | 0.14 s |

Right channel は各 mode の frequency を少し上げ、decay を 0.93 倍しています。

```text
right_freq = left_freq * (1.0 + 0.007 * (index + 1))
right_decay = left_decay * 0.93
```

body output は direct signal と crossfade されています。

## 現在のテスト

`thirdparty/z-audio-dsp/crates/z-audio-synth/src/piano/synth.rs` に以下の数値テストがあります。

- note on で有限かつ非ゼロの出力がある
- velocity が大きい方が RMS が大きい
- note off 後に voice が idle になる
- low/mid/high note が finite
- pedal resonance で release tail RMS が伸びる
- sympathetic amount を変えると出力が変わる
- A2/A4/A5 相当で fundamental と一部 partial が存在し、高域 partial が過剰ではない

重要: これらは「壊れていない」ことを確認するテストであり、「ピアノらしい音」かは検証していません。

## 現状でピアノらしくない可能性が高い点

1. Hammer model が hammer-string の非線形接触ではない
   - short pulse を全 modes に同じ入力として入れているだけです。
   - hammer mass/stiffness/felt compression/contact duration の周波数依存がありません。
   - note frequency を hammer に渡しているものの未使用です。

2. Mode gain/decay が実測や物理式ではなく手動カーブ
   - `harmonic_shape` や `0.0065` などは聴感/安全性ベースの係数です。
   - 各音域の実ピアノらしい spectral envelope にフィットしていません。

3. Resonator が振幅正規化されていない
   - decay が長い mode の出力レベルが直感的な gain と一致しません。
   - mode gain の意味が物理量として扱いづらいです。

4. String coupling がない
   - 複弦 detune は独立 mode を足しているだけです。
   - 弦同士のエネルギー交換、beating の時間変化、unison coupling はありません。

5. Damper / pedal model が粗い
   - NoteOff は全 mode に同じ release scalar を掛けています。
   - sustain pedal は release/tail/body send を伸ばすだけです。
   - damper が弦や音域によって異なる挙動はありません。

6. Sympathetic resonance が粗い
   - sympathetic amount は mode decay、detune、body send、transient gain を少し変えるだけです。
   - 他の開放弦/押鍵中の弦が共鳴するモデルではありません。

7. Soundboard/body が簡略化されすぎている
   - 固定 7 mode の coloration で、bridge/soundboard/radiation の周波数応答ではありません。
   - actual piano body impulse response や measured response に合わせていません。

8. Stereo image が単純
   - note number から pan を決めるだけです。
   - マイク位置、響板 radiation、低域の中央寄り/高域の広がりなどはありません。

9. Attack transient がピアノらしくない可能性
   - felt snap と noise はありますが、hammer knock、string initial condition、soundboard transient の分離がありません。
   - velocity で hammer spectrum は変わるが、実ピアノの打鍵感に合わせたモデルではありません。

## 外部相談で聞きたいこと

1. この構造を維持する場合、まず直すべき順序はどこか
   - hammer excitation
   - modal frequency/gain/decay
   - soundboard/body
   - damper/pedal/sympathetic resonance

2. 2-pole modal bank のまま、ピアノらしい spectrum に近づける実用的な係数設計はあるか
   - note/key tracking
   - inharmonicity B の範囲
   - harmonic amplitude envelope
   - partial decay envelope
   - velocity dependence

3. 実測サンプルや reference recording から mode gain/decay をフィットする簡易手順は何がよいか
   - 1音ごとの spectral peak extraction
   - partial decay estimation
   - note range interpolation
   - loudness normalization

4. Hammer model をどう置き換えるべきか
   - current short pulse でよいのか
   - hammer-string nonlinear contact を簡易実装すべきか
   - raised cosine/half-sine pulse の duration/gain/filter を note/velocity 依存にするだけで十分か

5. BodyResonator は固定 mode ではなく、どの程度の filter/IR/model にすべきか
   - static EQ/radiation filter
   - convolution IR
   - modal soundboard
   - feedback delay network

6. ペダル/共鳴を、この規模の synth でどこまで実装すべきか
   - global sympathetic modal bank
   - currently held notes only
   - pedal-down all strings approximation
   - damped release only

## 制約

- サンプル再生ではなく、できれば数式/モーダル合成の路線を維持したいです。
- VST3/CLAP/WebCLAP で動かすため、CPU とメモリは軽めにしたいです。
- 既存 API と ParamId はなるべく維持したいです。
- ただし、音がピアノに聞こえないなら内部実装は大きく変えて構いません。

## 相談相手に渡したい結論

現在の実装は「ピアノの主要要素を名前としては持っているが、実測/物理に基づいた piano model ではない」です。特に、hammer excitation、mode gain/decay、soundboard/body、damper/pedal のモデルが全て手作りの近似です。

外部レビューでは、ピアノ音色に効く優先順位と、現実的に導入できる軽量モデルまたは係数フィット手順を提案してもらいたいです。
