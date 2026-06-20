This crate is vendored from [`taluvi-dev/plinken-org`](https://github.com/taluvi-dev/plinken-org)
(`crates/wclap-plugin`), MIT-licensed (see `LICENSE`). It provides the CLAP/WCLAP ABI scaffold
(`clap_entry`, factory, plugin vtable, audio-ports/note-ports/params/state/webview extensions)
that `wclap.plinken.org` expects a `wasm32-unknown-unknown` cdylib plugin to export.

Local additions on top of upstream:

- `ProcessCtx::note_events()` — reads `clap_event_note` (NOTE_ON/NOTE_OFF) from the process
  struct's `in_events` list, which upstream did not yet expose (their own Rust plugins are all
  audio effects; `synome`, their one instrument attempt, is an explicit "Phase A scaffold —
  silent" stub that never reads notes).
- `plugin_process` now applies `clap_event_param_value` (type 5) events from `in_events` to
  `Plugin::set_param` before calling the plugin's own `process()`. Upstream only ever applies
  host/UI-driven parameter changes through `clap_host_webview.send`/`receive` (every one of their
  own plugins ships a custom webview UI); `clap.params.flush` itself is — and remains — a no-op.
  A plugin with no UI (`has_ui: false`), or any host driving `clap.params` the standard CLAP way
  via input events, had no working path to `set_param` at all without this; confirmed by measuring
  zero effect from `pluginSetParam` calls before this fix, and a correctly filtered signal after.
