# VCSLサンプラーピアノ実装計画

作成日: 2026-06-26

この文書は実装前の計画です。現時点では Rust 実装、Cargo 設定、README、既存 WebCLAP UI、`docs/VCSL_Keys.zip` の中身は変更しません。

## 目的

`docs/VCSL_Keys.zip` に入っている VCSL Keys の SFZ/FLAC サンプルを使い、サンプラーベースのピアノをこのリポジトリで実装する。

対象フォーマットは次の3つです。

- Native VST3
- Native CLAP
- WebCLAP

共通方針は、サンプラーとしての基本 DSP と楽器エンジンを `thirdparty/z-audio-dsp` 側に置き、ルートリポジトリでは native wrapper、WebCLAP wrapper、GUI、バンドル、サンプルアセット変換を管理することです。

既存の `Z Audio Formula Piano` は式/モーダル合成ピアノなので、まずは置き換えずに新規の `Z Audio VCSL Piano` として追加する方が安全です。最終的に既存ピアノを置き換えるか、別プラグインとして残すかは実装後の音質と配布サイズを見て判断します。

## 参照素材

公式ページ: https://versilian-studios.com/vcsl-keys/

公式ページ上の要点:

- VCSL Keys は SFZ 形式の鍵盤音源。
- VCSL Keys のサンプル/パッチは Creative Commons 0 (CC0) と説明されている。
- 内容は 3台のグランド、2台のアップライト、5台のハープシコード。
- 1,466 samples / 1,466 files と説明されている。
- すべてのピアノはリリースサンプルを持ち、`S Model B` と `Knight` はサステインペダル用サンプルも持つ。

ローカルの `docs/VCSL_Keys.zip` から確認したピアノ系の候補は次の通りです。

| 候補 | SFZ | region数 | FLAC数 | FLAC合計サイズ | 備考 |
| --- | --- | ---: | ---: | ---: | --- |
| Grand Piano, K | `Grand Piano, K.sfz` | 228 | 228 | 約42.8 MB | MVP向き。サイズが小さく、attack/release と複数velocity layerがある。 |
| Grand Piano, S Model B 1895 | `Grand Piano, S Model B 1895.sfz` | 351 | 351 | 約186.6 MB | リッチだが大きい。CC64で sustain/no-sustain layer を切り替える。 |
| Upright Piano, Knight | `Upright Piano, Knight.sfz` | 143 | 143 | 約159.1 MB | CC64/sequence系 opcode がある。 |
| Upright Piano, Y | `Upright Piano, Y.sfz` | 142 | 143 | 約41.5 MB | サイズは小さいがアップライト系。 |

MVPは `Grand Piano, K` を対象にします。理由は、WebCLAP の配布サイズと初期実装の SFZ 対応範囲を抑えつつ、ピアノとして必要な複数ベロシティ層とリリースサンプルを検証できるためです。`S Model B 1895` は第2段階で、CC64 sustain/no-sustain layer と大容量アセット配布が固まってから対応します。

## 実装全体像

```text
docs/VCSL_Keys.zip
  -> offline asset preparation / SFZ validation
  -> generated sample bank manifest + optimized sample payloads
  -> thirdparty/z-audio-dsp sampler playback core
  -> thirdparty/z-audio-dsp z-audio-synth VCSL piano engine
  -> native VST3/CLAP wrapper + GUI
  -> WebCLAP wrapper + webview GUI
```

オーディオスレッドではファイルI/O、zip展開、FLACデコード、ヒープ確保、ロックをしません。サンプルの読み込み、デコード、manifest 検証はすべて初期化時またはオフライン変換時に行い、`process` では事前確保済みの PCM バッファを読むだけにします。

## アセット管理方針

`docs/VCSL_Keys.zip` は約655MBあり、現在の作業ツリーでは未追跡です。実装前に次のどれで管理するかを決める必要があります。

| 方針 | 長所 | 短所 |
| --- | --- | --- |
| Git LFSで `docs/VCSL_Keys.zip` を管理 | ソース素材をリポジトリ上で再現できる | LFS設定とリモート容量が必要 |
| zipは開発者ローカル/Release asset、生成物だけ管理 | Git本体が軽い | 再生成手順で外部依存が残る |
| MVP用に抽出済みの小さい bank のみ管理 | WebCLAP検証が速い | フルVCSL再現ではない |

推奨は、初期MVPでは `docs/VCSL_Keys.zip` を入力として扱い、`target/` または `assets/generated/` に生成するサンプル bank は実装検証用に生成するだけにすることです。配布段階で Git LFS または Release asset に移します。

生成物の候補:

```text
assets/vcsl-piano/
  Grand Piano, K/
    bank.json          # SFZから変換したregion/metadata
    samples/*.pcm16    # または *.flac / *.wav / custom packed format
    LICENSE.txt        # VCSL Keys CC0由来であることを明記
```

WebCLAP は特にサイズ制約が強いので、最初は `Grand Piano, K` のみ、必要ならノート/velocityを間引いた dev bank を用意します。フルバンク同梱は、ホストでのロード時間と `.wclap.tar.gz` サイズを実測してから決めます。

## SFZ対応範囲

MVPで対応する opcode は、`Grand Piano, K.sfz` に必要なものに限定します。

| opcode | 用途 | MVP |
| --- | --- | --- |
| `sample` | サンプルファイル参照 | 必須 |
| `lokey` / `hikey` | key range | 必須 |
| `pitch_keycenter` | 元サンプルの基準ノート | 必須 |
| `lovel` / `hivel` | velocity range | 必須 |
| `volume` / `global_volume` | dB gain | 必須 |
| `tune` | cent単位の補正 | 必須 |
| `offset` | 再生開始サンプル位置 | 必須 |
| `amp_veltrack` | velocityによる音量追従 | 必須、sforzandoとの比較で調整 |
| `ampeg_attack` / `ampeg_release` | 簡易amp envelope | 必須 |
| `ampeg_decay` / `ampeg_sustain` | envelope補助 | 対応するがまずは単純化 |
| `trigger=attack` | NoteOnで鳴るregion | 必須 |
| `trigger=release` | NoteOffで鳴るリリースregion | 必須 |
| `rt_decay` | release triggerの減衰補助 | 可能ならMVP、難しければ明示的に未対応ログ |

第2段階で追加する opcode:

| opcode | 用途 |
| --- | --- |
| `locc64` / `hicc64` | sustain pedal stateによるlayer選択 |
| `on_locc64` / `on_hicc64` | NoteOn時CC64条件 |
| `seq_length` / `seq_position` | round-robin/sequence selection |
| `pitch_keytrack` | pitch tracking補正 |

SFZ全体を汎用実装するのではなく、VCSL Keys のピアノを正しく鳴らすためのサブセットとして扱います。未対応 opcode は無視せず、asset preparation の検証で一覧化して fail または warning にします。

## DSP実装方針

### 低レベルサンプラー

場所:

```text
thirdparty/z-audio-dsp/crates/z-audio-dsp/src/sampler/
```

追加候補:

- `SampleBuffer`: mono/stereo PCM、元サンプルレート、フレーム数。
- `SampleRegion`: key range、velocity range、root key、tune、volume、offset、trigger種別、envelope。
- `SamplerVoice`: 1サンプル再生voice。phase、pitch ratio、gain envelope、release stateを持つ。
- `SamplerEngine`: voice pool、voice stealing、note on/off、release sample spawn、stereo mix。
- `Interpolation`: MVPはlinear、後で4-point Hermiteなどを追加。

重要な制約:

- `process_*` 中に allocation しない。
- `process_*` 中に file I/O、decode、log出力をしない。
- note-onで使う voice pool は `prepare()` または engine 初期化時に確保する。
- pitch ratio は `2^((note - pitch_keycenter + tune / 100) / 12)` を基準に、サンプルレート差を掛ける。
- denormal対策は既存の `flush_denormal` 方針に合わせる。

### VCSLピアノエンジン

場所:

```text
thirdparty/z-audio-dsp/crates/z-audio-synth/src/vcsl_piano/
```

追加候補:

- `VcslPiano`
- `VcslPianoConfig`
- `VcslPianoParams`
- `VcslSampleBank`
- `VcslRegionMap`

`z-audio-synth` は MIDI/note handling と楽器レベルの状態管理を担当し、低レベルの補間や voice mix は `z-audio-dsp::sampler` に寄せます。

`EventKind` は現在 `NoteOn`、`NoteOff`、`Param` のみです。CC64 sustain pedal を扱う段階では、次のどちらかが必要です。

- `EventKind::MidiCc { cc, value }` を追加する。
- sustain pedal を `ParamId` として扱い、native/WebCLAP wrapper 側で CC64 を param event に変換する。

MVPの `Grand Piano, K` では CC64 layer がないため、NoteOn/NoteOff と params だけで開始できます。

## パラメータ設計

`ParamId` は現状 `FormulaDrumKit` が 160-171 を使っています。VCSLピアノ用には 180 番台以降を予約します。

MVP候補:

| Param | 範囲 | 用途 |
| --- | ---: | --- |
| Instrument | enum | MVPは `Grand Piano K` のみ。将来 `S Model B` / Uprightを追加。 |
| Master Gain | -24..12 dB | 出力音量 |
| Tone | 0..1 | 簡易ローパス/明るさ調整 |
| Velocity Curve | 0..1 | MIDI velocity の反応調整 |
| Release Level | -24..12 dB | release sample の音量 |
| Release Time | 0.05..5.0 s | NoteOff後の減衰 |
| Stereo Width | 0..1 | ステレオ幅 |
| Max Polyphony | 8..128 | voice上限 |
| Pedal | 0..1 or host CC64 | 第2段階でCC64対応 |

既存の `FormulaPiano` パラメータ名と衝突しないよう、metadata名は `VCSL Master Gain` のように区別します。

## Native VST3/CLAP

新規crateとして追加する案:

```text
crates/z-audio-vcsl-piano-plugin/
  Cargo.toml
  src/lib.rs
  src/editor.rs
```

想定メタデータ:

- Plugin name: `Z Audio VCSL Piano`
- CLAP ID: `dev.zaudio.vcsl-piano`
- VST3 Class ID: `ZAudioVCSLPiano1`
- Category: instrument / piano / stereo
- MIDI input: basic MIDI
- Audio input: none
- Audio output: stereo

native wrapper の責務:

- `nih_plug` で VST3/CLAP を公開する。
- MIDI NoteOn/NoteOff を `TimedEvent` に変換する。
- 将来は CC64 を `TimedEvent` または param event に変換する。
- 初期化時または background task で sample bank を読み込む。
- サンプルが見つからない場合は無音にせず、GUI上に明確な missing asset 状態を出す。

GUI は `nih_plug` の editor として実装します。既存の effect UI とは違い、ピアノ向けに次を置きます。

- instrument selector
- master/tone/release/velocity/stereo/polyphony controls
- sample bank load status
- MIDI入力/active voice表示
- 簡易keyboard表示

## WebCLAP

新規crateとして追加する案:

```text
crates/z-audio-webclap-vcsl-piano/
  Cargo.toml
  build.rs
  plugin.json
  src/lib.rs
  ui/index.html
  ui/main.js
  ui/styles.css
```

`PluginDef` は `ui_path: Some(b"/ui/index.html\0")` にして、`plugin.json` は `has_ui: true` とします。`cargo xtask bundle-webclap --release` は既に `ui/` を `.wclap` と `.wclap.tar.gz` にコピーする仕組みを持つため、新しい bundle を `crates/xtask/src/main.rs` に追加します。

WebCLAPのサンプル配布には2案あります。

| 案 | 内容 | 判断 |
| --- | --- | --- |
| Embedded bank | `include_bytes!` で圧縮済みbankを wasm に含める | MVPで最も確実。wasmが大きくなる。 |
| Bundle asset | `.wclap/assets/` に bank を同梱し、WASI/host経由で読む | 理想形。host互換性の検証が必要。 |

MVPは embedded bank で始めます。ただし最初からフル `S Model B` を埋め込むのは避け、`Grand Piano, K` または dev bank にします。WebCLAPで `.wclap/assets/` のファイルアクセスが実証できたら bundle asset 方式へ移行します。

WebCLAP UI は既存の `crates/z-audio-webclap-eq/ui/` と同じ、HTML/CSS/JS の軽量構成に寄せます。

UI要素:

- top status: loaded / loading / missing samples / host connected
- compact controls: Tone、Dynamics、Release、Width、Gain
- instrument selector
- voice meter
- simple piano keyboard visual
- optional sample bank details

UIから音を鳴らすオンスクリーン鍵盤は、UI-to-plugin note event のプロトコルを追加できる場合のみ実装します。最初のGUI要件は、ホスト/MIDI入力で演奏し、GUIは状態とパラメータを制御できることに置きます。

## オフライン変換ツール

`xtask` に次のサブコマンドを追加する想定です。

```powershell
cargo xtask prepare-vcsl-piano --instrument "Grand Piano, K" --source docs/VCSL_Keys.zip
```

処理:

1. zip内の対象SFZを読む。
2. 対応 opcode をパースする。
3. 各 `sample=` パスがzip内に存在することを検証する。
4. 未対応 opcode を一覧化する。
5. FLACをデコードして、pack形式へ変換する。
6. region manifest と sample payload を生成する。
7. manifest に入力zipのhash、SFZ hash、生成日時、instrument名を記録する。

サンプル形式は実装時に決めます。候補は次の通りです。

| 形式 | 長所 | 短所 |
| --- | --- | --- |
| FLACのまま | サイズが小さい | runtime decodeが必要 |
| WAV/PCM | 読み込みが簡単 | サイズが大きい |
| custom packed PCM16/PCM24 | ロードと同梱のバランスが良い | packer/loader実装が必要 |

MVPは custom packed PCM16 または FLACのまま+初期化時decode のどちらかにします。WebCLAPではロード時間とメモリ消費を測り、フル解凍PCMを常駐させて問題ないか確認します。

## テスト計画

### DSP/submodule

`thirdparty/z-audio-dsp` 側で追加するテスト:

- region selection: note/velocityから正しい `SampleRegion` を選ぶ。
- pitch ratio: root key、tune、sample rate差が正しく反映される。
- interpolation: out-of-range read が起きない。
- release trigger: NoteOffで release sample voice が起動する。
- voice stealing: 最大polyphony到達時に古い/releasing voice を優先して奪う。
- no allocation during process: 既存方針に合わせて確認する。
- golden render: C4単音、低音、高音、和音、release sample入りの短いwav/json snapshot。

### asset preparation

- `Grand Piano, K.sfz` の全regionをパースできる。
- `sample=` の参照先がすべて zip 内に存在する。
- 対応外 opcode が想定リスト内に収まる。
- 生成manifestのhashが安定する。
- 生成bankをロードして全regionのPCMフレーム数が0でない。

### native plugin

- `cargo test --workspace`
- `cargo xtask bundle z-audio-vcsl-piano-plugin --release`
- VST3/CLAP bundleに必要なresourceまたは外部bank参照が入る。
- sample bank missing時のGUI表示と無音安全性。
- DAW/hostで MIDI note input、polyphony、release、automation を確認。

### WebCLAP

- `cargo check --target wasm32-unknown-unknown -p z-audio-webclap-vcsl-piano`
- `cargo xtask bundle-webclap --release`
- tarballに `module.wasm`、`plugin.json`、`ui/`、必要なら `assets/` が入る。
- WebCLAP hostで load、MIDI input、parameter set/get、state save/load、GUI表示を確認。
- large wasm/asset のロード時間とメモリ使用量を記録する。

## マイルストーン

1. **素材/ライセンス確認**: VCSL Keys公式ページ、zip構成、CC0表記、対象SFZのopcodeを再確認する。
2. **asset preparation MVP**: `Grand Piano, K.sfz` から manifest と小さいbankを生成する。
3. **low-level sampler DSP**: `z-audio-dsp::sampler` を追加し、単音再生と補間を通す。
4. **VCSL piano engine**: `z-audio-synth::vcsl_piano` を追加し、NoteOn/NoteOff、velocity layer、release sampleを鳴らす。
5. **native plugin**: `Z Audio VCSL Piano` の VST3/CLAP wrapper とGUIを追加する。
6. **WebCLAP MVP**: embedded/dev bank でWebCLAP + GUIを動かす。
7. **full Grand Piano K**: 全regionを使い、音量/velocity/releaseをsforzando相当へ調整する。
8. **S Model B / CC64**: sustain/no-sustain layer、CC64、重いbankの配布方法を確定する。
9. **release packaging**: README、bundle一覧、license/notice、配布手順を更新する。

## リスクと先に決めること

- `docs/VCSL_Keys.zip` をGitで管理するか、Git LFS/Release assetにするか。
- WebCLAPで大容量サンプルをどう読むか。embedded bankは確実だがwasmが大きい。
- Native VST3/CLAPでサンプルbankを plugin bundle 内に入れるか、ユーザー指定フォルダから読むか。
- FLAC decodeを runtime に持ち込むか、オフラインでPCM bankへ変換するか。
- `FormulaPiano` を残すか、最終的に `Z Audio VCSL Piano` に置き換えるか。
- CC64 sustain pedal を `EventKind` に追加するか、parameter event に変換するか。

## 推奨する初期実装スコープ

最初の実装PRは、次の範囲に絞るのがよいです。

- `Grand Piano, K` のみ。
- `trigger=attack` と `trigger=release`。
- linear interpolation。
- full preload。streamingはしない。
- nativeは外部bankまたはbundle resourceから読む。
- WebCLAPは embedded dev bank でまずGUI/音出しを成立させる。
- GUIは音源選択、Tone、Release、Velocity Curve、Width、Gain、load status に限定する。

この範囲なら、サンプラーDSP、SFZ subset、VST3/CLAP、WebCLAP GUIの一通りを実証しつつ、`S Model B` の大容量/CC64問題を後続タスクとして分離できます。
