# References

## z-audio-dsp (DSP core dependency)

- GitHub:
  - https://github.com/SuzukiDaishi/z-audio-dsp.git

Notes:

- Cargo workspace with `crates/z-audio-dsp` (pure DSP: generators/modulators/effects/params), `crates/z-audio-synth` (`SimpleSynth` runtime), `crates/z-audio-examples`.
- このプラグインcrateはgit dependencyとして参照します（パッケージ名指定でcargoがリポジトリ内を解決します）。

```toml
[dependencies]
z-audio-synth = { git = "https://github.com/SuzukiDaishi/z-audio-dsp.git" }
z-audio-dsp = { git = "https://github.com/SuzukiDaishi/z-audio-dsp.git" }
```

- 本家にも `docs/z-audio-dsp_dsp_plan/` というDSP側計画ドキュメントがあり、本ZIPの計画はそれに対するplugin/web側の続きという位置づけです。
- 本家roadmap (`11_final_goal_roadmap.md`) の `v0.2: Plugin-ready Synth Core`（parameter metadata / 安定process API / voice stealing / automation smoothing）は2026-06-14時点で **実装済み**。`01_wrapper_architecture.md` / `02_vst3_clap_nih_plug_plan.md` / `05_parameters_automation_presets.md` は実際のAPI（`SimpleSynth::new` / `process_with_context` / `ParamId::ALL` 27種 / `ParamId::metadata()`）に合わせて記述しています。

## NIH-plug

- GitHub:
  - https://github.com/robbert-vdh/nih-plug

Notes:

- Rust VST3/CLAP plugin framework
- 本家READMEではframeworkがmaintenance modeと記載されているため、長期的には依存戦略を再検討する

## CLAP

- GitHub:
  - https://github.com/free-audio/clap

Notes:

- CLever Audio Plugin
- stable ABI
- host/plugin communication via C interfaces
- extensions-based design

## VST3 SDK

- GitHub:
  - https://github.com/steinbergmedia/vst3sdk

Notes:

- VST3 plugin/host SDK
- block processing
- sample-accurate automation
- separation of UI and processing

## WebCLAP

`github.com/WebCLAP` organization repos:

- https://github.com/WebCLAP/wclap-host-js — "JS / C++ library for managing WCLAPs in the browser"
- https://github.com/WebCLAP/wclap-host-cpp — C++ headers/helpers for using WCLAP modules
- https://github.com/WebCLAP/browser-test-host — browser-based host PoC, Emscripten-backed C++
- https://github.com/WebCLAP/as-clap — "WCLAP written in AssemblyScript"
- https://github.com/WebCLAP/examples — test/demo WCLAPs across toolchains: `clack` (Rust,
  `clack-plugin` crate — `clack_plugin_gain.wasm`/`clack_plugin_polysynth.wasm`), `as-clap`,
  `signalsmith-basics`, `signalsmith-clap-cpp`
- https://github.com/WebCLAP/wasi.wasm — WASI functions implemented as a WASM module

Notes:

- Browser-based host for CLAP plugins compiled to WASM; AudioWorklet-backed AudioNode wrapper.
- A `.wclap` bundle is a `.tar.gz` (manifest + wasm + optional UI assets); a bare instrument/effect
  with no UI can ship as a plain `<name>.wasm` instead.
- `clack-plugin` (the Rust crate from [`prokopyl/clack`](https://github.com/prokopyl/clack)) builds
  fine for `wasm32-unknown-unknown` as a `cdylib` — contrary to what we assumed in
  [`04_webclap_plan.md`](04_webclap_plan.md)'s 2026-06-14 entry.

### `taluvi-dev/plinken-org` — the site this project targets (`wclap.plinken.org`)

- https://github.com/taluvi-dev/plinken-org
- Hosts a WCLAP plugin shelf at `wclap.plinken.org` (`apps/wclap-host`), built on a Rust port of
  the WebCLAP host (`crates/wclap-host`, replaces the upstream C++ `host.wasm`) plus the upstream
  `vendor/wclap-host-js` JS glue (git submodule).
- Ships its own shared Rust plugin scaffold, `crates/wclap-plugin` (MIT) — hand-rolled CLAP ABI
  glue (`clap_entry`, factory, plugin vtable, audio-ports/note-ports/params/state/webview
  extensions) for `#![no_std] + alloc` `cdylib`s, lighter than `clack`. We vendor this into
  [`crates/wclap-plugin`](../../crates/wclap-plugin) (see its `NOTICE.md` for our local additions —
  note-event reading, which upstream's own plugins didn't need yet).
- Vendor plugin contribution convention: `plugins/<reverse-dns-vendor>/<plugin-name>/plugin.json`
  (see `plugins/README.md` in that repo) — mirrored by our
  [`crates/z-audio-webclap/plugin.json`](../../crates/z-audio-webclap/plugin.json).

## Emscripten Wasm AudioWorklets

- Documentation:
  - https://emscripten.org/docs/api_reference/wasm_audio_worklets.html

Notes:

- WASM AudioWorklet can run custom audio processors
- Real-time audio callback constraints apply
- Web Audio render quantum is typically 128 samples
