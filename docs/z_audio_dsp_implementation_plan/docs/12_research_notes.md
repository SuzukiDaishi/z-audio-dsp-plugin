# 12. 実装調査メモ

## 数式シンセシス関連

### FM / PM

- Yamaha DX 系の基本。
- 数式で扱いやすい。
- piano / bell / metallic tone に強い。
- aliasing に注意。

式:

```text
out = sin(carrier_phase + index * sin(modulator_phase))
```

### Phase distortion

- Casio CZ 系。
- sine table の phase を歪ませる。
- saw / pulse 風の倍音を比較的少ない計算で作れる。

### Wavefolding

- West Coast 系。
- `sin` や triangle を折り返して倍音を作る。
- macro knob との相性が良い。

### Additive synthesis

- partial の gain / decay を数式で制御できる。
- piano の modal bank と相性が良い。
- CPU と aliasing 管理が必要。

## ピアノ合成関連

### サンプル方式を避ける理由

- 今回の目的が数式ベースの実装だから。
- asset なしで配布できる。
- パラメータで大きく変化できる。

### modal synthesis

- 打撃音、ベル、ピアノのような減衰音に向く。
- partial frequency / decay / gain を note ごとに設計する。
- 物理モデルの近似として扱いやすい。

### waveguide string

- 弦モデルに向く。
- fractional delay / damping / dispersion が必要。
- まずは modal bank の方が実装しやすい。

## Reverb 関連

### Schroeder reverb

- comb filter + allpass。
- 実装が簡単。
- metallic になりやすい。

### FDN

- feedback delay network。
- matrix で delay line を混ぜる。
- 高品質な algorithmic reverb の基礎にしやすい。

### Dattorro style

- diffusion tank ベース。
- plate 系に向く。
- 実装資料は多いが tuning が重要。

今回の MVP は FDN が扱いやすい。

## Dynamics 関連

### Feed-forward compressor

- 入力から level を検出して gain を計算。
- 動作が予測しやすい。
- plugin として一般的。

### Feedback compressor

- 出力から level を検出。
- アナログ機材風にしやすい。
- tuning が難しい。

MVP は feed-forward。

### Limiter

- compressor ratio 無限大に近いが、実装上は lookahead と ceiling が重要。
- true peak には oversampling detector が必要。
- latency compensation が plugin 品質に直結する。

## 推奨する音質改善順

1. NaN/Inf/暴走しない
2. parameter smoothing
3. denormal 対策
4. aliasing 対策
5. oversampling
6. SIMD
7. preset tuning

## 参考実装で見るべきキーワード

Web 検索やコード調査で見るなら以下。

- `feedback delay network reverb hadamard`
- `Dattorro plate reverb implementation`
- `Schroeder allpass comb reverb`
- `digital dynamic range compressor ballistics`
- `lookahead limiter implementation`
- `modal synthesis piano resonator bank`
- `Karplus Strong stiff string dispersion`
- `PolyBLEP oscillator rust`
- `audio plugin sample accurate automation nih-plug`
