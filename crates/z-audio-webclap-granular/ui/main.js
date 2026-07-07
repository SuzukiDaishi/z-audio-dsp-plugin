// Z Audio Granular UI — Phase Plant-style granular synth front end.
//
// Load an audio file (click / drag & drop), decode it in the GUI with
// decodeAudioData, and stream the PCM to the plugin over the
// clap.webview/3 binary channel:
//
//   ZGRN 0x01 BeginSample   f32 rate · u8 channels · u32 frames
//   ZGRN 0x02 SampleChunk   u32 floatOffset · f32le PCM payload
//   ZGRN 0x03 CommitSample
//   ZGRN 0x04 NotePreview   u8 on · u8 key · u8 velocity
//   ZGRN 0x05 ClearSample
//   ZGRN 0x06 PollActivity  (native webview polls; WebCLAP receives pushes)
//   ZGRN 0x81 Status        (plugin → UI)
//   ZGRN 0x82 Activity      (plugin → UI: live grain positions)
//
// The waveform doubles as the seek bar: drag it to move Position (web
// param 403, host-automatable), the shaded band shows the Random Position
// spray range, and the dots are live grains.

"use strict";

// ---------------------------------------------------------------------------
// Parameter surface (web ids 400-429, mirrored by both plugin builds).
// ---------------------------------------------------------------------------

const P = {
  LEVEL: 400,
  PITCH: 401,
  FINE: 402,
  POSITION: 403,
  GRAIN_LEN: 404,
  LEN_KEYTRACK: 405,
  G_ATTACK: 406,
  G_DECAY: 407,
  A_CURVE: 408,
  D_CURVE: 409,
  SPAWN_MODE: 410,
  RATE: 411,
  SYNC_RATE: 412,
  DENSITY: 413,
  ROOT: 414,
  ALIGN: 415,
  WARM: 416,
  R_POS: 417,
  R_TIMING: 418,
  R_PITCH: 419,
  R_LEVEL: 420,
  R_PAN: 421,
  R_REVERSE: 422,
  CHORD_TYPE: 423,
  CHORD_RANGE: 424,
  CHORD_PATTERN: 425,
  A_ATTACK: 426,
  A_DECAY: 427,
  A_SUSTAIN: 428,
  A_RELEASE: 429,
};

function fmtSeconds(v) {
  return v < 1 ? `${(v * 1000).toFixed(0)} ms` : `${v.toFixed(2)} s`;
}

function fmtPercent(v) {
  return `${(v * 100).toFixed(0)} %`;
}

function fmtMs(v) {
  return v >= 1000 ? `${(v / 1000).toFixed(2)} s` : `${v.toFixed(0)} ms`;
}

// Sliders rendered into the params grid, grouped by section.
const SLIDERS = [
  { section: "Grain" },
  { id: P.POSITION, label: "Position", min: 0, max: 1, default: 0, fmt: fmtPercent },
  { id: P.GRAIN_LEN, label: "Length", min: 2, max: 1000, default: 100, fmt: fmtMs, curve: 2.5 },
  { id: P.G_ATTACK, label: "Attack", min: 0, max: 1, default: 0.5, fmt: fmtPercent },
  { id: P.G_DECAY, label: "Decay", min: 0, max: 1, default: 0.5, fmt: fmtPercent },
  { id: P.A_CURVE, label: "Atk Curve", min: -1, max: 1, default: 0, fmt: (v) => v.toFixed(2) },
  { id: P.D_CURVE, label: "Dec Curve", min: -1, max: 1, default: 0, fmt: (v) => v.toFixed(2) },
  { section: "Pitch / Level" },
  { id: P.LEVEL, label: "Level", min: 0, max: 2, default: 1, fmt: fmtPercent },
  { id: P.PITCH, label: "Pitch", min: -48, max: 48, default: 0, step: 1, fmt: (v) => `${v > 0 ? "+" : ""}${v.toFixed(0)} st` },
  { id: P.FINE, label: "Fine", min: -100, max: 100, default: 0, fmt: (v) => `${v.toFixed(0)} ct` },
  { section: "Random" },
  { id: P.R_POS, label: "Position", min: 0, max: 2000, default: 0, fmt: fmtMs, curve: 2 },
  { id: P.R_TIMING, label: "Timing", min: 0, max: 1, default: 0, fmt: fmtPercent },
  { id: P.R_PITCH, label: "Pitch", min: 0, max: 24, default: 0, fmt: (v) => `${v.toFixed(1)} st` },
  { id: P.R_LEVEL, label: "Level", min: 0, max: 1, default: 0, fmt: fmtPercent },
  { id: P.R_PAN, label: "Pan", min: 0, max: 1, default: 0, fmt: fmtPercent },
  { id: P.R_REVERSE, label: "Reverse", min: 0, max: 1, default: 0, fmt: fmtPercent },
  { section: "Amp Envelope" },
  { id: P.A_ATTACK, label: "Attack", min: 0.001, max: 5, default: 0.002, fmt: fmtSeconds, curve: 3 },
  { id: P.A_DECAY, label: "Decay", min: 0, max: 5, default: 0, fmt: fmtSeconds, curve: 3 },
  { id: P.A_SUSTAIN, label: "Sustain", min: 0, max: 1, default: 1, fmt: fmtPercent },
  { id: P.A_RELEASE, label: "Release", min: 0.01, max: 10, default: 0.25, fmt: fmtSeconds, curve: 3 },
];

const SYNC_LABELS = [
  "16 beats",
  "8 beats",
  "4 beats",
  "2 beats",
  "1 beat",
  "1/2 beat",
  "1/4 beat",
  "1/8 beat",
  "1/16 beat",
];

const CHORD_TYPES = [
  "Off",
  "Octave",
  "Fifth",
  "Major",
  "Minor",
  "Maj7",
  "Min7",
  "Dom7",
  "Sus2",
  "Sus4",
];

const CHORD_PATTERNS = ["Up", "Down", "Up-Down", "Random"];

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

function noteName(key) {
  return `${NOTE_NAMES[key % 12]}${Math.floor(key / 12) - 1}`;
}

// ---------------------------------------------------------------------------
// Transport.
//
// Two backends, same ZGRN packets:
//  - WebCLAP: raw ArrayBuffers over clap.webview/3 postMessage.
//  - Native VST3/CLAP (Windows/macOS): the wry webview injects
//    sendToPlugin/onPluginMessage (see crates/z-audio-webview-editor);
//    ZGRN packets ride a {"type":"bin","data":<base64>} JSON envelope and
//    params use the same {"ready"|"set"|"params"} JSON as the other UIs.
// ---------------------------------------------------------------------------

const NATIVE = typeof window.sendToPlugin === "function";

const MAGIC = [0x5a, 0x47, 0x52, 0x4e]; // "ZGRN"
const OP_BEGIN = 0x01;
const OP_CHUNK = 0x02;
const OP_COMMIT = 0x03;
const OP_NOTE = 0x04;
const OP_POLL = 0x06;
const OP_STATUS = 0x81;
const OP_ACTIVITY = 0x82;

const MAX_SAMPLE_FLOATS = 48000 * 60 * 2; // 60 s of stereo 48 kHz
// 128 KiB per chunk over WebCLAP postMessage; smaller on the native path so
// each base64ed IPC string stays well under per-message limits.
const CHUNK_FLOATS = NATIVE ? 8192 : 32768;

function bytesToBase64(bytes) {
  let binary = "";
  const step = 0x8000; // keep String.fromCharCode's argument list bounded
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + step));
  }
  return btoa(binary);
}

function base64ToBytes(text) {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function post(buffer) {
  if (NATIVE) {
    window.sendToPlugin({ type: "bin", data: bytesToBase64(new Uint8Array(buffer)) });
  } else {
    window.parent.postMessage(buffer, "*");
  }
}

function zgrnPacket(op, bodyBytes) {
  const out = new Uint8Array(5 + bodyBytes);
  out.set(MAGIC, 0);
  out[4] = op;
  return out;
}

function encodeReady() {
  return new Uint8Array([0x65, 0x72, 0x65, 0x61, 0x64, 0x79]).buffer;
}

function encodeSet(id, value) {
  const buf = new ArrayBuffer(20);
  const view = new DataView(buf);
  view.setUint8(0, 0xa1);
  view.setUint8(1, 0x63);
  view.setUint8(2, 0x73);
  view.setUint8(3, 0x65);
  view.setUint8(4, 0x74);
  view.setUint8(5, 0x82);
  view.setUint8(6, 0x1a);
  view.setUint32(7, id, false);
  view.setUint8(11, 0xfb);
  view.setFloat64(12, value, false);
  return buf;
}

function decodeParamsSnapshot(ab) {
  const view = new DataView(ab);
  let p = 0;
  if (view.byteLength < 9 || view.getUint8(p++) !== 0xa1 || view.getUint8(p++) !== 0x66) {
    return null;
  }
  if (String.fromCharCode(...new Uint8Array(ab, p, 6)) !== "params") return null;
  p += 6;
  const head = view.getUint8(p++);
  let count;
  if ((head & 0xe0) === 0xa0 && (head & 0x1f) < 24) {
    count = head & 0x1f;
  } else if (head === 0xb8) {
    count = view.getUint8(p++);
  } else {
    return null;
  }
  const out = new Map();
  for (let i = 0; i < count; i++) {
    if (p + 13 > view.byteLength || view.getUint8(p++) !== 0x1a) return null;
    const key = view.getUint32(p, false);
    p += 4;
    if (view.getUint8(p++) !== 0xfb) return null;
    out.set(key, view.getFloat64(p, false));
    p += 8;
  }
  return out;
}

function isZgrn(bytes, op) {
  if (bytes.length < 5) return false;
  for (let i = 0; i < 4; i++) if (bytes[i] !== MAGIC[i]) return false;
  return bytes[4] === op;
}

function decodeStatus(ab) {
  const bytes = new Uint8Array(ab);
  if (bytes.length < 15 || !isZgrn(bytes, OP_STATUS)) return null;
  const view = new DataView(ab);
  return {
    hasSample: bytes[5] !== 0,
    channels: bytes[6],
    frames: view.getUint32(7, true),
    sampleRate: view.getFloat32(11, true),
  };
}

function decodeActivity(ab) {
  const bytes = new Uint8Array(ab);
  if (bytes.length < 6 || !isZgrn(bytes, OP_ACTIVITY)) return null;
  const count = Math.min(bytes[5], Math.floor((bytes.length - 6) / 4));
  const view = new DataView(ab);
  const positions = [];
  for (let i = 0; i < count; i++) positions.push(view.getFloat32(6 + i * 4, true));
  return positions;
}

function sendSet(id, value) {
  if (NATIVE) {
    window.sendToPlugin({ type: "set", id, value });
  } else {
    post(encodeSet(id, value));
  }
}

function sendNote(on, key, velocity) {
  const pkt = zgrnPacket(OP_NOTE, 3);
  pkt[5] = on ? 1 : 0;
  pkt[6] = key & 0x7f;
  pkt[7] = velocity & 0x7f;
  post(pkt.buffer);
}

function sendSample(interleaved, sampleRate, channels, frames) {
  const begin = zgrnPacket(OP_BEGIN, 9);
  const bview = new DataView(begin.buffer);
  bview.setFloat32(5, sampleRate, true);
  begin[9] = channels;
  bview.setUint32(10, frames, true);
  post(begin.buffer);

  for (let offset = 0; offset < interleaved.length; offset += CHUNK_FLOATS) {
    const slice = interleaved.subarray(offset, Math.min(offset + CHUNK_FLOATS, interleaved.length));
    const pkt = zgrnPacket(OP_CHUNK, 4 + slice.length * 4);
    const view = new DataView(pkt.buffer);
    view.setUint32(5, offset, true);
    for (let i = 0; i < slice.length; i++) {
      view.setFloat32(9 + i * 4, slice[i], true);
    }
    post(pkt.buffer);
  }
  post(zgrnPacket(OP_COMMIT, 0).buffer);
}

// ---------------------------------------------------------------------------
// State.
// ---------------------------------------------------------------------------

const values = new Map(); // web id -> current value (params + snapshot)
for (const def of SLIDERS) if (def.id !== undefined) values.set(def.id, def.default);
values.set(P.SPAWN_MODE, 0);
values.set(P.RATE, 25);
values.set(P.SYNC_RATE, 4);
values.set(P.DENSITY, 8);
values.set(P.ROOT, 60);
values.set(P.LEN_KEYTRACK, 0);
values.set(P.ALIGN, 0);
values.set(P.WARM, 0);
values.set(P.CHORD_TYPE, 0);
values.set(P.CHORD_RANGE, 1);
values.set(P.CHORD_PATTERN, 0);

const state = {
  fileName: null,
  sampleRate: 0,
  frames: 0,
  mono: null, // Float32Array mono mix for display
  pluginStatus: null,
  grains: [], // { x: normalized position, y: lane 0..1, t: ms first seen }
};

// ---------------------------------------------------------------------------
// DOM.
// ---------------------------------------------------------------------------

const $ = (id) => document.getElementById(id);
const statusEl = $("status");
const fileLabel = $("file-label");
const waveWrap = $("wave-wrap");
const canvas = $("wave");
const ctx2d = canvas.getContext("2d");
const dropHint = $("drop-hint");
const fileInput = $("file-input");
const sampleInfo = $("sample-info");
const spawnTabs = $("spawn-tabs");
const rowRate = $("row-rate");
const rowSync = $("row-sync");
const rowDensity = $("row-density");
const rateInput = $("rate");
const rateValue = $("rate-value");
const densityInput = $("density");
const densityValue = $("density-value");
const syncSelect = $("sync-rate");
const syncHint = $("sync-hint");
const rootSelect = $("root-note");
const chordTypeSelect = $("chord-type");
const chordRangeSelect = $("chord-range");
const chordPatternSelect = $("chord-pattern");
const paramsForm = $("params");
const keyboardEl = $("keyboard");

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

// ---------------------------------------------------------------------------
// Param controls.
// ---------------------------------------------------------------------------

// id -> { set(value) } updaters for snapshot application.
const controls = new Map();

function makeSlider(def, input, readoutEl) {
  const toNorm = (v) => {
    const t = (v - def.min) / (def.max - def.min);
    return Math.round(Math.pow(clamp(t, 0, 1), 1 / (def.curve || 1)) * 1000);
  };
  const fromNorm = (n) => {
    const t = Math.pow(n / 1000, def.curve || 1);
    let v = def.min + t * (def.max - def.min);
    if (def.step) v = Math.round(v / def.step) * def.step;
    return clamp(v, def.min, def.max);
  };
  input.value = toNorm(values.get(def.id));
  if (readoutEl) readoutEl.textContent = def.fmt(values.get(def.id));
  input.addEventListener("input", () => {
    const value = fromNorm(Number(input.value));
    values.set(def.id, value);
    if (readoutEl) readoutEl.textContent = def.fmt(value);
    sendSet(def.id, value);
  });
  controls.set(def.id, {
    set(value) {
      const v = clamp(value, def.min, def.max);
      values.set(def.id, v);
      input.value = toNorm(v);
      if (readoutEl) readoutEl.textContent = def.fmt(v);
    },
  });
}

for (const def of SLIDERS) {
  if (def.section) {
    const title = document.createElement("p");
    title.className = "params-section";
    title.textContent = def.section;
    paramsForm.append(title);
    continue;
  }
  const wrap = document.createElement("label");
  wrap.className = "control";
  const title = document.createElement("span");
  title.className = "control-label";
  title.textContent = def.label;
  const readout = document.createElement("span");
  readout.className = "readout";
  const input = document.createElement("input");
  input.type = "range";
  input.min = 0;
  input.max = 1000;
  input.step = 1;
  wrap.append(title, input, readout);
  paramsForm.append(wrap);
  makeSlider(def, input, readout);
}

makeSlider(
  { id: P.RATE, label: "Rate", min: 0.1, max: 400, default: 25, curve: 2.5, fmt: (v) => `${v.toFixed(1)} Hz` },
  rateInput,
  rateValue,
);
makeSlider(
  { id: P.DENSITY, label: "Density", min: 0.5, max: 64, default: 8, curve: 2, fmt: (v) => `${v.toFixed(1)} grains` },
  densityInput,
  densityValue,
);

function makeSelect(id, select, onchange) {
  select.addEventListener("change", () => {
    const value = Number(select.value);
    values.set(id, value);
    sendSet(id, value);
    if (onchange) onchange(value);
  });
  controls.set(id, {
    set(value) {
      values.set(id, value);
      select.value = Math.round(value);
      if (onchange) onchange(value);
    },
  });
}

function makeToggle(id, button) {
  button.addEventListener("click", () => {
    const value = values.get(id) >= 0.5 ? 0 : 1;
    values.set(id, value);
    button.classList.toggle("active", value >= 0.5);
    sendSet(id, value);
  });
  controls.set(id, {
    set(value) {
      values.set(id, value);
      button.classList.toggle("active", value >= 0.5);
    },
  });
}

function fillSelect(select, entries) {
  for (const [value, label] of entries) {
    const opt = document.createElement("option");
    opt.value = value;
    opt.textContent = label;
    select.append(opt);
  }
}

fillSelect(rootSelect, Array.from({ length: 128 }, (_, k) => [k, `${noteName(k)} (${k})`]));
rootSelect.value = 60;
fillSelect(syncSelect, SYNC_LABELS.map((label, i) => [i, label]));
syncSelect.value = 4;
fillSelect(chordTypeSelect, CHORD_TYPES.map((label, i) => [i, label]));
fillSelect(chordRangeSelect, [1, 2, 3, 4].map((n) => [n, `${n} oct`]));
fillSelect(chordPatternSelect, CHORD_PATTERNS.map((label, i) => [i, label]));

makeSelect(P.ROOT, rootSelect, () => renderKeyboard());
makeSelect(P.SYNC_RATE, syncSelect);
makeSelect(P.CHORD_TYPE, chordTypeSelect);
makeSelect(P.CHORD_RANGE, chordRangeSelect);
makeSelect(P.CHORD_PATTERN, chordPatternSelect);
makeToggle(P.LEN_KEYTRACK, $("toggle-keytrack"));
makeToggle(P.ALIGN, $("toggle-align"));
makeToggle(P.WARM, $("toggle-warm"));

// Spawn mode tabs (a stepped param rendered as tabs).
function applySpawnMode(mode) {
  const m = Math.round(clamp(mode, 0, 2));
  values.set(P.SPAWN_MODE, m);
  for (const btn of spawnTabs.querySelectorAll("button")) {
    btn.classList.toggle("active", Number(btn.dataset.mode) === m);
  }
  rowRate.hidden = m !== 0;
  rowSync.hidden = m !== 1;
  rowDensity.hidden = m !== 2;
}

spawnTabs.addEventListener("click", (e) => {
  const btn = e.target.closest("button[data-mode]");
  if (!btn) return;
  const mode = Number(btn.dataset.mode);
  applySpawnMode(mode);
  sendSet(P.SPAWN_MODE, mode);
});
controls.set(P.SPAWN_MODE, { set: applySpawnMode });

// The WebCLAP scaffold has no host transport; Sync runs at a fixed 120 BPM
// there. The native builds feed the real host tempo.
syncHint.textContent = NATIVE ? "follows host tempo" : "@ 120 BPM (no host transport)";

function applySnapshot(snapshot) {
  for (const [id, value] of snapshot) {
    const control = controls.get(id);
    if (control) control.set(value);
  }
  statusEl.textContent = "CONNECTED";
}

// ---------------------------------------------------------------------------
// File loading & decoding.
// ---------------------------------------------------------------------------

let audioContext = null;

async function decodeFile(file) {
  if (!audioContext) audioContext = new AudioContext();
  if (audioContext.state === "suspended") audioContext.resume().catch(() => {});
  const bytes = await file.arrayBuffer();
  return audioContext.decodeAudioData(bytes);
}

async function loadFile(file) {
  statusEl.textContent = "DECODING…";
  let audioBuffer;
  try {
    audioBuffer = await decodeFile(file);
  } catch (err) {
    statusEl.textContent = "DECODE FAILED";
    sampleInfo.textContent = `Could not decode ${file.name}: ${err && err.message ? err.message : err}`;
    return;
  }

  const channels = Math.min(2, audioBuffer.numberOfChannels) || 1;
  let frames = audioBuffer.length;
  const maxFrames = Math.floor(MAX_SAMPLE_FLOATS / channels);
  const truncated = frames > maxFrames;
  if (truncated) frames = maxFrames;

  const chans = [];
  for (let c = 0; c < channels; c++) chans.push(audioBuffer.getChannelData(c));
  const interleaved = new Float32Array(frames * channels);
  const mono = new Float32Array(frames);
  for (let i = 0; i < frames; i++) {
    let sum = 0;
    for (let c = 0; c < channels; c++) {
      const s = chans[c][i];
      interleaved[i * channels + c] = s;
      sum += s;
    }
    mono[i] = sum / channels;
  }

  state.fileName = file.name;
  state.sampleRate = audioBuffer.sampleRate;
  state.frames = frames;
  state.mono = mono;
  state.grains = [];

  fileLabel.textContent = file.name;
  fileLabel.title = file.name;
  dropHint.classList.add("hidden");
  const secs = frames / audioBuffer.sampleRate;
  sampleInfo.textContent =
    `${audioBuffer.sampleRate.toFixed(0)} Hz · ${channels} ch · ${secs.toFixed(2)} s` +
    (truncated ? " · truncated to 60 s" : "");

  statusEl.textContent = "SENDING…";
  sendSample(interleaved, state.sampleRate, channels, frames);
  renderWave();
}

fileLabel.addEventListener("click", () => fileInput.click());
fileLabel.style.cursor = "pointer";
fileInput.addEventListener("change", () => {
  const file = fileInput.files && fileInput.files[0];
  if (file) loadFile(file);
  fileInput.value = "";
});
dropHint.addEventListener("click", () => fileInput.click());
for (const ev of ["dragover", "dragenter"]) {
  waveWrap.addEventListener(ev, (e) => {
    e.preventDefault();
    waveWrap.classList.add("dragover");
  });
}
for (const ev of ["dragleave", "drop"]) {
  waveWrap.addEventListener(ev, (e) => {
    e.preventDefault();
    waveWrap.classList.remove("dragover");
  });
}
waveWrap.addEventListener("drop", (e) => {
  const file = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
  if (file) loadFile(file);
});

// ---------------------------------------------------------------------------
// Waveform + seek bar + grain activity.
//
// The waveform peaks are pre-rendered into an offscreen canvas once per
// file/resize; the animation loop just blits it and draws the overlays
// (spray band, position cursor, fading grain dots).
// ---------------------------------------------------------------------------

const waveImage = document.createElement("canvas");

function resizeCanvas() {
  const rect = waveWrap.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(rect.width * dpr));
  canvas.height = Math.max(1, Math.round(rect.height * dpr));
  renderWave();
}
window.addEventListener("resize", resizeCanvas);

function renderWave() {
  waveImage.width = canvas.width;
  waveImage.height = canvas.height;
  const g = waveImage.getContext("2d");
  const w = waveImage.width;
  const h = waveImage.height;
  g.clearRect(0, 0, w, h);
  const mid = h / 2;

  g.strokeStyle = "rgba(141, 134, 171, 0.25)";
  g.beginPath();
  g.moveTo(0, mid);
  g.lineTo(w, mid);
  g.stroke();

  const mono = state.mono;
  if (!mono) return;
  const frames = state.frames;
  g.strokeStyle = "#7a68c9";
  for (let x = 0; x < w; x++) {
    const f0 = Math.floor((x / w) * frames);
    const f1 = Math.max(f0 + 1, Math.floor(((x + 1) / w) * frames));
    let lo = 1;
    let hi = -1;
    const step = Math.max(1, Math.floor((f1 - f0) / 64));
    for (let f = f0; f < f1; f += step) {
      const s = mono[f];
      if (s < lo) lo = s;
      if (s > hi) hi = s;
    }
    g.beginPath();
    g.moveTo(x + 0.5, mid - hi * (mid - 4));
    g.lineTo(x + 0.5, mid - lo * (mid - 4));
    g.stroke();
  }
}

/// Spray half-width in normalized (0..1) sample position.
function sprayNorm() {
  const frames = state.frames || (state.pluginStatus && state.pluginStatus.frames) || 0;
  const rate = state.sampleRate || (state.pluginStatus && state.pluginStatus.sampleRate) || 0;
  if (!frames || !rate) return 0;
  const sprayFrames = (values.get(P.R_POS) / 1000) * rate;
  return clamp(sprayFrames / frames, 0, 1);
}

const GRAIN_FADE_MS = 350;

function drawOverlay() {
  const w = canvas.width;
  const h = canvas.height;
  ctx2d.clearRect(0, 0, w, h);
  ctx2d.drawImage(waveImage, 0, 0);

  const dpr = window.devicePixelRatio || 1;
  const position = clamp(values.get(P.POSITION), 0, 1);
  const px = position * w;

  // Spray band.
  const spray = sprayNorm();
  if (spray > 0) {
    const bx = clamp(position - spray, 0, 1) * w;
    const bw = clamp(position + spray, 0, 1) * w - bx;
    ctx2d.fillStyle = "rgba(167, 139, 250, 0.10)";
    ctx2d.fillRect(bx, 0, bw, h);
  }

  // Live grains.
  const now = performance.now();
  state.grains = state.grains.filter((grain) => now - grain.t < GRAIN_FADE_MS);
  for (const grain of state.grains) {
    const alpha = 1 - (now - grain.t) / GRAIN_FADE_MS;
    ctx2d.fillStyle = `rgba(214, 198, 255, ${(alpha * 0.9).toFixed(3)})`;
    ctx2d.beginPath();
    ctx2d.arc(grain.x * w, (0.15 + grain.y * 0.7) * h, 2.2 * dpr * (0.6 + alpha * 0.6), 0, Math.PI * 2);
    ctx2d.fill();
  }

  // Position cursor (the seek bar).
  ctx2d.fillStyle = "#c9b8ff";
  ctx2d.fillRect(px - 1 * dpr, 0, 2 * dpr, h);
  ctx2d.fillStyle = "#a78bfa";
  ctx2d.beginPath();
  ctx2d.moveTo(px - 5 * dpr, 0);
  ctx2d.lineTo(px + 5 * dpr, 0);
  ctx2d.lineTo(px, 7 * dpr);
  ctx2d.closePath();
  ctx2d.fill();

  requestAnimationFrame(drawOverlay);
}

// Seek by dragging anywhere on the waveform.
let seeking = false;

function seekTo(e) {
  const rect = canvas.getBoundingClientRect();
  const position = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const control = controls.get(P.POSITION);
  if (control) control.set(position);
  sendSet(P.POSITION, position);
}

canvas.addEventListener("pointerdown", (e) => {
  seeking = true;
  canvas.setPointerCapture(e.pointerId);
  seekTo(e);
});
canvas.addEventListener("pointermove", (e) => {
  if (seeking) seekTo(e);
});
canvas.addEventListener("pointerup", () => {
  seeking = false;
});

function handleActivity(positions) {
  const now = performance.now();
  for (const x of positions) {
    state.grains.push({ x: clamp(x, 0, 1), y: Math.random(), t: now });
  }
  if (state.grains.length > 192) {
    state.grains.splice(0, state.grains.length - 192);
  }
}

// The native webview bridge is request/response, so poll for activity;
// the WebCLAP host pushes it from process() unprompted.
if (NATIVE) {
  setInterval(() => post(zgrnPacket(OP_POLL, 0).buffer), 33);
}

// ---------------------------------------------------------------------------
// Keyboard.
// ---------------------------------------------------------------------------

const KEY_LO = 24; // C0
const KEY_HI = 96; // C6
const BLACK = new Set([1, 3, 6, 8, 10]);
let heldKey = -1;

function renderKeyboard() {
  const root = Math.round(values.get(P.ROOT));
  keyboardEl.textContent = "";
  for (let key = KEY_LO; key <= KEY_HI; key++) {
    const el = document.createElement("div");
    const black = BLACK.has(key % 12);
    el.className = `key ${black ? "black" : "white"}`;
    el.dataset.key = key;
    if (key === root) el.classList.add("root");
    el.title = noteName(key);
    keyboardEl.append(el);
  }
}

function previewOff() {
  if (heldKey >= 0) {
    sendNote(false, heldKey, 0);
    const el = keyboardEl.querySelector(`[data-key="${heldKey}"]`);
    if (el) el.classList.remove("held");
    heldKey = -1;
  }
}

keyboardEl.addEventListener("pointerdown", (e) => {
  const el = e.target.closest(".key");
  if (!el) return;
  previewOff();
  heldKey = Number(el.dataset.key);
  el.classList.add("held");
  sendNote(true, heldKey, 100);
});
window.addEventListener("pointerup", previewOff);
keyboardEl.addEventListener("pointerleave", previewOff);

// ---------------------------------------------------------------------------
// Plugin messages.
// ---------------------------------------------------------------------------

function handleStatus(status) {
  state.pluginStatus = status;
  if (status.hasSample && !state.mono) {
    const secs = status.sampleRate > 0 ? status.frames / status.sampleRate : 0;
    sampleInfo.textContent =
      `Plugin: ${status.sampleRate.toFixed(0)} Hz · ${status.channels} ch · ` +
      `${secs.toFixed(2)} s (drop a file to replace)`;
  }
  statusEl.textContent = "CONNECTED";
}

function handleBinary(ab) {
  const status = decodeStatus(ab);
  if (status) {
    handleStatus(status);
    return true;
  }
  const positions = decodeActivity(ab);
  if (positions) {
    handleActivity(positions);
    return true;
  }
  return false;
}

if (NATIVE) {
  window.onPluginMessage = (msg) => {
    if (!msg) return;
    if (msg.type === "params" && msg.values) {
      applySnapshot(new Map(Object.entries(msg.values).map(([id, v]) => [Number(id), v])));
    } else if (msg.type === "bin" && typeof msg.data === "string") {
      handleBinary(base64ToBytes(msg.data).buffer);
    }
  };
} else {
  window.addEventListener("message", (event) => {
    if (!(event.data instanceof ArrayBuffer)) return;
    if (handleBinary(event.data)) return;
    const snapshot = decodeParamsSnapshot(event.data);
    if (snapshot) applySnapshot(snapshot);
  });
}

// ---------------------------------------------------------------------------
// Boot.
// ---------------------------------------------------------------------------

applySpawnMode(0);
renderKeyboard();
resizeCanvas();
requestAnimationFrame(drawOverlay);
if (NATIVE) {
  window.sendToPlugin({ type: "ready" });
} else {
  post(encodeReady());
}
