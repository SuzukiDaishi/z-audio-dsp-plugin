# Z Audio DSP Plugin

Audio plugin wrappers and packaging for `z-audio-dsp`.

This workspace builds native and WebCLAP wrappers for:

- `Z Audio Simple Synth`: MIDI note input to stereo audio output
- `Z Audio Simple EQ`: mono/stereo audio input to audio output
- `Z Audio Formula Piano`: modal/formula piano instrument
- `Z Audio VCSL Piano`: sampler piano built from VCSL Keys "Grand Piano, K"
- `Z Audio Sampler`: multi-zone WebCLAP sampler with GUI file loading and
  auto-slicing (see below)
- `Z Audio Formula Drum Set`: modal/formula GM drum set instrument
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
  z-audio-plugin/             Native VST3/CLAP synth
  z-audio-eq-plugin/          Native VST3/CLAP EQ
  z-audio-vcsl-piano-plugin/  Native VST3/CLAP VCSL sampler piano
  z-audio-*-plugin/           Native VST3/CLAP piano, drums, reverb, limiter, compressor
  z-audio-webclap/            WebCLAP synth wasm + UI
  z-audio-webclap-eq/         WebCLAP EQ wasm + UI
  z-audio-webclap-vcsl-piano/ WebCLAP VCSL sampler piano wasm + UI
  z-audio-webclap-*/          WebCLAP piano, drums, reverb, limiter, compressor wasm
  wclap-plugin/               Minimal WebCLAP/CLAP runtime glue
  xtask/                      Packaging tasks (incl. `prepare-vcsl-piano`)
thirdparty/
  z-audio-dsp/                DSP and synth library submodule
assets/
  vcsl-piano/                 Generated VCSL sampler banks (see Licensing below)
```

Generated artifacts are written under `target/` and are not tracked.

## Assets & Licensing

`Z Audio VCSL Piano` is built from [VCSL Keys](https://versilian-studios.com/vcsl-keys/)
("Grand Piano, K"), which Versilian Studios LLC releases under
[CC0](https://creativecommons.org/publicdomain/zero/1.0/) (public domain).

- `docs/VCSL_Keys.zip` (the ~650MB source SFZ/FLAC archive) is **not** committed
  to this repository; it's gitignored. Download it yourself from the VCSL Keys
  page above and place it at `docs/VCSL_Keys.zip`.
- `assets/vcsl-piano/grand-piano-k.bank` (the full sampler bank, ~450MB, used by
  the native VST3/CLAP plugin) is generated locally and also gitignored.
- `assets/vcsl-piano/grand-piano-k-dev.bank` (a small ~2.7MB preview bank — six
  notes, mono, truncated — embedded in the WebCLAP build) **is** committed,
  since `z-audio-webclap-vcsl-piano` needs it at compile time.

Regenerate both banks from the source archive with:

```powershell
cargo xtask prepare-vcsl-piano
```

See `docs/VCSLサンプラーピアノ実装計画.md` for the full implementation plan and
the SFZ opcode subset that's currently supported.

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
cargo xtask bundle z-audio-vcsl-piano-plugin --release
cargo xtask bundle z-audio-drums-plugin --release
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
target/bundled/z-audio-vcsl-piano-plugin.vst3
target/bundled/z-audio-vcsl-piano-plugin.clap
target/bundled/Z Audio Formula Drum Set.vst3
target/bundled/Z Audio Formula Drum Set.clap
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
target/webclap/z-audio-vcsl-piano.wclap/
target/webclap/z-audio-vcsl-piano.wclap.tar.gz
target/webclap/z-audio-formula-drums.wclap/
target/webclap/z-audio-formula-drums.wclap.tar.gz
target/webclap/z-audio-parametric-reverb.wclap/
target/webclap/z-audio-parametric-reverb.wclap.tar.gz
target/webclap/z-audio-limiter.wclap/
target/webclap/z-audio-limiter.wclap.tar.gz
target/webclap/z-audio-compressor.wclap/
target/webclap/z-audio-compressor.wclap.tar.gz
```

Use the `.wclap.tar.gz` files when testing with WebCLAP hosts such as Plinken.

Run the local first-party WebCLAP host:

```powershell
python apps/z-audio-wclap-host/server.py 8765
```

Then open `http://127.0.0.1:8765/apps/z-audio-wclap-host/` and drop a
`target/webclap/*.wclap.tar.gz` bundle onto a chain slot. The host keeps
Audio -> Audio, MIDI -> MIDI, MIDI -> Audio, and Audio -> MIDI plugins in the
same 4-slot WebCLAP chain, so instruments and effects can be tested together.
The built-in source can generate sine, triangle, white noise, pink noise, or
brown noise, or play a browser-decodable audio file through the chain. Web MIDI
input is available from the source panel when the browser and device allow it.

The synth, EQ, reverb, limiter, and compressor tarballs contain these paths at
archive root:

```text
module.wasm
plugin.json
ui/index.html
ui/main.js
ui/styles.css
```

The piano and drum WebCLAP bundles currently expose host parameters without a
custom WebCLAP UI, so their tarballs contain `module.wasm` and `plugin.json`.

## Z Audio Sampler (WebCLAP)

`z-audio-webclap-sampler` is a Logic Quick Sampler-style instrument: load an
audio file from the plugin GUI, and it is decoded in the WebView with
`decodeAudioData`, streamed to the wasm plugin in 128 KiB binary chunks over
`clap.webview/3`, and cut into key-mapped zones. All heavy work (decode,
upload assembly, zone cutting) happens outside `process()`; the audio path
only reads prepared `SampleRegion`s.

Modes (chosen in the UI, mapped to a zone table the engine plays as-is):

- **Classic** — the whole (trimmed) sample mapped chromatically around a
  root key, with loop modes Off / Forward / Sustain / Ping-Pong / Reverse,
  draggable trim + loop markers, and loop crossfade.
- **One Shot** — plays through per note and ignores note-off.
- **Slice** — auto-cut at detected transients (sensitivity control) or an
  equal grid (4/8/16/32), one key per slice from a base key up; markers can
  be added (double-click), removed, and dragged. Up to 128 zones.

Global parameters (automatable): master gain, ADSR, tune, transpose,
velocity sensitivity, stereo width. A small embedded piano preview bank is
mapped as one Classic zone at startup, so the instrument makes sound before
any file is loaded. Samples up to 60 seconds (stereo, source rate preserved
as metadata) are accepted; longer files are truncated by the UI.

The UI <-> plugin binary protocol is documented in
`crates/z-audio-webclap-sampler/src/protocol.rs`. Sample PCM is not stored
in host projects yet; the generic `clap.state` blob persists parameters
only, so reload the file after reopening a project.

## Plugin IDs

| Plugin | CLAP ID | VST3 Class ID | WebCLAP bundle |
| --- | --- | --- | --- |
| Z Audio Simple Synth | `dev.zaudio.simple-synth` | `ZAudioSmplSynth1` | `z-audio-simple-synth.wclap.tar.gz` |
| Z Audio Simple EQ | `dev.zaudio.simple-eq` | `ZAudioSimpleEQ01` | `z-audio-simple-eq.wclap.tar.gz` |
| Z Audio Formula Piano | `dev.zaudio.formula-piano` | `ZAudioFormulaPno` | `z-audio-formula-piano.wclap.tar.gz` |
| Z Audio VCSL Piano | `dev.zaudio.vcsl-piano` | `ZAudioVCSLPiano1` | `z-audio-vcsl-piano.wclap.tar.gz` |
| Z Audio Formula Drum Set | `dev.zaudio.formula-drums` | `ZAudioDrumSet001` | `z-audio-formula-drums.wclap.tar.gz` |
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

The EQ has three serial EQ bands:

- Low
- Mid
- High

Each band exposes:

- Enabled
- Type: Low Shelf / Bell / High Shelf / High Pass / Low Pass
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
  -p z-audio-webclap-drums `
  -p z-audio-webclap-reverb `
  -p z-audio-webclap-limiter `
  -p z-audio-webclap-compressor
cargo build --release --target wasm32-unknown-unknown `
  -p z-audio-webclap `
  -p z-audio-webclap-eq `
  -p z-audio-webclap-piano `
  -p z-audio-webclap-drums `
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
node --check crates/z-audio-webclap-sampler/ui/main.js
```

Packaging smoke checks:

```powershell
cargo xtask bundle-webclap --release
tar -tf target/webclap/z-audio-simple-synth.wclap.tar.gz
tar -tf target/webclap/z-audio-simple-eq.wclap.tar.gz
tar -tf target/webclap/z-audio-formula-piano.wclap.tar.gz
tar -tf target/webclap/z-audio-formula-drums.wclap.tar.gz
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
