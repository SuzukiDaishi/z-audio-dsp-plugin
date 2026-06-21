# 09. テスト・ベンチマーク・音質検証

## ゴール

実装後に「動く」だけでなく、DAW / WebCLAP / ゲームランタイムで安全に使える状態にする。

## テスト分類

1. Unit tests
2. DSP numerical tests
3. Offline render tests
4. Snapshot / golden tests
5. Performance benchmarks
6. Plugin smoke tests
7. WebAssembly smoke tests

## 共通 unit tests

すべての DSP module に共通。

- silence input を処理して NaN/Inf が出ない
- max/min parameter で panic しない
- sample rate 44.1 / 48 / 96 / 192 kHz
- block size 1 / 16 / 64 / 128 / 1024
- reset 後に状態がクリアされる
- prepare 後に処理できる
- process 中に出力 slice 長を越えない

## Formula tests

- built-in program 全部が eval できる
- macro min/max で NaN/Inf なし
- phase wrap
- deterministic noise
- stack depth validation
- `reset()` で phase/time reset
- high frequency でも panic なし

## Piano tests

- NoteOn で音が出る
- NoteOff で release へ移る
- sustain pedal on で release が伸びる
- voice stealing が動く
- velocity によって RMS が変わる
- 低音 / 高音で NaN/Inf なし
- 88鍵を順に鳴らして crash しない

## Reverb tests

- impulse input で tail が出る
- decay を長くすると tail が長い
- mix 0 で dry と一致
- reset で tail が消える
- feedback gain が 1 を超えない
- damping を上げると高域が減る

## Compressor tests

- ratio 1 で変化しない
- threshold 以下で変化しない
- threshold 以上で gain reduction
- attack / release の時間差
- mix 0 で dry
- mix 1 で wet

## Limiter tests

- ceiling を超えない
- release で戻る
- lookahead ありで transient が抑えられる
- input gain と ceiling の組み合わせ
- reset で lookahead buffer clear

## Offline render examples

`crates/z-audio-examples` に追加。

```text
examples/
  render_formula_synth.rs
  render_formula_piano.rs
  render_reverb_ir.rs
  render_compressor_sweep.rs
  render_limiter_transient.rs
```

出力:

```text
target/audio-tests/
  formula_bright_fold.wav
  piano_chord.wav
  reverb_impulse.wav
  compressor_sweep.wav
  limiter_transient.wav
```

## Golden tests

音声そのものの完全一致は浮動小数点や最適化で壊れやすい。
代わりに特徴量で検証する。

- RMS
- peak
- spectral centroid
- zero crossing rate
- tail energy
- gain reduction max

## Benchmarks

`criterion` を使う。

```text
benches/
  formula_generator.rs
  piano_voice.rs
  reverb.rs
  compressor.rs
  limiter.rs
```

測るもの:

- samples/sec
- CPU per block
- allocation count if possible
- wasm build size

## Realtime safety check

Rust だけでは完全検出できないが、方針を守る。

- process 内 `Vec::new`, `push`, `resize` 禁止
- process 内 `println!`, `log!` 禁止
- process 内 `unwrap()` / `expect()` 禁止
- process 内 lock 禁止
- process 内 `Box`, `Arc` clone/drop を避ける

CI で grep する簡易 check も有効。

```bash
rg "println!|dbg!|unwrap\(|expect\(" crates/z-audio-dsp/src
```

ただし tests には許容するなど調整する。

## Plugin smoke tests

手動確認 checklist:

- REAPER で VST3 scan 成功
- Bitwig で CLAP scan 成功
- parameter 表示
- automation lane 表示
- preset save/load
- bypass
- mono/stereo layout
- sample rate change
- buffer size change

## WebCLAP smoke tests

- wasm build
- demo host で load
- audio callback が動く
- parameter change
- MIDI note for instruments
- browser console error なし

## 音質検証 checklist

### Formula synth

- 低音で太いか
- 高音で aliasing がどれくらい出るか
- macro morph が破綻しないか

### Piano

- attack がピアノらしいか
- decay が自然か
- low note が濁りすぎないか
- high note が細すぎないか
- velocity 表現があるか

### Reverb

- metallic ringing が少ないか
- decay が自然か
- mono compatibility
- piano / drum / voice で破綻しないか

### Dynamics

- pumping が自然か
- limiter が歪みすぎないか
- compressor の attack/release が分かりやすいか
