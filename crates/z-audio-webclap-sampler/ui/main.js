// Z Audio Sampler UI — Logic-style sampler front end.
//
// Load an audio file (click / drag & drop), decode it in the GUI with
// decodeAudioData, and stream the PCM plus a zone table to the plugin over
// the clap.webview/3 binary channel:
//
//   ZSMP 0x01 BeginSample   f32 rate · u8 channels · u32 frames
//   ZSMP 0x02 SampleChunk   u32 floatOffset · f32le PCM payload
//   ZSMP 0x03 CommitZones   u16 count · count × 40-byte zone record
//   ZSMP 0x04 NotePreview   u8 on · u8 key · u8 velocity
//   ZSMP 0x05 ClearSample
//   ZSMP 0x81 Status        (plugin → UI)
//
// Modes mirror Logic's Quick Sampler: CLASSIC (chromatic repitch with
// loop), ONE SHOT (plays through, ignores note-off), SLICE (auto-cut at
// transients or an equal grid, one key per slice).

"use strict";

import { computeOnsetCurve, detectSliceMarkers } from "./onsets.js";

// ---------------------------------------------------------------------------
// Parameters (shared ready/set/params-snapshot protocol with the scaffold).
// ---------------------------------------------------------------------------

const PARAMS = [
  { id: 300, label: "Gain", min: -48, max: 12, default: 0, fmt: (v) => `${v.toFixed(1)} dB` },
  { id: 301, label: "Attack", min: 0.001, max: 5, default: 0.002, fmt: fmtSeconds, curve: 3 },
  { id: 302, label: "Decay", min: 0, max: 5, default: 0, fmt: fmtSeconds, curve: 3 },
  { id: 303, label: "Sustain", min: 0, max: 1, default: 1, fmt: fmtPercent },
  { id: 304, label: "Release", min: 0.01, max: 10, default: 0.25, fmt: fmtSeconds, curve: 3 },
  { id: 305, label: "Tune", min: -100, max: 100, default: 0, fmt: (v) => `${v.toFixed(0)} ct` },
  { id: 306, label: "Transpose", min: -24, max: 24, default: 0, step: 1, fmt: (v) => `${v > 0 ? "+" : ""}${v.toFixed(0)} st` },
  { id: 307, label: "Velocity", min: 0, max: 1, default: 1, fmt: fmtPercent },
  { id: 308, label: "Width", min: 0, max: 1, default: 1, fmt: fmtPercent },
];

function fmtSeconds(v) {
  return v < 1 ? `${(v * 1000).toFixed(0)} ms` : `${v.toFixed(2)} s`;
}

function fmtPercent(v) {
  return `${(v * 100).toFixed(0)} %`;
}

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

function noteName(key) {
  return `${NOTE_NAMES[key % 12]}${Math.floor(key / 12) - 1}`;
}

// ---------------------------------------------------------------------------
// Transport.
// ---------------------------------------------------------------------------

const MAGIC = [0x5a, 0x53, 0x4d, 0x50]; // "ZSMP"
const OP_BEGIN = 0x01;
const OP_CHUNK = 0x02;
const OP_COMMIT = 0x03;
const OP_NOTE = 0x04;
const OP_STATUS = 0x81;

const MAX_ZONES = 128;
const MAX_SAMPLE_FLOATS = 48000 * 60 * 2; // 60 s of stereo 48 kHz
const CHUNK_FLOATS = 32768; // 128 KiB per chunk message

function post(buffer) {
  window.parent.postMessage(buffer, "*");
}

function zsmpPacket(op, bodyBytes) {
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

function decodeStatus(ab) {
  const bytes = new Uint8Array(ab);
  if (bytes.length < 17) return null;
  for (let i = 0; i < 4; i++) if (bytes[i] !== MAGIC[i]) return null;
  if (bytes[4] !== OP_STATUS) return null;
  const view = new DataView(ab);
  return {
    hasSample: bytes[5] !== 0,
    channels: bytes[6],
    zones: view.getUint16(7, true),
    frames: view.getUint32(9, true),
    sampleRate: view.getFloat32(13, true),
  };
}

function sendSet(id, value) {
  post(encodeSet(id, value));
}

function sendNote(on, key, velocity) {
  const pkt = zsmpPacket(OP_NOTE, 3);
  pkt[5] = on ? 1 : 0;
  pkt[6] = key & 0x7f;
  pkt[7] = velocity & 0x7f;
  post(pkt.buffer);
}

function sendSample(interleaved, sampleRate, channels, frames) {
  const begin = zsmpPacket(OP_BEGIN, 9);
  const bview = new DataView(begin.buffer);
  bview.setFloat32(5, sampleRate, true);
  begin[9] = channels;
  bview.setUint32(10, frames, true);
  post(begin.buffer);

  for (let offset = 0; offset < interleaved.length; offset += CHUNK_FLOATS) {
    const slice = interleaved.subarray(offset, Math.min(offset + CHUNK_FLOATS, interleaved.length));
    const pkt = zsmpPacket(OP_CHUNK, 4 + slice.length * 4);
    const view = new DataView(pkt.buffer);
    view.setUint32(5, offset, true);
    for (let i = 0; i < slice.length; i++) {
      view.setFloat32(9 + i * 4, slice[i], true);
    }
    post(pkt.buffer);
  }
}

function sendZones(zones) {
  const pkt = zsmpPacket(OP_COMMIT, 2 + zones.length * 40);
  const view = new DataView(pkt.buffer);
  view.setUint16(5, zones.length, true);
  zones.forEach((z, i) => {
    const at = 7 + i * 40;
    view.setUint8(at + 0, z.lokey);
    view.setUint8(at + 1, z.hikey);
    view.setUint8(at + 2, z.root);
    view.setUint8(at + 3, z.oneShot ? 1 : 0);
    view.setUint8(at + 4, z.loopMode);
    view.setUint32(at + 8, z.startFrame, true);
    view.setUint32(at + 12, z.endFrame, true);
    view.setUint32(at + 16, z.loopStart, true);
    view.setUint32(at + 20, z.loopEnd, true);
    view.setFloat32(at + 24, z.gainDb, true);
    view.setFloat32(at + 28, z.tuneCents, true);
    view.setFloat32(at + 32, z.pan, true);
    view.setFloat32(at + 36, z.loopXfadeS, true);
  });
  post(pkt.buffer);
}

// ---------------------------------------------------------------------------
// State.
// ---------------------------------------------------------------------------

const state = {
  mode: "classic", // classic | oneshot | slice
  fileName: null,
  sampleRate: 0,
  channels: 0,
  frames: 0,
  mono: null, // Float32Array mono mix for display + analysis
  interleaved: null, // what was sent to the plugin
  trimStart: 0,
  trimEnd: 0,
  rootNote: 60,
  loopMode: 0,
  loopStart: 0,
  loopEnd: 0,
  loopXfadeS: 0.01,
  sliceGrid: "transient",
  sliceSensitivity: 0.5,
  sliceBaseKey: 36, // C2
  sliceMarkers: [], // absolute frame positions, sorted, first >= trimStart
  onsetCurve: null, // cached spectral-flux curve (per file; see onsets.js)
  mappedKeys: new Map(), // key -> {root:boolean} for keyboard display
  pluginStatus: null,
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
const modeTabs = $("mode-tabs");
const rowPitched = $("row-pitched");
const rowSlice = $("row-slice");
const modeHint = $("mode-hint");
const rootSelect = $("root-note");
const loopModeSelect = $("loop-mode");
const loopModeWrap = $("loop-mode-wrap");
const loopXfade = $("loop-xfade");
const loopXfadeWrap = $("loop-xfade-wrap");
const loopXfadeValue = $("loop-xfade-value");
const sliceGridSelect = $("slice-grid");
const sliceSensitivity = $("slice-sensitivity");
const sliceSensitivityWrap = $("slice-sensitivity-wrap");
const sliceBaseKeySelect = $("slice-base-key");
const sliceCountEl = $("slice-count");
const paramsForm = $("params");
const keyboardEl = $("keyboard");

const MODE_HINTS = {
  classic: "Notes repitch the sample around the root key. Drag the edge handles to trim; loop handles appear when looping is on.",
  oneshot: "Plays the whole (trimmed) sample per note and ignores note-off. Notes still repitch around the root key.",
  slice: "Auto-cut at transients or an equal grid, one key per slice from the base key up. Double-click to add/remove markers, drag to move.",
};

// ---------------------------------------------------------------------------
// Param controls.
// ---------------------------------------------------------------------------

const controls = new Map();

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

for (const def of PARAMS) {
  const wrap = document.createElement("label");
  wrap.className = "control";
  const title = document.createElement("span");
  title.className = "control-label";
  title.textContent = def.label;
  const readout = document.createElement("span");
  readout.className = "readout";
  readout.textContent = def.fmt(def.default);
  const input = document.createElement("input");
  input.type = "range";
  input.min = 0;
  input.max = 1000;
  input.step = 1;
  const toNorm = (v) => {
    const t = (v - def.min) / (def.max - def.min);
    return Math.round(Math.pow(t, 1 / (def.curve || 1)) * 1000);
  };
  const fromNorm = (n) => {
    const t = Math.pow(n / 1000, def.curve || 1);
    let v = def.min + t * (def.max - def.min);
    if (def.step) v = Math.round(v / def.step) * def.step;
    return clamp(v, def.min, def.max);
  };
  input.value = toNorm(def.default);
  input.addEventListener("input", () => {
    const value = fromNorm(Number(input.value));
    readout.textContent = def.fmt(value);
    sendSet(def.id, value);
  });
  wrap.append(title, input, readout);
  paramsForm.append(wrap);
  controls.set(def.id, { input, readout, def, toNorm });
}

function applySnapshot(snapshot) {
  for (const [id, value] of snapshot) {
    const control = controls.get(id);
    if (!control) continue;
    const v = clamp(value, control.def.min, control.def.max);
    control.input.value = control.toNorm(v);
    control.readout.textContent = control.def.fmt(v);
  }
  statusEl.textContent = "CONNECTED";
}

// ---------------------------------------------------------------------------
// Note selects.
// ---------------------------------------------------------------------------

for (let key = 0; key < 128; key++) {
  const opt = document.createElement("option");
  opt.value = key;
  opt.textContent = `${noteName(key)} (${key})`;
  rootSelect.append(opt);
  sliceBaseKeySelect.append(opt.cloneNode(true));
}
rootSelect.value = state.rootNote;
sliceBaseKeySelect.value = state.sliceBaseKey;

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
  state.channels = channels;
  state.frames = frames;
  state.mono = mono;
  state.interleaved = interleaved;
  state.trimStart = 0;
  state.trimEnd = frames;
  state.loopStart = 0;
  state.loopEnd = frames;
  state.sliceMarkers = [];
  state.onsetCurve = null;

  fileLabel.textContent = file.name;
  fileLabel.title = file.name;
  dropHint.classList.add("hidden");
  const secs = frames / audioBuffer.sampleRate;
  sampleInfo.textContent =
    `${audioBuffer.sampleRate.toFixed(0)} Hz · ${channels} ch · ${secs.toFixed(2)} s` +
    (truncated ? " · truncated to 60 s" : "");

  statusEl.textContent = "SENDING…";
  sendSample(interleaved, state.sampleRate, channels, frames);
  if (state.mode === "slice") detectSlices();
  commitZones();
  render();
}

waveWrap.addEventListener("click", () => {
  if (!state.mono) fileInput.click();
});
// Once a sample is loaded the canvas is for editing; the file name in the
// header stays clickable to load a different file.
fileLabel.addEventListener("click", () => fileInput.click());
fileLabel.style.cursor = "pointer";
fileInput.addEventListener("change", () => {
  const file = fileInput.files && fileInput.files[0];
  if (file) loadFile(file);
  fileInput.value = "";
});
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
// Slice detection.
// ---------------------------------------------------------------------------

function detectSlices() {
  if (!state.mono) {
    state.sliceMarkers = [];
    return;
  }
  const start = state.trimStart;
  const end = state.trimEnd;
  if (state.sliceGrid !== "transient") {
    const n = Number(state.sliceGrid);
    const markers = [];
    for (let i = 0; i < n; i++) {
      markers.push(Math.round(start + ((end - start) * i) / n));
    }
    state.sliceMarkers = markers;
    return;
  }
  // The STFT onset curve depends only on the audio; computing it once per
  // file keeps sensitivity-slider drags cheap (re-pick + refine only).
  if (!state.onsetCurve) {
    state.onsetCurve = computeOnsetCurve(state.mono, state.sampleRate);
  }
  state.sliceMarkers = detectSliceMarkers(
    state.mono,
    state.sampleRate,
    start,
    end,
    state.sliceSensitivity,
    state.onsetCurve,
    MAX_ZONES,
  );
}

// ---------------------------------------------------------------------------
// Zone building & commit.
// ---------------------------------------------------------------------------

let commitTimer = 0;

function commitZonesSoon() {
  clearTimeout(commitTimer);
  commitTimer = setTimeout(commitZones, 90);
}

function currentZones() {
  if (!state.mono) return null; // nothing loaded locally — leave plugin as-is
  const trimStart = Math.min(state.trimStart, state.trimEnd);
  const trimEnd = Math.max(state.trimStart, state.trimEnd);
  if (state.mode === "slice") {
    const markers = state.sliceMarkers.filter((m) => m >= trimStart && m < trimEnd);
    const zones = [];
    for (let i = 0; i < markers.length && zones.length < MAX_ZONES; i++) {
      const key = state.sliceBaseKey + i;
      if (key > 127) break;
      zones.push({
        lokey: key,
        hikey: key,
        root: key,
        oneShot: true,
        loopMode: 0,
        startFrame: markers[i],
        endFrame: i + 1 < markers.length ? markers[i + 1] : trimEnd,
        loopStart: 0,
        loopEnd: 0,
        gainDb: 0,
        tuneCents: 0,
        pan: 0,
        loopXfadeS: 0,
      });
    }
    return zones;
  }
  const oneShot = state.mode === "oneshot";
  const loopMode = oneShot ? 0 : state.loopMode;
  const loopStart = clamp(Math.min(state.loopStart, state.loopEnd), trimStart, trimEnd);
  const loopEnd = clamp(Math.max(state.loopStart, state.loopEnd), trimStart, trimEnd);
  return [
    {
      lokey: 0,
      hikey: 127,
      root: state.rootNote,
      oneShot,
      loopMode,
      startFrame: trimStart,
      endFrame: trimEnd,
      loopStart: Math.max(0, loopStart - trimStart),
      loopEnd: Math.max(0, loopEnd - trimStart),
      gainDb: 0,
      tuneCents: 0,
      pan: 0,
      loopXfadeS: state.loopXfadeS,
    },
  ];
}

function commitZones() {
  const zones = currentZones();
  if (!zones) return;
  sendZones(zones);
  state.mappedKeys = new Map();
  for (const z of zones) {
    for (let k = z.lokey; k <= z.hikey; k++) {
      state.mappedKeys.set(k, { root: k === z.root });
    }
  }
  sliceCountEl.textContent =
    state.mode === "slice" ? `${zones.length} slice${zones.length === 1 ? "" : "s"}` : "";
  renderKeyboard();
  render();
}

// ---------------------------------------------------------------------------
// Mode + control events.
// ---------------------------------------------------------------------------

function setMode(mode) {
  state.mode = mode;
  for (const btn of modeTabs.querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.mode === mode);
  }
  rowPitched.hidden = mode === "slice";
  rowSlice.hidden = mode !== "slice";
  loopModeWrap.style.display = mode === "classic" ? "" : "none";
  loopXfadeWrap.style.display = mode === "classic" ? "" : "none";
  modeHint.textContent = MODE_HINTS[mode];
  if (mode === "slice" && state.mono && state.sliceMarkers.length === 0) {
    detectSlices();
  }
  commitZones();
}

modeTabs.addEventListener("click", (e) => {
  const btn = e.target.closest("button[data-mode]");
  if (btn) setMode(btn.dataset.mode);
});

rootSelect.addEventListener("change", () => {
  state.rootNote = Number(rootSelect.value);
  commitZones();
});
loopModeSelect.addEventListener("change", () => {
  state.loopMode = Number(loopModeSelect.value);
  commitZones();
});
loopXfade.addEventListener("input", () => {
  state.loopXfadeS = Number(loopXfade.value);
  loopXfadeValue.textContent = `${Math.round(state.loopXfadeS * 1000)} ms`;
  commitZonesSoon();
});
sliceGridSelect.addEventListener("change", () => {
  state.sliceGrid = sliceGridSelect.value;
  sliceSensitivityWrap.style.display = state.sliceGrid === "transient" ? "" : "none";
  detectSlices();
  commitZones();
});
sliceSensitivity.addEventListener("input", () => {
  state.sliceSensitivity = Number(sliceSensitivity.value);
  detectSlices();
  commitZonesSoon();
  render();
});
sliceBaseKeySelect.addEventListener("change", () => {
  state.sliceBaseKey = Number(sliceBaseKeySelect.value);
  commitZones();
});

// ---------------------------------------------------------------------------
// Waveform rendering & marker dragging.
// ---------------------------------------------------------------------------

function resizeCanvas() {
  const rect = waveWrap.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(rect.width * dpr));
  canvas.height = Math.max(1, Math.round(rect.height * dpr));
  render();
}
window.addEventListener("resize", resizeCanvas);

function frameToX(frame) {
  return (frame / Math.max(1, state.frames)) * canvas.width;
}

function xToFrame(x) {
  return clamp(Math.round((x / canvas.width) * state.frames), 0, state.frames);
}

function render() {
  const w = canvas.width;
  const h = canvas.height;
  ctx2d.clearRect(0, 0, w, h);
  if (!state.mono) return;

  const trimStart = Math.min(state.trimStart, state.trimEnd);
  const trimEnd = Math.max(state.trimStart, state.trimEnd);

  // Dim the trimmed-out regions.
  ctx2d.fillStyle = "rgba(10, 14, 20, 0.0)";
  ctx2d.fillRect(0, 0, w, h);

  // Loop region tint (classic with loop on).
  if (state.mode === "classic" && state.loopMode !== 0 && state.loopMode !== 4) {
    const lx = frameToX(Math.min(state.loopStart, state.loopEnd));
    const lw = frameToX(Math.max(state.loopStart, state.loopEnd)) - lx;
    ctx2d.fillStyle = "rgba(88, 185, 138, 0.10)";
    ctx2d.fillRect(lx, 0, lw, h);
  }

  // Waveform peaks.
  const mono = state.mono;
  const mid = h / 2;
  for (let x = 0; x < w; x++) {
    const f0 = Math.floor((x / w) * state.frames);
    const f1 = Math.max(f0 + 1, Math.floor(((x + 1) / w) * state.frames));
    let lo = 1;
    let hi = -1;
    const step = Math.max(1, Math.floor((f1 - f0) / 64));
    for (let f = f0; f < f1; f += step) {
      const s = mono[f];
      if (s < lo) lo = s;
      if (s > hi) hi = s;
    }
    const inTrim = f0 >= trimStart && f0 <= trimEnd;
    ctx2d.strokeStyle = inTrim ? "#58b98a" : "#2b3a46";
    ctx2d.beginPath();
    ctx2d.moveTo(x + 0.5, mid - hi * (mid - 4));
    ctx2d.lineTo(x + 0.5, mid - lo * (mid - 4));
    ctx2d.stroke();
  }

  // Center line.
  ctx2d.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx2d.beginPath();
  ctx2d.moveTo(0, mid);
  ctx2d.lineTo(w, mid);
  ctx2d.stroke();

  const dpr = window.devicePixelRatio || 1;
  ctx2d.font = `${10 * dpr}px sans-serif`;

  // Slice markers.
  if (state.mode === "slice") {
    state.sliceMarkers.forEach((m, i) => {
      if (m < trimStart || m >= trimEnd) return;
      const x = frameToX(m);
      ctx2d.strokeStyle = "#ffb054";
      ctx2d.beginPath();
      ctx2d.moveTo(x + 0.5, 0);
      ctx2d.lineTo(x + 0.5, h);
      ctx2d.stroke();
      ctx2d.fillStyle = "#ffb054";
      ctx2d.fillText(noteName(state.sliceBaseKey + i), x + 3 * dpr, 12 * dpr);
    });
  }

  // Trim handles.
  for (const f of [trimStart, trimEnd]) {
    const x = frameToX(f);
    ctx2d.fillStyle = "#9fb4c4";
    ctx2d.fillRect(x - 1.5 * dpr, 0, 3 * dpr, h);
    ctx2d.fillRect(x - 5 * dpr, 0, 10 * dpr, 6 * dpr);
  }

  // Loop handles.
  if (state.mode === "classic" && state.loopMode !== 0 && state.loopMode !== 4) {
    for (const f of [state.loopStart, state.loopEnd]) {
      const x = frameToX(f);
      ctx2d.fillStyle = "#58b98a";
      ctx2d.fillRect(x - 1 * dpr, 0, 2 * dpr, h);
      ctx2d.fillRect(x - 4 * dpr, h - 6 * dpr, 8 * dpr, 6 * dpr);
    }
  }
}

// Dragging.
let drag = null; // { kind: "trimStart"|"trimEnd"|"loopStart"|"loopEnd"|"slice", index }

function canvasX(e) {
  const rect = canvas.getBoundingClientRect();
  return ((e.clientX - rect.left) / rect.width) * canvas.width;
}

function hitTest(x) {
  const tol = 6 * (window.devicePixelRatio || 1);
  const near = (f) => Math.abs(frameToX(f) - x) < tol;
  if (state.mode === "slice") {
    for (let i = 0; i < state.sliceMarkers.length; i++) {
      if (near(state.sliceMarkers[i])) return { kind: "slice", index: i };
    }
  }
  if (state.mode === "classic" && state.loopMode !== 0 && state.loopMode !== 4) {
    if (near(state.loopStart)) return { kind: "loopStart" };
    if (near(state.loopEnd)) return { kind: "loopEnd" };
  }
  if (near(state.trimStart)) return { kind: "trimStart" };
  if (near(state.trimEnd)) return { kind: "trimEnd" };
  return null;
}

canvas.addEventListener("pointerdown", (e) => {
  if (!state.mono) return;
  drag = hitTest(canvasX(e));
  if (drag) canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  if (!state.mono) return;
  const x = canvasX(e);
  if (!drag) {
    canvas.style.cursor = hitTest(x) ? "ew-resize" : "crosshair";
    return;
  }
  const f = xToFrame(x);
  if (drag.kind === "trimStart") state.trimStart = Math.min(f, state.trimEnd - 16);
  else if (drag.kind === "trimEnd") state.trimEnd = Math.max(f, state.trimStart + 16);
  else if (drag.kind === "loopStart") state.loopStart = Math.min(f, state.loopEnd - 16);
  else if (drag.kind === "loopEnd") state.loopEnd = Math.max(f, state.loopStart + 16);
  else if (drag.kind === "slice") {
    state.sliceMarkers[drag.index] = f;
    state.sliceMarkers.sort((a, b) => a - b);
  }
  commitZonesSoon();
  render();
});

canvas.addEventListener("pointerup", (e) => {
  if (drag) {
    drag = null;
    commitZones();
  }
});

canvas.addEventListener("dblclick", (e) => {
  if (!state.mono || state.mode !== "slice") return;
  const x = canvasX(e);
  const hit = hitTest(x);
  if (hit && hit.kind === "slice") {
    state.sliceMarkers.splice(hit.index, 1);
  } else if (state.sliceMarkers.length < MAX_ZONES) {
    state.sliceMarkers.push(xToFrame(x));
    state.sliceMarkers.sort((a, b) => a - b);
  }
  commitZones();
});

// ---------------------------------------------------------------------------
// Keyboard.
// ---------------------------------------------------------------------------

const KEY_LO = 24; // C0
const KEY_HI = 96; // C6
const BLACK = new Set([1, 3, 6, 8, 10]);
let heldKey = -1;

function renderKeyboard() {
  keyboardEl.textContent = "";
  for (let key = KEY_LO; key <= KEY_HI; key++) {
    const el = document.createElement("div");
    const black = BLACK.has(key % 12);
    el.className = `key ${black ? "black" : "white"}`;
    el.dataset.key = key;
    const mapped = state.mappedKeys.get(key);
    if (mapped) {
      el.classList.add("mapped");
      if (mapped.root) el.classList.add("root");
    }
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

window.addEventListener("message", (event) => {
  if (!(event.data instanceof ArrayBuffer)) return;
  const status = decodeStatus(event.data);
  if (status) {
    state.pluginStatus = status;
    if (status.hasSample && !state.mono) {
      const secs = status.sampleRate > 0 ? status.frames / status.sampleRate : 0;
      sampleInfo.textContent =
        `Plugin: ${status.sampleRate.toFixed(0)} Hz · ${status.channels} ch · ` +
        `${secs.toFixed(2)} s · ${status.zones} zone${status.zones === 1 ? "" : "s"} ` +
        `(drop a file to replace)`;
    }
    statusEl.textContent = "CONNECTED";
    return;
  }
  const snapshot = decodeParamsSnapshot(event.data);
  if (snapshot) applySnapshot(snapshot);
});

// ---------------------------------------------------------------------------
// Boot.
// ---------------------------------------------------------------------------

setMode("classic");
renderKeyboard();
resizeCanvas();
loopXfadeValue.textContent = `${Math.round(state.loopXfadeS * 1000)} ms`;
sliceSensitivityWrap.style.display = "";
post(encodeReady());
