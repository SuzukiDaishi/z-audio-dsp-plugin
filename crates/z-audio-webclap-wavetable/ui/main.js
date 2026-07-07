// Z Audio Wave Synth — WebCLAP UI.
//
// Parameter edits ride the shared zui.js transport. On top of that the
// plugin pushes two binary packet kinds (see src/protocol.rs):
//   "ZWTW" u8 osc u16 n  n×f32   — morphed single-cycle waveform preview
//   "ZWTM" u8 voices 4×f32       — env1/env2/lfo1/lfo2 meter (~30 Hz)
// The oscillator canvases draw the plugin-rendered preview (the plugin is
// the source of truth for table content); the filter/env/LFO canvases are
// exact re-computations of the DSP formulas from the current params.

"use strict";

import { connect, createParams, fmt, clamp, setupCanvas, markConnected } from "./zui.js";

// --- Parameter ids (mirror crates/z-audio-webclap-wavetable/src/params.rs) --

const P = {
  MASTER: 500,
  POLYPHONY: 501,
  BEND_RANGE: 502,
  GLIDE: 503,
  OSC_A: 510,
  OSC_B: 530,
  // per-osc field offsets
  ENABLE: 0,
  TABLE: 1,
  WT_POS: 2,
  OCTAVE: 3,
  SEMI: 4,
  FINE: 5,
  UNISON: 6,
  UNI_DETUNE: 7,
  UNI_BLEND: 8,
  PHASE: 9,
  RAND_PHASE: 10,
  PAN: 11,
  LEVEL: 12,
  FILTER_ENABLE: 550,
  FILTER_TYPE: 551,
  CUTOFF: 552,
  RESO: 553,
  DRIVE: 554,
  KEYTRACK: 555,
  FILTER_MIX: 556,
  ROUTE_A: 557,
  ROUTE_B: 558,
  ENV1: 560,
  ENV2: 565,
  // env field offsets
  ATTACK: 0,
  DECAY: 1,
  SUSTAIN: 2,
  RELEASE: 3,
  CURVE: 4,
  LFO1: 570,
  LFO2: 574,
  // lfo field offsets
  WAVE: 0,
  RATE: 1,
  LFO_PHASE: 2,
  RETRIG: 3,
  MOD_BASE: 580,
  MOD_FIELDS: 3,
  MOD_SLOTS: 8,
};

const TABLE_NAMES = ["Basic Shapes", "PWM", "Harmonic Sweep", "Metal Bell"];
const MOD_SOURCES = ["None", "Env 2", "LFO 1", "LFO 2", "Velocity", "Note"];
const MOD_DESTS = [
  "None",
  "A WT Pos",
  "A Pitch",
  "A Level",
  "A Pan",
  "B WT Pos",
  "B Pitch",
  "B Level",
  "B Pan",
  "Cutoff",
  "Reso",
  "Master",
];

// --- Control definitions ----------------------------------------------------

function oscDefs(base, mountPrefix) {
  const at = (offset) => base + offset;
  return [
    { id: at(P.ENABLE), label: "On", kind: "toggle", min: 0, max: 1, default: base === P.OSC_A ? 1 : 0, mount: `#${mountPrefix}-enable` },
    {
      id: at(P.TABLE),
      label: "Table",
      kind: "select",
      min: 0,
      max: 3,
      default: 0,
      options: TABLE_NAMES,
      mount: `#${mountPrefix}-table`,
    },
    { id: at(P.WT_POS), label: "WT Pos", min: 0, max: 1, default: 0, fmt: fmt.pct, mount: `#${mountPrefix}-controls` },
    { id: at(P.UNISON), label: "Unison", min: 1, max: 8, default: 1, step: 1, fmt: fmt.int, mount: `#${mountPrefix}-controls` },
    { id: at(P.OCTAVE), label: "Oct", min: -4, max: 4, default: 0, step: 1, fmt: fmt.int, mount: `#${mountPrefix}-controls` },
    { id: at(P.UNI_DETUNE), label: "Detune", min: 0, max: 1, default: 0.25, fmt: fmt.pct, mount: `#${mountPrefix}-controls` },
    { id: at(P.SEMI), label: "Semi", min: -12, max: 12, default: 0, step: 1, fmt: fmt.int, mount: `#${mountPrefix}-controls` },
    { id: at(P.UNI_BLEND), label: "Blend", min: 0, max: 1, default: 0.75, fmt: fmt.pct, mount: `#${mountPrefix}-controls` },
    { id: at(P.FINE), label: "Fine", min: -100, max: 100, default: 0, fmt: (v) => `${v.toFixed(0)} ct`, mount: `#${mountPrefix}-controls` },
    { id: at(P.PHASE), label: "Phase", min: 0, max: 1, default: 0, fmt: fmt.pct, mount: `#${mountPrefix}-controls` },
    { id: at(P.PAN), label: "Pan", min: -1, max: 1, default: 0, fmt: (v) => (Math.abs(v) < 0.005 ? "C" : v < 0 ? `${(-v * 100).toFixed(0)}L` : `${(v * 100).toFixed(0)}R`), mount: `#${mountPrefix}-controls` },
    { id: at(P.RAND_PHASE), label: "Rand", min: 0, max: 1, default: 1, fmt: fmt.pct, mount: `#${mountPrefix}-controls` },
    { id: at(P.LEVEL), label: "Level", min: 0, max: 1, default: 0.75, fmt: fmt.pct, mount: `#${mountPrefix}-controls` },
  ];
}

function envDefs(base, mount) {
  const at = (offset) => base + offset;
  return [
    { id: at(P.ATTACK), label: "Attack", min: 0, max: 5, default: 0.005, fmt: fmt.s, mount },
    { id: at(P.DECAY), label: "Decay", min: 0, max: 5, default: 0.2, fmt: fmt.s, mount },
    { id: at(P.SUSTAIN), label: "Sustain", min: 0, max: 1, default: base === P.ENV1 ? 0.7 : 0.5, fmt: fmt.pct, mount },
    { id: at(P.RELEASE), label: "Release", min: 0, max: 5, default: 0.15, fmt: fmt.s, mount },
    { id: at(P.CURVE), label: "Curve", min: -1, max: 1, default: 0, fmt: fmt.plain, mount },
  ];
}

function lfoDefs(base, mount) {
  const at = (offset) => base + offset;
  return [
    {
      id: at(P.WAVE),
      label: "Wave",
      kind: "select",
      min: 0,
      max: 4,
      default: 0,
      options: ["Sin", "Tri", "Saw", "Sqr", "S&H"],
      mount,
    },
    { id: at(P.RATE), label: "Rate", min: 0.01, max: 20, default: 2, scale: "log", fmt: fmt.hzLfo, mount },
    { id: at(P.LFO_PHASE), label: "Phase", min: 0, max: 1, default: 0, fmt: fmt.pct, mount },
    { id: at(P.RETRIG), label: "Retrig", kind: "toggle", min: 0, max: 1, default: 1, mount },
  ];
}

const DEFS = [
  ...oscDefs(P.OSC_A, "osc-a"),
  ...oscDefs(P.OSC_B, "osc-b"),
  { id: P.FILTER_ENABLE, label: "On", kind: "toggle", min: 0, max: 1, default: 1, mount: "#filter-enable" },
  {
    id: P.FILTER_TYPE,
    label: "Type",
    kind: "select",
    min: 0,
    max: 3,
    default: 0,
    options: ["LP12", "LP24", "HP12", "BP12"],
    mount: "#filter-type",
  },
  { id: P.CUTOFF, label: "Cutoff", min: 20, max: 20000, default: 20000, scale: "log", fmt: fmt.hz, mount: "#filter-controls" },
  { id: P.RESO, label: "Reso", min: 0, max: 1, default: 0.15, fmt: fmt.pct, mount: "#filter-controls" },
  { id: P.DRIVE, label: "Drive", min: 0, max: 1, default: 0, fmt: fmt.pct, mount: "#filter-controls" },
  { id: P.KEYTRACK, label: "Keytrk", min: 0, max: 1, default: 0, fmt: fmt.pct, mount: "#filter-controls" },
  { id: P.FILTER_MIX, label: "Mix", min: 0, max: 1, default: 1, fmt: fmt.pct, mount: "#filter-controls" },
  { id: P.ROUTE_A, label: "A → Filt", kind: "toggle", min: 0, max: 1, default: 1, mount: "#filter-routes" },
  { id: P.ROUTE_B, label: "B → Filt", kind: "toggle", min: 0, max: 1, default: 1, mount: "#filter-routes" },
  ...envDefs(P.ENV1, "#env1-controls"),
  ...envDefs(P.ENV2, "#env2-controls"),
  ...lfoDefs(P.LFO1, "#lfo1-controls"),
  ...lfoDefs(P.LFO2, "#lfo2-controls"),
  { id: P.MASTER, label: "Master", min: 0, max: 1, default: 0.8, fmt: fmt.pct, mount: "#global-controls" },
  { id: P.POLYPHONY, label: "Voices", min: 1, max: 16, default: 8, step: 1, fmt: fmt.int, mount: "#global-controls" },
  { id: P.BEND_RANGE, label: "Bend", min: 0, max: 24, default: 2, step: 1, fmt: (v) => `${Math.round(v)} st`, mount: "#global-controls" },
  { id: P.GLIDE, label: "Glide", min: 0, max: 2, default: 0, fmt: fmt.s, mount: "#global-controls" },
];

// --- Mod matrix rows (hand-rolled: selects need long option lists) ----------

function buildMatrix(sendSet, onEdit) {
  const mountEl = document.getElementById("matrix");
  const rows = [];
  for (let slot = 0; slot < P.MOD_SLOTS; slot++) {
    const base = P.MOD_BASE + slot * P.MOD_FIELDS;
    const row = document.createElement("div");
    row.className = "matrix-row";

    const index = document.createElement("span");
    index.className = "slot-index";
    index.textContent = `${slot + 1}`;

    const source = document.createElement("select");
    for (const [i, name] of MOD_SOURCES.entries()) {
      source.add(new Option(name, i));
    }
    const dest = document.createElement("select");
    for (const [i, name] of MOD_DESTS.entries()) {
      dest.add(new Option(name, i));
    }
    const amount = document.createElement("input");
    amount.type = "range";
    amount.min = -100;
    amount.max = 100;
    amount.step = 1;
    amount.value = 0;
    const readout = document.createElement("span");
    readout.className = "readout";
    readout.textContent = "0 %";

    const engage = () => {
      const active = Number(source.value) > 0 && Number(dest.value) > 0;
      source.classList.toggle("engaged", active);
      dest.classList.toggle("engaged", active);
    };
    source.addEventListener("change", () => {
      sendSet(base + 0, Number(source.value));
      engage();
      onEdit();
    });
    dest.addEventListener("change", () => {
      sendSet(base + 1, Number(dest.value));
      engage();
      onEdit();
    });
    amount.addEventListener("input", () => {
      readout.textContent = `${amount.value} %`;
      sendSet(base + 2, Number(amount.value) / 100);
      onEdit();
    });
    amount.addEventListener("dblclick", () => {
      amount.value = 0;
      readout.textContent = "0 %";
      sendSet(base + 2, 0);
      onEdit();
    });

    row.append(index, source, dest, amount, readout);
    mountEl.append(row);
    rows.push({ base, source, dest, amount, readout, engage });
  }
  return {
    applySnapshot(map) {
      for (const row of rows) {
        if (map.has(row.base)) row.source.value = String(Math.round(map.get(row.base)));
        if (map.has(row.base + 1)) row.dest.value = String(Math.round(map.get(row.base + 1)));
        if (map.has(row.base + 2)) {
          const pct = Math.round(map.get(row.base + 2) * 100);
          row.amount.value = String(pct);
          row.readout.textContent = `${pct} %`;
        }
        row.engage();
      }
    },
  };
}

// --- Canvas drawing ----------------------------------------------------------

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

function drawWave(ctx, canvas, samples) {
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  if (!samples || samples.length === 0) return;
  const accent = css("--accent");
  const midY = h / 2;
  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  ctx.beginPath();
  for (let i = 0; i < samples.length; i++) {
    const x = (i / (samples.length - 1)) * w;
    const y = midY - samples[i] * (h * 0.42);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = Math.max(1.5, h / 60);
  ctx.stroke();
  // Soft fill under the curve, Serum-style.
  ctx.lineTo(w, midY);
  ctx.lineTo(0, midY);
  ctx.closePath();
  ctx.fillStyle = css("--accent-soft");
  ctx.fill();
}

/** Analog-prototype SVF magnitude for the response plot. */
function filterMagnitudeDb(freq, cutoff, reso, type) {
  const s = freq / Math.max(cutoff, 1);
  const k = 2 - 1.9 * clamp(reso, 0, 1);
  const denom = Math.sqrt((1 - s * s) ** 2 + (k * s) ** 2);
  let mag;
  if (type === 0) mag = 1 / denom; // LP12
  else if (type === 1) mag = 1 / (denom * denom); // LP24
  else if (type === 2) mag = (s * s) / denom; // HP12
  else mag = (k * s) / denom; // BP12 (k-scaled like the DSP)
  return 20 * Math.log10(Math.max(mag, 1e-6));
}

function drawFilter(ctx, canvas, store) {
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const cutoff = store.get(P.CUTOFF) || 20000;
  const reso = store.get(P.RESO) || 0;
  const type = Math.round(store.get(P.FILTER_TYPE) || 0);
  const enabled = (store.get(P.FILTER_ENABLE) || 0) >= 0.5;
  const lo = Math.log(20);
  const hi = Math.log(20000);
  const dbTop = 24;
  const dbBottom = -48;
  // Grid lines each decade.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.15)";
  ctx.lineWidth = 1;
  for (const f of [100, 1000, 10000]) {
    const x = ((Math.log(f) - lo) / (hi - lo)) * w;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
  }
  const zeroY = (dbTop / (dbTop - dbBottom)) * h;
  ctx.beginPath();
  ctx.moveTo(0, zeroY);
  ctx.lineTo(w, zeroY);
  ctx.stroke();

  ctx.beginPath();
  const steps = 160;
  for (let i = 0; i <= steps; i++) {
    const f = Math.exp(lo + ((hi - lo) * i) / steps);
    const db = filterMagnitudeDb(f, cutoff, reso, type);
    const x = (i / steps) * w;
    const y = ((dbTop - clamp(db, dbBottom, dbTop)) / (dbTop - dbBottom)) * h;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = enabled ? css("--accent") : "rgba(126, 147, 163, 0.5)";
  ctx.lineWidth = Math.max(1.5, h / 60);
  ctx.stroke();
}

/** The DSP's envelope curve shape: x^(2^(3c)). */
function envShape(x, curve) {
  return Math.pow(clamp(x, 0, 1), Math.pow(2, 3 * curve));
}

function drawEnv(ctx, canvas, store, base, level) {
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const a = store.get(base + P.ATTACK);
  const d = store.get(base + P.DECAY);
  const s = store.get(base + P.SUSTAIN);
  const r = store.get(base + P.RELEASE);
  const curve = store.get(base + P.CURVE);
  const sustainWidth = 0.18; // fixed visual plateau
  const total = Math.max(a + d + r, 1e-3);
  const ax = (a / total) * (1 - sustainWidth);
  const dx = (d / total) * (1 - sustainWidth);
  const rx = (r / total) * (1 - sustainWidth);
  const pad = h * 0.08;
  const y = (v) => h - pad - v * (h - 2 * pad);

  ctx.beginPath();
  ctx.moveTo(0, y(0));
  const steps = 32;
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    ctx.lineTo(t * ax * w, y(envShape(t, curve)));
  }
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    ctx.lineTo((ax + t * dx) * w, y(s + (1 - s) * envShape(1 - t, curve)));
  }
  ctx.lineTo((ax + dx + sustainWidth) * w, y(s));
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    ctx.lineTo((ax + dx + sustainWidth + t * rx) * w, y(s * envShape(1 - t, curve)));
  }
  ctx.strokeStyle = css("--accent");
  ctx.lineWidth = Math.max(1.5, h / 50);
  ctx.stroke();

  // Live level marker from the meter packet.
  if (level > 0.001) {
    ctx.fillStyle = css("--accent");
    ctx.globalAlpha = 0.9;
    ctx.beginPath();
    ctx.arc(w * 0.04, y(clamp(level, 0, 1)), Math.max(2.5, h / 30), 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
  }
}

function lfoShape(x, wave) {
  const t = x - Math.floor(x);
  if (wave === 0) return Math.sin(2 * Math.PI * t);
  if (wave === 1) return t < 0.5 ? 4 * t - 1 : 3 - 4 * t;
  if (wave === 2) return 2 * t - 1;
  if (wave === 3) return t < 0.5 ? 1 : -1;
  // S&H: deterministic pseudo-random staircase for display only.
  const stepIndex = Math.floor(x * 8);
  const r = Math.sin(stepIndex * 127.1) * 43758.5453;
  return (r - Math.floor(r)) * 2 - 1;
}

function drawLfo(ctx, canvas, store, base, liveValue) {
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const wave = Math.round(store.get(base + P.WAVE));
  const phase = store.get(base + P.LFO_PHASE);
  const midY = h / 2;
  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  ctx.beginPath();
  const steps = 128;
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const v = lfoShape(t + phase, wave);
    const x = t * w;
    const y = midY - v * (h * 0.38);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = css("--accent");
  ctx.lineWidth = Math.max(1.5, h / 50);
  ctx.stroke();

  // Live output marker on the left edge.
  ctx.fillStyle = css("--accent");
  ctx.beginPath();
  ctx.arc(w * 0.04, midY - clamp(liveValue, -1, 1) * (h * 0.38), Math.max(2.5, h / 30), 0, Math.PI * 2);
  ctx.fill();
}

// --- Wire everything up -------------------------------------------------------

const state = {
  waveA: new Float32Array(0),
  waveB: new Float32Array(0),
  env1: 0,
  env2: 0,
  lfo1: 0,
  lfo2: 0,
  voices: 0,
};

let store = null;
let matrix = null;

const canvases = {};

function redrawAll() {
  if (!store) return;
  canvases.oscA?.redraw();
  canvases.oscB?.redraw();
  canvases.filter?.redraw();
  canvases.env1?.redraw();
  canvases.env2?.redraw();
  canvases.lfo1?.redraw();
  canvases.lfo2?.redraw();
  const capA = document.getElementById("cap-osc-a");
  const capB = document.getElementById("cap-osc-b");
  if (capA) capA.textContent = TABLE_NAMES[Math.round(store.get(P.OSC_A + P.TABLE))] || "";
  if (capB) capB.textContent = TABLE_NAMES[Math.round(store.get(P.OSC_B + P.TABLE))] || "";
}

function handleBinary(ab) {
  if (!(ab instanceof ArrayBuffer) || ab.byteLength < 5) return;
  const view = new DataView(ab);
  const magic = String.fromCharCode(
    view.getUint8(0),
    view.getUint8(1),
    view.getUint8(2),
    view.getUint8(3),
  );
  if (magic === "ZWTW") {
    const oscB = view.getUint8(4) === 1;
    const n = view.getUint16(5, true);
    if (ab.byteLength < 7 + n * 4) return;
    const samples = new Float32Array(n);
    for (let i = 0; i < n; i++) samples[i] = view.getFloat32(7 + i * 4, true);
    if (oscB) state.waveB = samples;
    else state.waveA = samples;
    (oscB ? canvases.oscB : canvases.oscA)?.redraw();
  } else if (magic === "ZWTM") {
    state.voices = view.getUint8(4);
    state.env1 = view.getFloat32(5, true);
    state.env2 = view.getFloat32(9, true);
    state.lfo1 = view.getFloat32(13, true);
    state.lfo2 = view.getFloat32(17, true);
    const note = document.getElementById("voice-note");
    if (note) note.textContent = `${state.voices} voice${state.voices === 1 ? "" : "s"}`;
    canvases.env1?.redraw();
    canvases.env2?.redraw();
    canvases.lfo1?.redraw();
    canvases.lfo2?.redraw();
  }
}

const sendSet = connect({
  onSnapshot(map) {
    markConnected();
    if (store) store.applySnapshot(map);
    if (matrix) matrix.applySnapshot(map);
    redrawAll();
  },
  onMessage: handleBinary,
});

store = createParams(DEFS, sendSet, () => redrawAll(), "#global-controls");
matrix = buildMatrix(sendSet, () => redrawAll());

const canvasOscA = document.getElementById("viz-osc-a");
const canvasOscB = document.getElementById("viz-osc-b");
const canvasFilter = document.getElementById("viz-filter");
const canvasEnv1 = document.getElementById("viz-env1");
const canvasEnv2 = document.getElementById("viz-env2");
const canvasLfo1 = document.getElementById("viz-lfo1");
const canvasLfo2 = document.getElementById("viz-lfo2");

canvases.oscA = setupCanvas(canvasOscA, () =>
  drawWave(canvasOscA.getContext("2d"), canvasOscA, state.waveA),
);
canvases.oscB = setupCanvas(canvasOscB, () =>
  drawWave(canvasOscB.getContext("2d"), canvasOscB, state.waveB),
);
canvases.filter = setupCanvas(canvasFilter, () =>
  drawFilter(canvasFilter.getContext("2d"), canvasFilter, store),
);
canvases.env1 = setupCanvas(canvasEnv1, () =>
  drawEnv(canvasEnv1.getContext("2d"), canvasEnv1, store, P.ENV1, state.env1),
);
canvases.env2 = setupCanvas(canvasEnv2, () =>
  drawEnv(canvasEnv2.getContext("2d"), canvasEnv2, store, P.ENV2, state.env2),
);
canvases.lfo1 = setupCanvas(canvasLfo1, () =>
  drawLfo(canvasLfo1.getContext("2d"), canvasLfo1, store, P.LFO1, state.lfo1),
);
canvases.lfo2 = setupCanvas(canvasLfo2, () =>
  drawLfo(canvasLfo2.getContext("2d"), canvasLfo2, store, P.LFO2, state.lfo2),
);
