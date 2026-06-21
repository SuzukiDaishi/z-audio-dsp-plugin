# 02. 数式ベースシンセシス core 実装計画

## ゴール

`z-audio-dsp` に、リアルタイム安全に動く「数式ベースの signal generator」を追加する。

ここでいう数式ベースとは、単に `sin()` を書けるという意味ではなく、以下を組み合わせて音色を作る仕組みとする。

- phase based oscillator formula
- FM / PM / PD / wavefold / waveshape
- additive partial formula
- noise / chaotic / recursive-like formula
- macro parameter による morphing
- velocity / note / time dependent shaping

## 最初に避けること

MVP では以下をやらない。

- audio thread 内で文字列 parse
- user-defined function の動的追加
- arbitrary recursion
- heap allocation in process
- JIT
- unsafe SIMD 最適化
- feedback network の無制限接続

## 推奨 API

```rust
pub struct FormulaGenerator {
    sample_rate: f32,
    phase: f32,
    time_samples: u64,
    frequency_hz: f32,
    velocity: f32,
    program: FormulaProgram,
    stack: [f32; FORMULA_STACK_SIZE],
    macros: [f32; FORMULA_MACRO_COUNT],
}

pub struct FormulaParams {
    pub frequency_hz: f32,
    pub velocity: f32,
    pub program_id: FormulaProgramId,
    pub macros: [f32; FORMULA_MACRO_COUNT],
    pub output_gain_db: f32,
}
```

`FormulaGenerator` は既存の `Generator` trait を実装する。

```rust
impl Generator for FormulaGenerator {
    fn prepare(&mut self, sample_rate: f32, max_block_size: usize) { ... }
    fn reset(&mut self) { ... }
    fn process_mono(&mut self, ctx: &ProcessContext, out: &mut [f32]) { ... }
}
```

## Formula VM 方式

最初は AST を直接辿らず、compile 済み opcode を stack VM で評価する。

```rust
pub enum FormulaOpcode {
    Const(f32),
    Phase,
    TimeSec,
    FrequencyHz,
    MidiNote,
    Velocity,
    Macro(u8),

    Add,
    Sub,
    Mul,
    DivSafe,
    Neg,

    Sin2Pi,
    Cos2Pi,
    TanH,
    Abs,
    Sign,
    Floor,
    Fract,
    PowSafe,
    Exp,
    LogSafe,
    SqrtSafe,

    Min,
    Max,
    Clamp01,
    Mix,

    Saw,
    Square,
    Triangle,
    NoiseHash,
    SmoothStep,
    SoftClip,
    WaveFold,
}
```

VM の制約:

- `Vec<FormulaOpcode>` は compile 時に作る。
- `process_mono()` では immutable slice と固定長 stack のみを使う。
- stack overflow は compile 時に検出する。
- runtime では debug assert 程度に留める。

## FormulaProgram

```rust
pub struct FormulaProgram {
    pub id: FormulaProgramId,
    pub name: &'static str,
    pub opcodes: &'static [FormulaOpcode],
    pub output_scale: f32,
    pub dc_block: bool,
    pub recommended_gain_db: f32,
    pub macro_metadata: [FormulaMacroMetadata; FORMULA_MACRO_COUNT],
}
```

MVP では program は `&'static` でよい。
将来的に user formula を導入する場合のみ `Arc<FormulaProgram>` などに拡張する。

## Built-in formula preset 案

### 1. PurePhase

```text
out = sin(phase)
```

基準音。

### 2. BrightFold

```text
x = sin(phase + macro0 * sin(phase * 2))
out = wavefold(x * (1 + macro1 * 8))
```

macro:

- macro0: phase modulation
- macro1: fold amount

### 3. FM Bell

```text
mod = sin(phase * ratio) * index * env_like_decay(time)
out = sin(phase + mod)
```

piano / bell 系に使える。

### 4. Additive Odd

```text
out = sin(p) + a3*sin(3p) + a5*sin(5p) + a7*sin(7p)
```

square-like。

### 5. PD Syncish

phase distortion で sync 風の倍音を作る。

```text
p2 = phase_distort(phase, macro0)
out = sin(p2 * (1 + macro1 * 8))
```

### 6. Chaotic Soft

noise hash と logistic-like shaping を少し混ぜる。

```text
n = noise_hash(floor(time * rate))
out = tanh(sin(phase) + macro0 * n)
```

## Phase handling

`phase` は `[0, 1)` の正規化 phase とする。

```rust
self.phase += self.frequency_hz / self.sample_rate;
self.phase -= self.phase.floor();
```

`Sin2Pi` は stack top を `sin(2πx)` として評価する。

## anti-aliasing 方針

数式ベースは aliasing が出やすい。
段階的に対策する。

### MVP

- output soft clip
- high frequency で macro amount を控えめにする optional safety
- formula preset に `max_pitch_hz` or `aliasing_risk` metadata を持たせる

### Phase 2

- PolyBLEP opcode
- bandlimited saw/square opcode
- 2x oversampling option

### Phase 3

- per-formula oversampling
- minBLEP table
- wavetable baking

## パラメータ設計

`ParamId` に追加する候補。

```rust
FormulaProgram
FormulaMacro1
FormulaMacro2
FormulaMacro3
FormulaMacro4
FormulaMacro5
FormulaMacro6
FormulaMacro7
FormulaMacro8
FormulaOutputGain
FormulaOversampling
FormulaDcBlock
```

plugin 表示名:

- Formula
- Macro 1..8
- Output Gain
- Oversampling
- DC Block

## 実装ファイル

```text
crates/z-audio-dsp/src/formula/mod.rs
crates/z-audio-dsp/src/formula/opcode.rs
crates/z-audio-dsp/src/formula/program.rs
crates/z-audio-dsp/src/formula/runtime.rs
crates/z-audio-dsp/src/formula/generator.rs
crates/z-audio-dsp/src/formula/builtins.rs
```

`lib.rs` に追加。

```rust
pub mod formula;
pub use formula::{FormulaGenerator, FormulaParams, FormulaProgram, FormulaProgramId};
```

## runtime 擬似コード

```rust
fn eval_program(&mut self, phase: f32) -> f32 {
    let mut sp = 0usize;

    for op in self.program.opcodes {
        match *op {
            FormulaOpcode::Const(v) => push(v),
            FormulaOpcode::Phase => push(phase),
            FormulaOpcode::FrequencyHz => push(self.frequency_hz),
            FormulaOpcode::Velocity => push(self.velocity),
            FormulaOpcode::Macro(i) => push(self.macros[i as usize]),
            FormulaOpcode::Add => bin(|a, b| a + b),
            FormulaOpcode::Sin2Pi => unary(|x| (TAU * x).sin()),
            FormulaOpcode::TanH => unary(|x| x.tanh()),
            FormulaOpcode::WaveFold => unary(wavefold),
            ...
        }
    }

    self.stack[0] * self.program.output_scale
}
```

## テスト項目

- program compile stack depth が上限内に収まる
- phase が wrap する
- output が NaN / Inf にならない
- silence / max macro / high pitch で panic しない
- deterministic noise hash
- `process_mono()` が out 長に正確に書く
- `reset()` で phase/time が戻る

## 将来の文字列式

文字列式を入れるなら、以下の順序。

1. UI thread で parse
2. AST 作成
3. constant folding
4. stack depth validation
5. opcode compile
6. immutable program swap

program swap は audio thread から直接 `Arc` clone/drop しない。
plugin 側で lock-free double buffer または atomic pointer swap を使う。
MVP では避ける。
