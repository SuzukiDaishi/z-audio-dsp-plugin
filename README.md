# Z Audio DSP Plugin

Audio plugin wrappers and packaging for `z-audio-dsp`.

This workspace builds native and WebCLAP wrappers for:

- `Z Audio Simple Synth`: MIDI note input to stereo audio output
- `Z Audio Simple EQ`: mono/stereo audio input to audio output
- `Z Audio Formula Piano`: modal/formula piano instrument
- `Z Audio Parametric Reverb`: stereo FDN reverb effect
- `Z Audio Limiter`: stereo lookahead limiter effect
- `Z Audio Compressor`: stereo feed-forward compressor effect

Supported plugin formats:

- Native VST3
- Native CLAP
- WebCLAP (`.wclap` directory and `.wclap.tar.gz`)

The DSP and synth implementation lives in the `thirdparty/z-audio-dsp` git
submodule. This repository owns the native adapters, WebCLAP adapters, UI
assets, and packaging tasks.

## Layout

```text
crates/
  z-audio-plugin/        Native VST3/CLAP synth
  z-audio-eq-plugin/     Native VST3/CLAP EQ
  z-audio-*-plugin/      Native VST3/CLAP piano, reverb, limiter, compressor
  z-audio-webclap/       WebCLAP synth wasm + UI
  z-audio-webclap-eq/    WebCLAP EQ wasm + UI
  z-audio-webclap-*/     WebCLAP piano, reverb, limiter, compressor wasm
  wclap-plugin/          Minimal WebCLAP/CLAP runtime glue
  xtask/                 Packaging tasks
thirdparty/
  z-audio-dsp/           DSP and synth library submodule
```

Generated artifacts are written under `target/` and are not tracked.

## Setup

Install Rust, then initialize the DSP submodule and wasm target:

```powershell
git submodule update --init --recursive
rustup target add wasm32-unknown-unknown
```

The workspace defines this cargo alias in `.cargo/config.toml`:

```powershell
cargo xtask ...
```

## Build Native Plugins

Bundle the synth:

```powershell
cargo xtask bundle z-audio-plugin --release
```

Bundle the EQ:

```powershell
cargo xtask bundle z-audio-eq-plugin --release
```

Bundle the additional plugins:

```powershell
cargo xtask bundle z-audio-piano-plugin --release
cargo xtask bundle z-audio-reverb-plugin --release
cargo xtask bundle z-audio-limiter-plugin --release
cargo xtask bundle z-audio-compressor-plugin --release
```

Outputs:

```text
target/bundled/Z Audio Simple Synth.vst3
target/bundled/Z Audio Simple Synth.clap
target/bundled/Z Audio Simple EQ.vst3
target/bundled/Z Audio Simple EQ.clap
target/bundled/Z Audio Formula Piano.vst3
target/bundled/Z Audio Formula Piano.clap
target/bundled/Z Audio Parametric Reverb.vst3
target/bundled/Z Audio Parametric Reverb.clap
target/bundled/Z Audio Limiter.vst3
target/bundled/Z Audio Limiter.clap
target/bundled/Z Audio Compressor.vst3
target/bundled/Z Audio Compressor.clap
```

## Build WebCLAP Plugins

Build wasm and package all WebCLAP plugins:

```powershell
cargo xtask bundle-webclap --release
```

Outputs:

```text
target/webclap/z-audio-simple-synth.wclap/
target/webclap/z-audio-simple-synth.wclap.tar.gz
target/webclap/z-audio-simple-eq.wclap/
target/webclap/z-audio-simple-eq.wclap.tar.gz
target/webclap/z-audio-formula-piano.wclap/
target/webclap/z-audio-formula-piano.wclap.tar.gz
target/webclap/z-audio-parametric-reverb.wclap/
target/webclap/z-audio-parametric-reverb.wclap.tar.gz
target/webclap/z-audio-limiter.wclap/
target/webclap/z-audio-limiter.wclap.tar.gz
target/webclap/z-audio-compressor.wclap/
target/webclap/z-audio-compressor.wclap.tar.gz
```

Use the `.wclap.tar.gz` files when testing with WebCLAP hosts such as Plinken.

The synth, EQ, reverb, limiter, and compressor tarballs contain these paths at
archive root:

```text
module.wasm
plugin.json
ui/index.html
ui/main.js
ui/styles.css
```

The piano WebCLAP bundle currently exposes host parameters without a custom
WebCLAP UI, so its tarball contains `module.wasm` and `plugin.json`.

## Plugin IDs

| Plugin | CLAP ID | VST3 Class ID | WebCLAP bundle |
| --- | --- | --- | --- |
| Z Audio Simple Synth | `dev.zaudio.simple-synth` | `ZAudioSmplSynth1` | `z-audio-simple-synth.wclap.tar.gz` |
| Z Audio Simple EQ | `dev.zaudio.simple-eq` | `ZAudioSimpleEQ01` | `z-audio-simple-eq.wclap.tar.gz` |
| Z Audio Formula Piano | `dev.zaudio.formula-piano` | `ZAudioFormulaPno` | `z-audio-formula-piano.wclap.tar.gz` |
| Z Audio Parametric Reverb | `dev.zaudio.parametric-reverb` | `ZAudioParaReverb` | `z-audio-parametric-reverb.wclap.tar.gz` |
| Z Audio Limiter | `dev.zaudio.limiter` | `ZAudioLimiter000` | `z-audio-limiter.wclap.tar.gz` |
| Z Audio Compressor | `dev.zaudio.compressor` | `ZAudioCompressor` | `z-audio-compressor.wclap.tar.gz` |

## Parameters

### Synth WebCLAP UI

The synth WebCLAP UI intentionally exposes only the main instrument controls:

- Oscillator: shape, level, pulse width
- Amp envelope: attack, decay, sustain, release, curve
- LFO: waveform, rate, depth, route
- Output: master

Lower-level synth parameters such as pan, phase, and internal EQ routing still
exist in the DSP/API layer, but they are not shown in the WebCLAP synth UI.

### EQ

The EQ has three serial Butterworth bands:

- Low
- Mid
- High

Each band exposes:

- Enabled
- Type: Low Pass / Band Pass / High Pass
- Frequency
- Gain dB
- Q

Bands default to disabled, so the EQ starts as pass-through. In the WebCLAP EQ
UI, editing Frequency, Type, Gain, or Q automatically enables that band.

## Test

Root workspace:

```powershell
cargo fmt --all
cargo test --workspace
cargo check --target wasm32-unknown-unknown `
  -p z-audio-webclap `
  -p z-audio-webclap-eq `
  -p z-audio-webclap-piano `
  -p z-audio-webclap-reverb `
  -p z-audio-webclap-limiter `
  -p z-audio-webclap-compressor
cargo build --release --target wasm32-unknown-unknown `
  -p z-audio-webclap `
  -p z-audio-webclap-eq `
  -p z-audio-webclap-piano `
  -p z-audio-webclap-reverb `
  -p z-audio-webclap-limiter `
  -p z-audio-webclap-compressor
```

DSP submodule:

```powershell
cd thirdparty/z-audio-dsp
cargo fmt --all
cargo test --workspace
cd ../..
```

UI syntax checks:

```powershell
node --check crates/z-audio-webclap/ui/main.js
node --check crates/z-audio-webclap-eq/ui/main.js
node --check crates/z-audio-webclap-reverb/ui/main.js
node --check crates/z-audio-webclap-limiter/ui/main.js
node --check crates/z-audio-webclap-compressor/ui/main.js
```

Packaging smoke checks:

```powershell
cargo xtask bundle-webclap --release
tar -tf target/webclap/z-audio-simple-synth.wclap.tar.gz
tar -tf target/webclap/z-audio-simple-eq.wclap.tar.gz
tar -tf target/webclap/z-audio-formula-piano.wclap.tar.gz
tar -tf target/webclap/z-audio-parametric-reverb.wclap.tar.gz
tar -tf target/webclap/z-audio-limiter.wclap.tar.gz
tar -tf target/webclap/z-audio-compressor.wclap.tar.gz
```

## Submodule Workflow

`thirdparty/z-audio-dsp` is a git submodule. If a change belongs to the DSP or
synth library API/behavior, commit and push it inside the submodule first. Then
commit the updated submodule pointer in this parent repository.

Typical flow:

```powershell
cd thirdparty/z-audio-dsp
cargo test --workspace
git add .
git commit -m "..."
git push origin main
cd ../..
git add thirdparty/z-audio-dsp
git commit -m "Update z-audio-dsp submodule"
```

Do not edit files under `target/webclap/...` directly. Edit source UI files
under `crates/z-audio-webclap*/ui/`, then regenerate bundles:

```powershell
cargo xtask bundle-webclap --release
```
