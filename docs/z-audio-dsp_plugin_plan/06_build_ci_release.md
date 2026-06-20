# 06. Build / CI / Release Plan

## Dependencies

DSPコア（`z-audio-dsp`/`z-audio-synth`）は https://github.com/SuzukiDaishi/z-audio-dsp.git を
`thirdparty/z-audio-dsp`にgit submoduleとして取り込み、path dependencyとして参照します。

```bash
git submodule update --init --recursive
```

```toml
[dependencies]
z-audio-dsp   = { path = "../../thirdparty/z-audio-dsp/crates/z-audio-dsp" }
z-audio-synth = { path = "../../thirdparty/z-audio-dsp/crates/z-audio-synth" }
```

submoduleのコミット固定がそのまま依存バージョンの固定になります。更新時は
`git -C thirdparty/z-audio-dsp fetch && git -C thirdparty/z-audio-dsp checkout <rev>` で
submodule側のコミットを進め、plugin側のテストを通すワークフローにします。

ルート`Cargo.toml`はvirtual workspaceで`thirdparty/z-audio-dsp`を`exclude`しています
（thirdparty側もworkspaceのため、ネストされたworkspaceとのpath依存衝突を避けるため）。

### nih-plug

`crates/z-audio-plugin`と`crates/xtask`は nih-plug を git dependencyとして参照し、
両方を同じ`rev`にpinします（VST3/CLAP実装とxtask bundlerのABI不整合を避けるため）。

```toml
nih_plug       = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "f36931f7af4646065488a9845d8f8c2f95252c23" }
nih_plug_xtask = { git = "https://github.com/robbert-vdh/nih-plug.git", rev = "f36931f7af4646065488a9845d8f8c2f95252c23" }
```

更新時は両方の`rev`を同じコミットに合わせて上げます。

## Targets

### Native

- Windows x86_64
- macOS universal or x86_64/aarch64
- Linux x86_64

### Web

- wasm32-unknown-unknown（Stage 1 AudioWorklet MVP: `crates/z-audio-webclap`）
- Emscriptenベースの実WebCLAP（Stage2/3）は[04_webclap_plan.md](04_webclap_plan.md)の
  「Stage 2/3 Deferral Findings」の通りdeferred

## First build strategy

```bash
# 0. submodule取得（初回のみ）
git submodule update --init --recursive

# DSP（thirdparty submodule、direct buildでの検証用）
cargo build -p z-audio-dsp
cargo build -p z-audio-synth

# Plugin (VST3 + CLAP, nih-plug)
cargo xtask bundle z-audio-plugin --release
# -> target/bundled/Z Audio Simple Synth.vst3
# -> target/bundled/Z Audio Simple Synth.clap

# Web (WASM + AudioWorklet, Stage 1 MVP)
rustup target add wasm32-unknown-unknown
# crates/z-audio-webclap/Cargo.toml の wasm-bindgen と同じバージョンをインストール
cargo install wasm-bindgen-cli --version 0.2.104
cargo build -p z-audio-webclap --release --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/z_audio_webclap.wasm \
  --out-dir crates/z-audio-webclap/web/pkg --target web
# crates/z-audio-webclap/web/ をhttp(s)で配信（AudioWorkletはsecure/local contextが必要）
```

## CI

GitHub Actions想定。

```text
check:
  cargo fmt --check
  cargo clippy
  cargo test

build:
  windows native
  macos native
  linux native

plugin:
  build VST3/CLAP bundle if supported by CI

web:
  cargo build --target wasm32-unknown-unknown
```

## Artifacts

```text
release/
├── windows/
│   ├── Z Audio Simple Synth.vst3
│   └── Z Audio Simple Synth.clap
├── macos/
│   ├── Z Audio Simple Synth.vst3
│   └── Z Audio Simple Synth.clap
├── linux/
│   ├── Z Audio Simple Synth.vst3
│   └── Z Audio Simple Synth.clap
└── web/
    ├── index.html
    ├── main.js
    ├── worklet-processor.js
    └── pkg/                    # wasm-bindgen --target web 出力一式
        ├── z_audio_webclap.js
        ├── z_audio_webclap_bg.wasm
        └── ...
```

## Signing / notarization

第一弾では後回し。  
macOS配布時はGatekeeperに注意。

## Versioning

```text
0.1.0-dsp-mvp
0.1.0-plugin-mvp
0.1.0-web-mvp
```

## Release checklist

- DSP tests pass
- plugin loads in at least 2 hosts
- no crash on parameter changes
- MIDI note on/off works
- state save/load works
- Web AudioWorklet demo runs
- README contains installation instructions
