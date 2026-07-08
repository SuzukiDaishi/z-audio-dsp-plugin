# Z Audio DSP Plugin

Audio plugin wrappers and packaging for `z-audio-dsp`.

This workspace builds native and WebCLAP wrappers for:

- `Z Audio Simple Synth`: MIDI note input to stereo audio output
- `Z Audio EQ`: Pro-Q-style 8-band parametric EQ (bell/shelf/cut/notch,
  6-48 dB/oct slopes, per-band Stereo/Mid/Side/L/R placement, band-solo
  listen, pre/post spectrum analyzer) as WebCLAP; the native VST3/CLAP
  build remains the original 3-band Simple EQ with its own UI snapshot
- `Z Audio Formula Piano`: modal/formula piano instrument
- `Z Audio VCSL Piano`: sampler piano built from VCSL Keys "Grand Piano, K"
- `Z Audio Sampler`: multi-zone sampler with GUI file loading and
  auto-slicing, as WebCLAP and native VST3/CLAP (see below)
- `Z Audio Formula Drum Set`: modal/formula GM drum set instrument
- `Z Audio Wave Synth`: Serum-inspired wavetable synth (2 morphing
  oscillators with unison, SVF filter, 2 envelopes, 2 LFOs, mod matrix),
  WebCLAP only for now
- `Z Audio Parametric Reverb`: stereo FDN reverb effect
- `Z Audio Limiter`: stereo lookahead limiter effect
- `Z Audio Compressor`: stereo feed-forward compressor effect
- `Z Audio Ring Mod`: ring modulator with a sine/tri/saw/square carrier,
  WebCLAP only
- `Z Audio Distortion`: waveshaping distortion (soft/hard/fold/asym) with
  tone control, WebCLAP only
- `Z Audio Saturator`: warm level-compensated saturation with tilt tone,
  WebCLAP only
- `Z Audio Bitcrusher`: bit-depth + sample-rate reduction, WebCLAP only
- `Z Audio Delay`: stereo delay with ping-pong and feedback damping,
  WebCLAP only
- `Z Audio Chorus`: multi-voice stereo chorus, WebCLAP only
- `Z Audio Flanger`: stereo flanger with bipolar feedback, WebCLAP only
- `Z Audio Phaser`: 2-12 stage stereo phaser, WebCLAP only
- `Z Audio Tremolo`: tremolo / auto-pan with stereo LFO phase, WebCLAP only
- `Z Audio Gate`: noise gate with hold and range, WebCLAP only

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
cargo xtask bundle z-audio-sampler-plugin --release
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
target/bundled/Z Audio Sampler.vst3
target/bundled/Z Audio Sampler.clap
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
target/webclap/z-audio-wavetable.wclap/
target/webclap/z-audio-wavetable.wclap.tar.gz
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
- **EQ** — a Pro-Q-style editor: real-time pre/post spectrum analyzer
  behind the summed curve, double-click to add one of 8 bands, drag its
  colored dot (freq/gain), wheel for Q, double-click the dot to remove.
  The panel picks the band type (bell / lo-hi shelf / lo-hi cut / notch),
  cut slope (6/12/24/48 dB/oct), Stereo/Mid/Side/L/R placement, and a
  SOLO that plays only that band's region. All plotted curves are the
  exact RBJ responses the wasm engine runs.
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
- **Ring Mod** — the carrier multiplied against a reference sine; drag
  for carrier frequency (←→) and mix (↑↓).
- **Distortion** — the exact waveshaper transfer curve (soft / hard /
  fold / asym); drag vertically for drive.
- **Saturator** — the level-compensated saturation curve; drag for drive
  (↑↓) and warmth/asymmetry (←→).
- **Bitcrusher** — a reference sine run through the exact quantize +
  sample-and-hold; drag for downsample (←→) and bit depth (↑↓).
- **Delay** — a decaying echo-tap timeline for L/R; drag for time (←→)
  and feedback (↑↓).
- **Chorus** — the per-voice delay-modulation LFO curves; drag for rate
  (←→) and depth (↑↓).
- **Flanger** — the comb-filter response at the sweep center; drag for
  manual delay (←→) and bipolar feedback (↑↓).
- **Phaser** — the notch response of the allpass cascade; drag for
  center (←→) and depth (↑↓).
- **Tremolo** — the L/R gain LFOs over one cycle; drag for rate (←→)
  and depth (↑↓).
- **Gate** — the input/output transfer curve with the threshold line;
  drag for threshold (←→) and range (↑↓).
- **Wave Synth** — Serum-style panels built from rotary knobs (vertical
  drag, Shift = fine, wheel, double-click = default) and directly
  editable canvases: each oscillator draws a pseudo-3D stack of its
  table's frames with the live morphed cycle riding at the current WT
  position (drag the canvas to morph), the filter response is draggable
  (cutoff/resonance, wheel = reso), the ADSR curves have three grab
  handles, and a preview keyboard at the bottom plays the synth without
  a MIDI device. Waveforms and meters are pushed from the plugin, so
  the canvases show exactly what the DSP plays.

The piano and drum WebCLAP bundles currently expose host parameters without a
custom WebCLAP UI, so their tarballs contain `module.wasm` and `plugin.json`.

## Z Audio Sampler

`z-audio-webclap-sampler` is a Logic Quick Sampler-style instrument: load an
audio file from the plugin GUI, and it is decoded in the WebView with
`decodeAudioData`, streamed to the wasm plugin in 128 KiB binary chunks over
`clap.webview/3`, and cut into key-mapped zones. All heavy work (decode,
upload assembly, zone cutting) happens outside `process()`; the audio path
only reads prepared `SampleRegion`s.

`z-audio-sampler-plugin` is the native VST3/CLAP build of the same
instrument. It links the WebCLAP crate as a library (same `ZoneSampler`
engine, same `ZSMP` protocol) and, on Windows/macOS, opens the identical
web UI in the wry webview; there the `ZSMP` packets ride a
`{"type":"bin","data":"<base64>"}` JSON envelope over the wry IPC bridge
instead of raw `clap.webview/3` buffers (smaller upload chunks, same
packets — see `crates/z-audio-webview-editor`). On other platforms it
falls back to a reduced egui editor that decodes WAV/FLAC natively and
maps the file as a single Classic zone (no trim/loop/slice editing).

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
in host projects yet; both the WebCLAP and native builds persist
parameters only, so reload the file after reopening a project.

## Native VST3/CLAP Webview Editors

On Windows and macOS the native VST3/CLAP builds of the synth, EQ, reverb,
diffuser, limiter, compressor, VCSL piano, and sampler open the *same* web
UI as their WebCLAP builds, rendered in a
[wry](https://github.com/tauri-apps/wry) webview (the engine Tauri uses)
embedded in the host's plugin window:

- `crates/nih-plug-webview/` — vendored fork (ISC) of
  [nih-plug-webview](https://github.com/httnn/nih-plug-webview), pinned to
  this workspace's `nih_plug`/`baseview` revisions (see its `NOTICE.md`).
- `crates/z-audio-webview-editor/` — inlines each WebCLAP `ui/` bundle
  into one self-contained HTML page at compile time and bridges the UI's
  numeric param ids to `nih_plug` `ParamPtr`s over a JSON IPC protocol
  (`{"type":"set"|"ready"|"params",…}`). The UI kit's `connect()` detects
  the wry bridge at runtime, so one UI source serves both plugin formats.
  Host-side automation and preset loads are pushed back to the UI by a
  per-frame diff of parameter values. UIs with a binary protocol (the
  sampler's `ZSMP` packets) additionally exchange
  `{"type":"bin","data":"<base64>"}` messages, dispatched to a plugin
  callback via `create_webview_editor_with_messages`.

On Linux, wry cannot embed a webview into a host-owned plugin window, so
the native plugins keep their egui editors (the VCSL piano, which never
had one, keeps host-generated controls; the sampler gets a reduced egui
editor with native WAV/FLAC loading as one Classic zone). The WebCLAP
builds are unaffected everywhere. Note that the reverb's Mod Rate/Depth
controls exist only in the WebCLAP DSP, so in the native webview UI those
two sliders are inactive.

## Plugin IDs

| Plugin | CLAP ID | VST3 Class ID | WebCLAP bundle |
| --- | --- | --- | --- |
| Z Audio Simple Synth | `dev.zaudio.simple-synth` | `ZAudioSmplSynth1` | `z-audio-simple-synth.wclap.tar.gz` |
| Z Audio EQ (WebCLAP) / Simple EQ (native) | `dev.zaudio.simple-eq` | `ZAudioSimpleEQ01` | `z-audio-simple-eq.wclap.tar.gz` |
| Z Audio Formula Piano | `dev.zaudio.formula-piano` | `ZAudioFormulaPno` | `z-audio-formula-piano.wclap.tar.gz` |
| Z Audio VCSL Piano | `dev.zaudio.vcsl-piano` | `ZAudioVCSLPiano1` | `z-audio-vcsl-piano.wclap.tar.gz` |
| Z Audio Sampler | `dev.zaudio.sampler` | `ZAudioSamplerMZ1` | `z-audio-sampler.wclap.tar.gz` |
| Z Audio Formula Drum Set | `dev.zaudio.formula-drums` | `ZAudioDrumSet001` | `z-audio-formula-drums.wclap.tar.gz` |
| Z Audio Wave Synth | `dev.zaudio.wavetable` | — (WebCLAP only) | `z-audio-wavetable.wclap.tar.gz` |
| Z Audio Parametric Reverb | `dev.zaudio.parametric-reverb` | `ZAudioParaReverb` | `z-audio-parametric-reverb.wclap.tar.gz` |
| Z Audio Limiter | `dev.zaudio.limiter` | `ZAudioLimiter000` | `z-audio-limiter.wclap.tar.gz` |
| Z Audio Compressor | `dev.zaudio.compressor` | `ZAudioCompressor` | `z-audio-compressor.wclap.tar.gz` |
| Z Audio Ring Mod | `dev.zaudio.ringmod` | — (WebCLAP only) | `z-audio-ring-mod.wclap.tar.gz` |
| Z Audio Distortion | `dev.zaudio.distortion` | — (WebCLAP only) | `z-audio-distortion.wclap.tar.gz` |
| Z Audio Saturator | `dev.zaudio.saturator` | — (WebCLAP only) | `z-audio-saturator.wclap.tar.gz` |
| Z Audio Bitcrusher | `dev.zaudio.bitcrusher` | — (WebCLAP only) | `z-audio-bitcrusher.wclap.tar.gz` |
| Z Audio Delay | `dev.zaudio.delay` | — (WebCLAP only) | `z-audio-delay.wclap.tar.gz` |
| Z Audio Chorus | `dev.zaudio.chorus` | — (WebCLAP only) | `z-audio-chorus.wclap.tar.gz` |
| Z Audio Flanger | `dev.zaudio.flanger` | — (WebCLAP only) | `z-audio-flanger.wclap.tar.gz` |
| Z Audio Phaser | `dev.zaudio.phaser` | — (WebCLAP only) | `z-audio-phaser.wclap.tar.gz` |
| Z Audio Tremolo | `dev.zaudio.tremolo` | — (WebCLAP only) | `z-audio-tremolo.wclap.tar.gz` |
| Z Audio Gate | `dev.zaudio.gate` | — (WebCLAP only) | `z-audio-gate.wclap.tar.gz` |

## Parameters

### Synth WebCLAP UI

The synth WebCLAP UI intentionally exposes only the main instrument controls:

- Oscillator: shape, level, pulse width
- Amp envelope: attack, decay, sustain, release, curve
- LFO: waveform, rate, depth, route
- Output: master

Lower-level synth parameters such as pan, phase, and internal EQ routing still
exist in the DSP/API layer, but they are not shown in the WebCLAP synth UI.

### Wave Synth

`z-audio-webclap-wavetable` is a Serum-inspired wavetable synth (WebCLAP
only for now; the crate builds as an rlib so a native VST3/CLAP wrapper
can reuse the engine later). Web param ids are the 500 block:

- Oscillators A/B: enable, factory table (Basic Shapes / PWM / Harmonic
  Sweep / Metal Bell), wavetable position (frame morph), octave/semi/fine,
  unison 1-8 with detune + blend, start phase, random phase, pan, level
- Filter: LP12/LP24/HP12/BP12 state-variable filter with cutoff,
  resonance, drive, key tracking, dry/wet mix, and per-oscillator routing
- Env 1 (amp) and Env 2: ADSR plus a shared curve control
- LFO 1/2: sine/tri/saw/square/S&H, 0.01-20 Hz, start phase, retrigger
- Mod matrix: 8 slots of source (Env 1/2, LFO 1/2, velocity, note) →
  destination (WT pos / pitch / level / pan per osc, cutoff, resonance,
  master) → bipolar amount; every slot field is host-automatable.
  Assignments are made Serum-style in the UI: drag a source chip
  (ENV 1/2, LFO 1/2, VEL, NOTE) onto a knob or canvas, drag the colored
  ring around a modulated knob to set the depth, and double-click the
  ring to remove the connection. The matrix list mirrors the same slots
  for fine editing.
- Global: master, polyphony (1-16), pitch-bend range (declared; WebCLAP
  hosts don't deliver bend events yet), glide

Wavetables are generated at activation by additive synthesis into 11
band-limited mip levels per frame (harmonics halve per level), and the
oscillator picks the mip whose full band stays below Nyquist for the
current pitch — wavetable playback stays alias-free across the key range.

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
  -p z-audio-webclap-compressor `
  -p z-audio-webclap-sampler `
  -p z-audio-webclap-wavetable
cargo build --release --target wasm32-unknown-unknown `
  -p z-audio-webclap `
  -p z-audio-webclap-eq `
  -p z-audio-webclap-piano `
  -p z-audio-webclap-drums `
  -p z-audio-webclap-reverb `
  -p z-audio-webclap-limiter `
  -p z-audio-webclap-compressor `
  -p z-audio-webclap-sampler `
  -p z-audio-webclap-wavetable
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
node --check crates/z-audio-webclap-wavetable/ui/main.js
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
tar -tf target/webclap/z-audio-sampler.wclap.tar.gz
tar -tf target/webclap/z-audio-wavetable.wclap.tar.gz
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
