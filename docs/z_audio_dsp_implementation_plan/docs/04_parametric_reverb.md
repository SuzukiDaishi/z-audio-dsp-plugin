# 04. パラメトリックリバーブ実装計画

## ゴール

`z-audio-dsp` に stereo parametric reverb を追加し、`z-audio-dsp-plugin` に native VST3/CLAP と WebCLAP plugin を作る。

対象は convolution reverb ではなく、パラメータで操作できる algorithmic reverb。

## 推奨アルゴリズム

MVP は以下。

```text
Input L/R
  -> Input Mix / Mono Sum
  -> Predelay
  -> Early Reflections
  -> Diffusion Allpass Chain
  -> 8-line FDN Late Reverb
  -> Damping / Tone
  -> Wet Gain
  -> Dry/Wet Mix
Output L/R
```

## なぜ FDN か

- パラメータ化しやすい
- stereo width を作りやすい
- delay line と feedback matrix だけで実装できる
- WebAssembly でも比較的軽い
- 将来 16-line / modulated FDN に拡張できる

## module layout

```text
crates/z-audio-dsp/src/reverb/
  mod.rs
  parametric_reverb.rs
  early_reflections.rs
  fdn.rs
  diffusion.rs
  damping.rs

crates/z-audio-dsp/src/delay/
  delay_line.rs
  allpass.rs
  modulated_delay.rs
```

## public API

```rust
pub struct ParametricReverb {
    sample_rate: f32,
    predelay: StereoDelay,
    early: EarlyReflections,
    diffuser: DiffusionChain,
    fdn: FdnReverb,
    params: ParametricReverbParams,
    smoothed: SmoothedReverbParams,
}

pub struct ParametricReverbParams {
    pub mix: f32,
    pub room_size: f32,
    pub decay_time_sec: f32,
    pub pre_delay_ms: f32,
    pub diffusion: f32,
    pub damping: f32,
    pub low_cut_hz: f32,
    pub high_cut_hz: f32,
    pub modulation_rate_hz: f32,
    pub modulation_depth: f32,
    pub width: f32,
    pub early_late_mix: f32,
}
```

`ParametricReverb` は `Effect` trait を実装する。

```rust
impl Effect for ParametricReverb {
    fn prepare(&mut self, sample_rate: f32, max_block_size: usize) { ... }
    fn reset(&mut self) { ... }
    fn process_stereo(&mut self, ctx: &ProcessContext, left: &mut [f32], right: &mut [f32]) { ... }
}
```

## Delay line

reverb の土台。

```rust
pub struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl DelayLine {
    pub fn prepare(&mut self, max_delay_samples: usize);
    pub fn clear(&mut self);
    pub fn push(&mut self, x: f32);
    pub fn read_int(&self, delay_samples: usize) -> f32;
    pub fn read_frac_lerp(&self, delay_samples: f32) -> f32;
}
```

`prepare()` でだけ `Vec` 確保する。
`process_stereo()` 内で resize しない。

## Early reflections

MVP は固定 tap delay。

```rust
pub struct EarlyTap {
    pub delay_ms_l: f32,
    pub delay_ms_r: f32,
    pub gain_l: f32,
    pub gain_r: f32,
}
```

room_size によって delay を scale する。

```text
delay_ms = base_delay_ms * lerp(0.5, 2.5, room_size)
```

## Diffusion

Schroeder allpass chain。

```text
y[n] = -g*x[n] + d[n]
d[n] = x[n] + g*y[n]
```

左右で異なる delay length を使う。

MVP:

- 4 allpass per channel
- gain = 0.5〜0.75
- diffusion parameter で gain と stage count 相当を調整

## FDN late reverb

8-line FDN。

```text
v = delay_outputs
v = damping_filters(v)
v = feedback_matrix(v)
delay_inputs = input_injection + feedback_gain * v
```

feedback matrix:

- Hadamard 8x8
- Householder

MVP は Hadamard が簡単。

## decay time mapping

target decay time T60 から feedback gain を求める。

```text
g_i = 10^(-3 * delay_seconds_i / decay_time_sec)
```

各 delay line の length が違うため、line ごとに gain を変える。

## damping

高域 damping は feedback loop 内に lowpass を入れる。

```rust
struct OnePoleLowpass {
    z: f32,
    a: f32,
}
```

cutoff mapping:

```text
cutoff = lerp(20000 Hz, 2000 Hz, damping)
```

低域の濁り対策として feedback 内または output に highpass を入れる。

## modulation

metallic ringing を減らすため、delay length をゆっくり揺らす。

MVP では optional。

```text
delay = base_delay + sin(lfo_phase + phase_offset) * modulation_depth_samples
```

注意:

- interpolation が必要
- modulation depth を大きくしすぎると chorus になる
- feedback 内の fractional delay は CPU が増える

## parameters

### Reverb plugin parameters

- Mix: 0..100 %
- Room Size: 0..1
- Decay: 0.1..20 sec
- Pre Delay: 0..250 ms
- Diffusion: 0..1
- Damping: 0..1
- Low Cut: 20..1000 Hz
- High Cut: 1000..20000 Hz
- Mod Rate: 0..2 Hz
- Mod Depth: 0..1
- Width: 0..1
- Early/Late: 0..1
- Output Gain: -24..24 dB

## safety

feedback 系は暴走しやすいので以下を入れる。

- feedback gain clamp: `0.0..0.9995`
- input soft clip optional
- output soft clip optional
- denormal flush
- NaN guard in debug test
- decay time min clamp

## plugin wrapper

`z-audio-reverb-plugin` を追加。

```text
Plugin type: AudioEffect
Inputs: stereo / mono
Outputs: stereo
Category: Reverb
```

mono input の場合:

- mono を L/R にコピー
- stereo output を返す

## WebCLAP

WebCLAP では CPU と memory に注意。
MVP は 8-line FDN のみ。
16-line, oversampling なし。

## テスト

### Unit tests

- silence input -> silence-ish output after reset
- impulse response length is non-zero
- decay parameter increases tail length
- mix 0 = dry
- mix 1 = wet only
- no NaN/Inf
- reset clears tail

### Audio quality tests

- impulse response wav 書き出し
- pink noise through reverb
- snare-like transient
- piano chord

### Performance tests

- 44.1kHz block 64/128/512
- 48kHz block 128
- 96kHz block 128
- wasm build smoke test

## 実装順

1. `DelayLine`
2. `Allpass`
3. `EarlyReflections`
4. `FdnReverb` no modulation
5. `ParametricReverb` wrapper
6. Unit tests
7. native plugin
8. modulation
9. WebCLAP
