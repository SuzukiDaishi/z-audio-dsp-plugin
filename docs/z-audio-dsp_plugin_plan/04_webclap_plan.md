# 04. WebCLAP Plan

## 方針

WebCLAP は実験的ターゲットとして扱います。`wclap.plinken.org`（[`taluvi-dev/plinken-org`](https://github.com/taluvi-dev/plinken-org)、
upstreamは[`github.com/WebCLAP`](https://github.com/WebCLAP)）でロード可能な、本物のCLAP-on-wasm
プラグインを生成することを最終目標とします。

目標:

```text
z-audio-dsp / z-audio-synth の同じDSPコアを、
本物のWCLAP（CLAP ABIをそのままexportするwasm）として
wclap.plinken.org 等のWCLAPホストで動かす。
```

## 背景

[`github.com/WebCLAP`](https://github.com/WebCLAP) organizationが「ブラウザでCLAPプラグインを
ホストする」仕様・実装一式（`wclap-host-js`/`wclap-host-cpp`/`as-clap`/`examples`等）を公開しており、
`taluvi-dev/plinken-org`はこれを使った実サイト（`wclap.plinken.org`）とプラグイン集を持っています。
詳細は[`REFERENCES.md`](./REFERENCES.md)を参照。

## 実装の段階（履歴）

### Stage 1: Pure WASM AudioWorklet（実証用MVP、Stage 2/3に置き換え済み）

最初からWebCLAPに行かず、まずRust/WASM + AudioWorkletでDSPが鳴ることを確認しました
（`wasm-bindgen`ベース、本物のCLAP ABIは持たないカスタムインターフェース）。目的（wasm32での
ビルド確認、block size 128での安定性確認、parameter/message bridgeの確認）はすべて達成し、
Stage 2/3着手にあたって削除しました。

### Stage 2/3: 本物のWCLAP（実装済み — 詳細は下記「Crate layout」）

```text
z-audio-synth (SimpleSynth)
  -> wclap-plugin (vendored CLAP/WCLAP ABI scaffold)
  -> wasm32-unknown-unknown cdylib (clap_entry export)
  -> wclap-host-js / wclap.plinken.org
  -> AudioWorklet
  -> Browser
```

## Stage 2/3 Deferral Findings (2026-06-14) — 撤回 (2026-06-19)

2026-06-14時点では、Stage 2/3（実WebCLAP）を以下の理由で非現実的と判断し、deferredとしていました。

- `browser-test-host`はEmscripten前提のC++ホストであり、Rust製pluginのwasmをそのまま
  ロードする経路がない（C++側のCLAP host実装＋Emscripten toolchainが前提）。
- RustのCLAP実装crateである`clack`（`clack-plugin`/`clack-host`）は`wasm32-unknown-unknown`
  ターゲットに対応していない（ホストOS向けABI/動的ライブラリロードを前提とした実装）。

**この判断は誤りでした。** `wclap.plinken.org`（[`taluvi-dev/plinken-org`](https://github.com/taluvi-dev/plinken-org)、
upstreamの[`WebCLAP`](https://github.com/WebCLAP) organization配下）を調査したところ:

- `clack-plugin`は実際には`wasm32-unknown-unknown`向けに普通の`cdylib`としてビルドでき、
  `clack_export_entry!`マクロが`clap_entry`をそのままexportする（`WebCLAP/examples`の
  `clack-gain.wasm`/`clack-polysynth.wasm`がその実例）。2026-06-14の調査時点でこの事実を
  見落としていました。
- さらに、`taluvi-dev/plinken-org`は独自に`crates/wclap-plugin`という、CLAP ABI（`clap_entry`、
  factory、plugin vtable、audio-ports/note-ports/params/state拡張）を素のRust
  （`#![no_std] + alloc`、`extern "C"`関数とオフセット直接書き込み、clackより薄い）で実装した
  共有crateを持っており、これをそのまま使えば**Rustだけで完結する**ことが分かりました。
- 残るギャップは「ノートイベント読み取り」のみで、upstreamの`wclap-plugin`はeffectプラグイン
  しか実装しておらず（唯一のinstrument実装である`synome`は"Phase A scaffold — silent"という
  明示的なスタブ）、`ProcessCtx`に`clap_event_note`を読むAPIがありませんでした。これは
  `crates/wclap-plugin`（vendor、下記参照）に`ProcessCtx::note_events()`として追加しました。

これにより、Stage 2/3を実装済みとし、Stage 1（wasm-bindgen + AudioWorklet）は置き換えました。

## Crate layout（実装済み・Stage 2/3）

```text
crates/
├── wclap-plugin/                # taluvi-dev/plinken-orgからvendor（MIT）。CLAP/WCLAP ABIスキャフォールド。
│   ├── Cargo.toml
│   ├── LICENSE
│   ├── NOTICE.md                # vendor元 + ローカル差分（note_events()追加）の記録
│   └── src/lib.rs
└── z-audio-webclap/             # 本体プラグイン。clap_entryをexportする本物のWCLAP cdylib。
    ├── Cargo.toml
    ├── build.rs                 # --export-table --growable-table（wclap-host-jsの関数テーブル拡張に必要）
    ├── plugin.json               # plinken-org `plugins/<vendor>/<name>/plugin.json` 規約に合わせたmanifest
    └── src/lib.rs                # wclap_plugin::Plugin を実装し z-audio-synth::SimpleSynth をラップ
```

`crates/z-audio-plugin`（VST3/CLAP, nih-plug）と同じ`z-audio-dsp`/`z-audio-synth`を再利用しており、
パラメータは`ParamId::ALL`（`ParamId::MaxPolyphony`除く）から自動生成しています。

### ローカルでの動作確認（2026-06-19）

`taluvi-dev/plinken-org`を`git clone` + `git submodule update --init`し、`apps/wclap-host`を
`pnpm install` + `vite`でローカル起動、PlaywrightでヘッドレスChromiumから操作して確認:

- `wasm32-unknown-unknown --release`でビルドした`z_audio_webclap.wasm`を本物のWCLAPホスト
  （Rust製`host-rust.wasm`）にロード → エラーなく `"Z Audio Simple Synth loaded in slot 1."` と表示。
- `clap_event_note`（NOTE_ON/NOTE_OFF）を送ると`ProcessCtx::note_events()`が正しく検出し、
  `SimpleSynth::note_on`/`note_off`が呼ばれ、音声出力に反映される（`active_voice_count()`が
  1になり、envelopeのattack/releaseが波形に反映される）ことをアンプ済み波形で確認。
- **既知の制限（WCLAP側の問題ではない）**: デフォルトパラメータのまま中音域のノート
  （例: ノート60 = 261.6Hz）を鳴らすと、3バンドEQ（Low 200Hz LowPass / Mid 1000Hz BandPass /
  High 5000Hz HighPass、すべて既定で有効・直列）が重なって**-70dB近くまで信号を削ってしまう**
  ことを実測で確認した（EQの3バンドを無効化すると振幅が0.0003→0.63に戻る）。これは
  `thirdparty/z-audio-dsp`側（`ThreeBandButterworthEq`の既定値）の特性で、ネイティブVST3/CLAP
  ビルドや旧Stage 1のAudioWorkletデモにも同様に影響する。WCLAP固有の問題ではないため、この
  ドキュメントでは現状維持のまま記録するに留め、修正は別タスクとする。

## Block size

`process()`に渡される`frames`はホスト依存（AudioWorkletのrender quantum、通常128samples）。
`activate(sample_rate, max_frames)`で`SimpleSynthConfig::max_block_size`を都度作り直すことで対応。

## Parameter bridge

`wclap_plugin::Plugin::params()`が`ParamId::ALL`（`MaxPolyphony`除く）から`ParamDef`の静的配列を
生成し、`clap.params`拡張として公開する。ホスト（wclap-host-js）の汎用パラメータUIや、将来の
DAW側オートメーションから`set_param(id, value)` / `get_param(id) -> value`として読み書きされる。
`clap.state`拡張（パラメータダンプの保存/復元）も`wclap-plugin`が自動的に提供する。

## MIDI / Note input

`PluginDef::note_inputs = 1`で`clap.note-ports`を公開し、`ProcessCtx::note_events()`
（`crates/wclap-plugin`にローカル追加）で`clap_event_note`（NOTE_ON/NOTE_OFF）を読み取り、
`z_audio_dsp::EventKind::NoteOn/NoteOff`に変換して`SimpleSynth::process_with_context`へ渡す。
wclap-host-js側はWeb MIDI API、または同一チェイン内の「keyboard」プラグインの出力イベント
転送（`com.plinken`系の鍵盤UIプラグインのパターン）からノートイベントを供給する。

## Acceptance Criteria

- [x] ブラウザの実WCLAPホスト（wclap.plinken.org / ローカルの`apps/wclap-host`）でwasmがエラーなくロードされる
- [x] note on/offで`active_voice_count`が変化し、envelopeのattack/releaseが波形に反映される
- [x] LFO/EQ等のパラメータは`clap.params`経由で読み書きできる（`get_param`/`set_param`の往復を確認）
- [ ] 実際に耳で聴いて確認（既定パラメータでは3バンドEQの重なりで中音域が極端に静かなので、要パラメータ調整 — 上記「ローカルでの動作確認」参照）

## Risks

- `plugins/` manifest（`plugin.json`の`format`/`artifact`等）の仕様はupstream
  （`taluvi-dev/plinken-org`）側のドキュメントが薄く、コード読解で仕様を確定させた。
  upstreamが仕様を変更した場合は追従が必要。
- `crates/wclap-plugin`はvendor（コピー）であり、upstreamの更新を自動的には取り込まない。
- 3バンドEQの既定値が中音域を大きく削る件は、WCLAP固有ではなく`thirdparty/z-audio-dsp`側の
  特性なので、ここでは現状記録のみとし別途対応する。

## Decision

Stage 1（wasm-bindgen + AudioWorklet）は実証目的のMVPとして役目を終え、Stage 2/3
（`crates/wclap-plugin` + `crates/z-audio-webclap`による本物のWCLAP）に置き換えた。

WebCLAPをいきなりMVP条件にするとリスクが高いため、まずAudioWorkletでDSPのwasm適性を確認します。
