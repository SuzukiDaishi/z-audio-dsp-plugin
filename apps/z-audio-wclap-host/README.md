# Z Audio WebCLAP Host

Local browser host for testing Z Audio WebCLAP bundles. It is modeled after
`wclap.plinken.org`: load a `.wclap.tar.gz`, choose a source, and run audio
through a small chain that carries both audio and MIDI event flow.

## Run

From the workspace root:

```powershell
cargo xtask bundle-webclap --release
python apps/z-audio-wclap-host/server.py 8765
```

Open:

```text
http://127.0.0.1:8765/apps/z-audio-wclap-host/
```

Drop any `target/webclap/*.wclap.tar.gz` file onto a slot. The shelf is populated
from `target/webclap/*.wclap.tar.gz` when the local server is running, with a
built-in fallback for the standard Z Audio bundles. The same 4-slot chain is used
for Audio -> Audio, MIDI -> MIDI, MIDI -> Audio, and Audio -> MIDI plugins,
matching the way VST effects and instruments are usually kept in one plugin
chain.

## Slot Controls

- `ui`: opens the plugin WebCLAP UI in its own browser window, sized from the
  bundle's `plugin.json` `ui` block (`expanded_size`, falling back to
  `compact_size`, then to measuring the UI document). Click again to close.
  Each slot can keep its own UI window open at the same time. Allow popups
  for the host origin if the browser blocks the window.
- `auto`: opens generated parameter sliders.
- `save` / `load`: copies plugin state to clipboard as base64, then restores it.
- `on` / `byp`: toggles slot bypass.
- `0smp`: shows plugin latency samples and refreshes it when the runtime exposes
  a latency API.
- `remove`: the `✕` slot button removes the plugin and returns the slot to
  pass-through.

## Audio Sources

- `Tone` can generate sine, triangle, white noise, pink noise, or brown noise
  routed through the chain.
- `Audio File` accepts browser-decodable audio files such as WAV, MP3, M4A,
  OGG, and WebM. Loaded files are connected to the same WebCLAP FX chain.
- `MIDI In` uses Web MIDI to send note on/off events into the first active MIDI
  target in the chain.

## Runtime Attribution

The files under `clap-audionode/` and `page-proxy-service-worker.js` are copied
from `thirdparty/browser-test-host`, which is the WebCLAP browser test host from
the WebCLAP/Signalsmith ecosystem. They provide the `ClapAudioNode`,
AudioWorklet processor, `host.wasm`, WCLAP loader, and iframe resource proxy used
by this local host.
