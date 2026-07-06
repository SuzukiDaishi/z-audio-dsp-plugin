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

The plugins with a custom UI (synth, EQ, reverb, diffuser, limiter,
compressor, VCSL piano, sampler) ship these paths at archive root:

```text
module.wasm
plugin.json
ui/index.html
ui/main.js
ui/styles.css
ui/zui.js        (shared Z Audio UI kit: transport + controls + canvas)
```

All UIs share one design system (`ui/zui.js` + `ui/styles.css`, copied
into each bundle with a per-plugin accent color) and put an interactive,
plugin-specific visualization front and center:

- **Simple Synth** — live scopes for the oscillator shape, amp envelope,
  and LFO that track the controls.
- **Simple EQ** — a log-frequency response editor with draggable band
  nodes (drag = freq/gain, wheel = Q, double-click = band on/off); the
  plotted curves are exact RBJ biquad responses.
- **Compressor** — a soft-knee transfer curve you can drag (threshold /
  ratio) and wheel (knee), with gain-reduction shading.
- **Limiter** — a brickwall transfer curve with draggable threshold and
  ceiling.
- **Parametric Reverb** — a stylized impulse response showing pre-delay,
  early reflections, decay tail, damping, and width; drag the tail to
  edit decay/damping.
- **Diffuser** — an echo-density cloud (one dot per emerging echo);
  drag to reshape size/diffusion.
- **VCSL Piano** — a draggable velocity→loudness response curve.
- **Sampler** — see the sampler section above.

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
- **Slice** — auto-cut at detected onsets (sensitivity control) or an
  equal grid (4/8/16/32), one key per slice from a base key up; markers can
  be added (double-click), removed, and dragged. Up to 128 zones.

Slice-point estimation (`ui/onsets.js`) uses the standard spectral-flux
recipe rather than a plain level detector: STFT (Hann 1024 / hop 256) →
log-compressed magnitudes → half-wave-rectified flux → adaptive
median-plus-floor threshold → peak picking with a minimum inter-onset gap
→ sample-accurate refinement to the attack start with a declick snap. It
therefore also catches pitch/timbre changes that have no level dip, scales
its floor to the trimmed region's strongest hit, and stays quiet on steady
material. The expensive curve is cached per file, so the sensitivity
slider re-picks in real time.

Global parameters (automatable): master gain, ADSR, tune, transpose,
velocity sensitivity, stereo width. A small embedded piano preview bank is
mapped as one Classic zone at startup, so the instrument makes sound before
any file is loaded. Samples up to 60 seconds (stereo, source rate preserved
as metadata) are accepted; longer files are truncated by the UI.

The UI <-> plugin binary protocol is documented in
`crates/z-audio-webclap-sampler/src/protocol.rs`. Sample PCM is not stored
in host projects yet; the generic `clap.state` blob persists parameters
only, so reload the file after reopening a project.

## Native VST3/CLAP Webview Editors

On Windows and macOS the native VST3/CLAP builds of the synth, EQ, reverb,
diffuser, limiter, compressor, and VCSL piano open the *same* web UI as
their WebCLAP builds, rendered in a [wry](https://github.com/tauri-apps/wry)
webview (the engine Tauri uses) embedded in the host's plugin window:

- `crates/nih-plug-webview/` — vendored fork (ISC) of
  [nih-plug-webview](https://github.com/httnn/nih-plug-webview), pinned to
  this workspace's `nih_plug`/`baseview` revisions (see its `NOTICE.md`).
- `crates/z-audio-webview-editor/` — inlines each WebCLAP `ui/` bundle
  into one self-contained HTML page at compile time and bridges the UI's
  numeric param ids to `nih_plug` `ParamPtr`s over a JSON IPC protocol
  (`{"type":"set"|"ready"|"params",…}`). The UI kit's `connect()` detects
  the wry bridge at runtime, so one UI source serves both plugin formats.
  Host-side automation and preset loads are pushed back to the UI by a
  per-frame diff of parameter values.

On Linux, wry cannot embed a webview into a host-owned plugin window, so
the native plugins keep their egui editors (the VCSL piano, which never
had one, keeps host-generated controls). The WebCLAP builds are unaffected
everywhere. Note that the reverb's Mod Rate/Depth controls exist only in
the WebCLAP DSP, so in the native webview UI those two sliders are
inactive.

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
node --test crates/z-audio-webclap-sampler/ui/onsets.test.mjs
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
