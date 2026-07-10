// Z Audio Wave Synth — WebCLAP UI (v2: knobs, direct canvas editing, keys).
//
// Parameter edits ride the shared zui.js transport. Binary packets (see
// src/protocol.rs):
//   plugin → UI  "ZWTW" u8 osc u16 n  n×f32      morphed cycle preview
//                "ZWTS" u8 osc u8 f u16 n f·n×f32 all frames (3D stack)
//                "ZWTM" u8 voices 4×f32           env/lfo meter (~30 Hz)
//   UI → plugin  "ZWTN" u8 on u8 key u8 velocity  preview keyboard
//
// Every control lives in a registry keyed by param id, so host
// automation / preset snapshots (`applySnapshot`) update knobs, selects,
// toggles and canvases alike.

"use strict";

import { connect, fmt, clamp, setupCanvas, markConnected } from "./zui.js";

// --- Parameter ids (mirror crates/z-audio-webclap-wavetable/src/params.rs) --

const P = {
  MASTER: 500,
  POLYPHONY: 501,
  BEND_RANGE: 502,
  GLIDE: 503,
  OSC_A: 510,
  OSC_B: 530,
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
  WARP_MODE: 13,
  WARP_AMT: 14,
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
  ATTACK: 0,
  DECAY: 1,
  SUSTAIN: 2,
  RELEASE: 3,
  CURVE: 4,
  LFO1: 570,
  LFO2: 574,
  WAVE: 0,
  RATE: 1,
  LFO_PHASE: 2,
  RETRIG: 3,
  MOD_BASE: 580,
  MOD_FIELDS: 3,
  MOD_SLOTS: 8,
  DIST_ENABLE: 604,
  DIST_MODE: 605,
  DIST_DRIVE: 606,
  DIST_MIX: 607,
};

const TABLE_NAMES = [
  "Basic Shapes",
  "PWM",
  "Harmonic Sweep",
  "Metal Bell",
  "Vowel Morph",
  "Growl",
  "FM Growl",
  "Sync Saw",
  "Digital Grit",
];
const WARP_MODES = [
  "Warp Off",
  "Bend +",
  "Bend −",
  "Sync",
  "Mirror",
  "Squeeze",
  "Quantize",
  "FM (other)",
  "RM (other)",
  "AM (other)",
];
const DIST_MODES = ["Tanh", "Hard", "Fold", "Sine", "Crush"];
const MOD_SOURCES = ["None", "Env 2", "LFO 1", "LFO 2", "Velocity", "Note", "Env 1"];
const MOD_COLORS = [
  "#7e93a3", // none (unused)
  "#ff8a5c", // env 2
  "#5cc8ff", // lfo 1
  "#c19bff", // lfo 2
  "#9dffb0", // velocity
  "#ff9bd4", // note
  "#f6c945", // env 1
];
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
  "A Warp",
  "B Warp",
  "Dist Drive",
  "A Detune",
  "B Detune",
];

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const $id = (id) => document.getElementById(id);

// --- Transport ----------------------------------------------------------------

const NATIVE = typeof window.sendToPlugin === "function";

function bytesToBase64(bytes) {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

function base64ToBytes(text) {
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function sendBinary(buffer) {
  if (NATIVE) {
    window.sendToPlugin({ type: "bin", data: bytesToBase64(new Uint8Array(buffer)) });
  } else {
    window.parent.postMessage(buffer, "*");
  }
}

function sendNote(on, key, velocity) {
  const buf = new ArrayBuffer(7);
  const b = new Uint8Array(buf);
  b[0] = 0x5a; // Z
  b[1] = 0x57; // W
  b[2] = 0x54; // T
  b[3] = 0x4e; // N
  b[4] = on ? 1 : 0;
  b[5] = key & 0x7f;
  b[6] = velocity & 0x7f;
  sendBinary(buf);
}

// --- Value scaling ---------------------------------------------------------------

function toNorm(def, v) {
  const x = clamp(v, def.min, def.max);
  if (def.scale === "log") {
    return (Math.log(x) - Math.log(def.min)) / (Math.log(def.max) - Math.log(def.min));
  }
  if (def.scale === "pow2") {
    return Math.sqrt((x - def.min) / (def.max - def.min));
  }
  return (x - def.min) / (def.max - def.min);
}

function fromNorm(def, t) {
  t = clamp(t, 0, 1);
  let v;
  if (def.scale === "log") {
    v = def.min * Math.pow(def.max / def.min, t);
  } else if (def.scale === "pow2") {
    v = def.min + (def.max - def.min) * t * t;
  } else {
    v = def.min + (def.max - def.min) * t;
  }
  if (def.step) v = Math.round(v / def.step) * def.step;
  return clamp(v, def.min, def.max);
}

// --- Registry --------------------------------------------------------------------

let sendSet = () => {};
const registry = new Map(); // id → { def, get, set }

function register(def, control) {
  registry.set(def.id, { def, ...control });
}

function val(id) {
  const c = registry.get(id);
  return c ? c.get() : 0;
}

/** Update a control's UI + notify the plugin (unless silent). */
function setParam(id, value, { silent = false } = {}) {
  const c = registry.get(id);
  if (c) {
    value = clamp(value, c.def.min, c.def.max);
    c.set(value);
  }
  if (!silent) sendSet(id, value);
  invalidate();
}

function applySnapshot(map) {
  for (const [id, value] of map) {
    const c = registry.get(id);
    if (c) c.set(clamp(value, c.def.min, c.def.max));
  }
  invalidate();
}

// --- Mod assignments (Serum-style: sources connect to parameters) -----------
//
// The 8 mod-matrix slots stay the storage model; drag-to-assign just writes
// the same slot params, so the matrix list view and presets stay in sync.

/** Engaged slots targeting `dest`: [{ base, src, amount }]. */
function modsForDest(dest) {
  const out = [];
  for (let s = 0; s < P.MOD_SLOTS; s++) {
    const base = P.MOD_BASE + s * P.MOD_FIELDS;
    const src = Math.round(val(base));
    if (src > 0 && Math.round(val(base + 1)) === dest) {
      out.push({ base, src, amount: val(base + 2) });
    }
  }
  return out;
}

/**
 * Connects `src` to `dest`, reusing an identical pair or claiming the first
 * free slot. Returns the slot base id, or -1 when the matrix is full.
 */
function assignMod(src, dest) {
  for (let s = 0; s < P.MOD_SLOTS; s++) {
    const base = P.MOD_BASE + s * P.MOD_FIELDS;
    if (Math.round(val(base)) === src && Math.round(val(base + 1)) === dest) {
      if (val(base + 2) === 0) setParam(base + 2, 0.5);
      return base;
    }
  }
  for (let s = 0; s < P.MOD_SLOTS; s++) {
    const base = P.MOD_BASE + s * P.MOD_FIELDS;
    if (Math.round(val(base)) === 0 || Math.round(val(base + 1)) === 0) {
      setParam(base, src);
      setParam(base + 1, dest);
      setParam(base + 2, 0.5);
      return base;
    }
  }
  return -1;
}

/** Clears one mod slot back to None/None/0. */
function clearMod(base) {
  setParam(base, 0);
  setParam(base + 1, 0);
  setParam(base + 2, 0);
}

/** Live source value for ring animation (null for velocity/note). */
function liveModValue(src) {
  switch (src) {
    case 1:
      return state.env2;
    case 2:
      return state.lfo1;
    case 3:
      return state.lfo2;
    case 6:
      return state.env1;
    default:
      return null;
  }
}

// --- Redraw scheduling --------------------------------------------------------------

let redrawQueued = false;
const redrawFns = [];

function invalidate() {
  if (redrawQueued) return;
  redrawQueued = true;
  requestAnimationFrame(() => {
    redrawQueued = false;
    for (const fn of redrawFns) fn();
  });
}

// --- Tooltip ---------------------------------------------------------------------

const tooltip = $id("tooltip");

function showTip(x, y, text) {
  tooltip.hidden = false;
  tooltip.textContent = text;
  const pad = 12;
  tooltip.style.left = `${Math.min(x + pad, window.innerWidth - tooltip.offsetWidth - 4)}px`;
  tooltip.style.top = `${Math.max(4, y - 26)}px`;
}

function hideTip() {
  tooltip.hidden = true;
}

// --- Knob component -----------------------------------------------------------------
//
// def: { id, label, min, max, default, fmt?, step?, scale?, bipolar?, small?, dest? }
// Vertical drag (Shift = fine), wheel, double-click = default.
// `dest` marks the knob as a mod destination: it accepts source-chip drops,
// draws one ring per assignment, and the ring region drags the mod depth.

function makeKnob(def) {
  const root = document.createElement("div");
  root.className = `knob${def.small ? " small" : ""}`;
  if (def.dest) root.dataset.dest = String(def.dest);
  const canvas = document.createElement("canvas");
  const label = document.createElement("span");
  label.className = "knob-label";
  label.textContent = def.label;
  root.append(canvas, label);

  let value = def.default;
  const format = def.fmt || fmt.plain;

  function draw() {
    const dpr = window.devicePixelRatio || 1;
    const sizeCss = canvas.clientHeight || (def.small ? 38 : 44);
    const size = Math.round(sizeCss * dpr);
    if (canvas.width !== size) {
      canvas.width = size;
      canvas.height = size;
    }
    const ctx = canvas.getContext("2d");
    const c = size / 2;
    const r = size * 0.36;
    const start = Math.PI * 0.75;
    const sweep = Math.PI * 1.5;
    const t = toNorm(def, value);
    ctx.clearRect(0, 0, size, size);

    // Body.
    const body = ctx.createLinearGradient(0, 0, 0, size);
    body.addColorStop(0, "#232c36");
    body.addColorStop(1, "#0d1218");
    ctx.beginPath();
    ctx.arc(c, c, r, 0, Math.PI * 2);
    ctx.fillStyle = body;
    ctx.fill();
    ctx.lineWidth = Math.max(1, size / 44);
    ctx.strokeStyle = "#05080c";
    ctx.stroke();

    // Track.
    ctx.beginPath();
    ctx.arc(c, c, r + size * 0.085, start, start + sweep);
    ctx.lineWidth = size * 0.055;
    ctx.lineCap = "round";
    ctx.strokeStyle = "rgba(126, 147, 163, 0.28)";
    ctx.stroke();

    // Value arc.
    const accent = css("--accent");
    ctx.beginPath();
    if (def.bipolar) {
      const mid = start + sweep / 2;
      const angle = start + sweep * t;
      ctx.arc(c, c, r + size * 0.085, Math.min(mid, angle), Math.max(mid, angle));
    } else {
      ctx.arc(c, c, r + size * 0.085, start, start + sweep * t);
    }
    ctx.strokeStyle = accent;
    ctx.shadowColor = accent;
    ctx.shadowBlur = size * 0.09;
    ctx.stroke();
    ctx.shadowBlur = 0;

    // Pointer.
    const angle = start + sweep * t;
    ctx.beginPath();
    ctx.moveTo(c + Math.cos(angle) * r * 0.35, c + Math.sin(angle) * r * 0.35);
    ctx.lineTo(c + Math.cos(angle) * r * 0.92, c + Math.sin(angle) * r * 0.92);
    ctx.lineWidth = size * 0.055;
    ctx.strokeStyle = accent;
    ctx.stroke();

    // Mod rings: one arc per assignment, from the value to its mod reach,
    // plus a live dot riding the source (Serum-style).
    if (def.dest) {
      const mods = modsForDest(def.dest);
      for (let i = 0; i < mods.length; i++) {
        const m = mods[i];
        const color = MOD_COLORS[m.src] || accent;
        const rr = r * (0.78 - i * 0.2);
        if (rr <= r * 0.2) break;
        const a0 = start + sweep * t;
        const a1 = start + sweep * clamp(t + m.amount, 0, 1);
        ctx.beginPath();
        ctx.arc(c, c, rr, Math.min(a0, a1), Math.max(a0, a1));
        ctx.lineWidth = size * 0.045;
        ctx.lineCap = "round";
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.85;
        ctx.stroke();
        ctx.globalAlpha = 1;

        const live = liveModValue(m.src);
        const dotT = clamp(t + m.amount * (live == null ? 0 : live), 0, 1);
        const dotA = start + sweep * dotT;
        ctx.beginPath();
        ctx.arc(c + Math.cos(dotA) * rr, c + Math.sin(dotA) * rr, size * 0.035, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
      }
    }
  }

  function emit(v) {
    value = clamp(v, def.min, def.max);
    draw();
    sendSet(def.id, value);
    invalidate();
  }

  // The ring region (outside the knob body) drags the depth of the first
  // assignment targeting this knob; the body drags the value itself.
  const ringModAt = (e) => {
    if (!def.dest) return null;
    const rect = canvas.getBoundingClientRect();
    const dist = Math.hypot(
      e.clientX - (rect.left + rect.width / 2),
      e.clientY - (rect.top + rect.height / 2),
    );
    if (dist / rect.width <= 0.3) return null;
    const mods = modsForDest(def.dest);
    return mods.length ? mods[0] : null;
  };
  const modTip = (e, m) =>
    showTip(
      e.clientX,
      e.clientY,
      `${MOD_SOURCES[m.src]} → ${def.label}: ${Math.round(val(m.base + 2) * 100)} %`,
    );

  let dragNorm = 0;
  let dragY = 0;
  let modDrag = null; // { base, amount }
  canvas.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    canvas.setPointerCapture(e.pointerId);
    dragY = e.clientY;
    const m = ringModAt(e);
    if (m) {
      modDrag = { base: m.base, amount: m.amount };
      root.classList.add("mod-dragging-ring");
      modTip(e, m);
      return;
    }
    root.classList.add("dragging");
    dragNorm = toNorm(def, value);
    showTip(e.clientX, e.clientY, `${def.label}: ${format(value)}`);
  });
  canvas.addEventListener("pointermove", (e) => {
    if (modDrag) {
      const range = e.shiftKey ? 1600 : 160;
      modDrag.amount = clamp(modDrag.amount + (dragY - e.clientY) / range, -1, 1);
      dragY = e.clientY;
      setParam(modDrag.base + 2, modDrag.amount);
      modTip(e, { base: modDrag.base, src: Math.round(val(modDrag.base)) });
      draw();
    } else if (root.classList.contains("dragging")) {
      const range = e.shiftKey ? 1600 : 160;
      dragNorm = clamp(dragNorm + (dragY - e.clientY) / range, 0, 1);
      dragY = e.clientY;
      emit(fromNorm(def, dragNorm));
      showTip(e.clientX, e.clientY, `${def.label}: ${format(value)}`);
    } else if (e.buttons === 0) {
      const m = ringModAt(e);
      if (m) modTip(e, m);
      else showTip(e.clientX, e.clientY, `${def.label}: ${format(value)}`);
    }
  });
  canvas.addEventListener("pointerup", (e) => {
    canvas.releasePointerCapture(e.pointerId);
    root.classList.remove("dragging");
    root.classList.remove("mod-dragging-ring");
    modDrag = null;
    hideTip();
  });
  canvas.addEventListener("pointerleave", () => {
    if (!root.classList.contains("dragging") && !modDrag) hideTip();
  });
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const stepNorm = def.step
      ? def.step / (def.max - def.min)
      : e.shiftKey
        ? 0.002
        : 0.02;
    const t = clamp(toNorm(def, value) - Math.sign(e.deltaY) * stepNorm, 0, 1);
    emit(fromNorm(def, t));
    showTip(e.clientX, e.clientY, `${def.label}: ${format(value)}`);
  });
  canvas.addEventListener("dblclick", (e) => {
    const m = ringModAt(e);
    if (m) {
      clearMod(m.base);
      draw();
      hideTip();
      return;
    }
    emit(def.default);
  });

  requestAnimationFrame(draw);
  // Live mod-ring animation rides the shared invalidate() pass (driven by
  // the ~30 Hz meter packets) but only when this knob is modulated.
  if (def.dest) {
    redrawFns.push(() => {
      if (modsForDest(def.dest).length) draw();
    });
  }
  register(def, {
    get: () => value,
    set: (v) => {
      value = v;
      draw();
    },
  });
  return root;
}

// --- Switch / segmented / table picker ------------------------------------------------

function makeSwitch(def) {
  const root = document.createElement("span");
  root.className = "switch";
  const label = document.createElement("span");
  label.className = "switch-label";
  label.textContent = def.label;
  const pill = document.createElement("span");
  pill.className = "pill";
  root.append(label, pill);

  let value = def.default;
  const render = () => root.classList.toggle("on", value >= 0.5);
  root.addEventListener("click", () => {
    value = value >= 0.5 ? 0 : 1;
    render();
    sendSet(def.id, value);
    invalidate();
  });
  render();
  register(def, {
    get: () => value,
    set: (v) => {
      value = v;
      render();
    },
  });
  return root;
}

function makeSegmented(def) {
  const root = document.createElement("div");
  root.className = "segmented";
  let value = def.default;
  const buttons = def.options.map((name, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = name;
    b.addEventListener("click", () => {
      value = i;
      render();
      sendSet(def.id, i);
      invalidate();
    });
    root.append(b);
    return b;
  });
  function render() {
    buttons.forEach((b, i) => b.classList.toggle("active", i === Math.round(value)));
  }
  render();
  register(def, {
    get: () => value,
    set: (v) => {
      value = v;
      render();
    },
  });
  return root;
}

/** Registered <select> — used for the per-osc warp mode picker. */
function makeSelect(def) {
  const select = document.createElement("select");
  for (const [i, name] of def.options.entries()) select.add(new Option(name, i));
  let value = def.default;
  const render = () => {
    select.value = String(Math.round(value));
    select.classList.toggle("engaged", Math.round(value) > 0);
  };
  select.addEventListener("change", () => {
    value = Number(select.value);
    render();
    sendSet(def.id, value);
    invalidate();
  });
  render();
  register(def, {
    get: () => value,
    set: (v) => {
      value = v;
      render();
    },
  });
  return select;
}

function makeTablePicker(def) {
  const root = document.createElement("div");
  root.className = "table-picker";
  const prev = document.createElement("button");
  prev.type = "button";
  prev.textContent = "‹";
  const name = document.createElement("span");
  name.className = "table-name";
  const next = document.createElement("button");
  next.type = "button";
  next.textContent = "›";
  root.append(prev, name, next);

  let value = def.default;
  const render = () => {
    name.textContent = def.options[Math.round(value)] || "?";
  };
  const bump = (dir) => {
    value = (Math.round(value) + dir + def.options.length) % def.options.length;
    render();
    sendSet(def.id, value);
    invalidate();
  };
  prev.addEventListener("click", () => bump(-1));
  next.addEventListener("click", () => bump(1));
  render();
  register(def, {
    get: () => value,
    set: (v) => {
      value = v;
      render();
    },
  });
  return root;
}

// --- Build controls ---------------------------------------------------------------------

const fmtCt = (v) => `${v.toFixed(0)} ct`;
const fmtSt = (v) => `${Math.round(v)} st`;
const fmtPan = (v) =>
  Math.abs(v) < 0.005 ? "C" : v < 0 ? `${(-v * 100).toFixed(0)}L` : `${(v * 100).toFixed(0)}R`;

function buildOsc(base, prefix) {
  const at = (o) => base + o;
  $id(`${prefix}-enable`).append(
    makeSwitch({ id: at(P.ENABLE), label: "On", min: 0, max: 1, default: base === P.OSC_A ? 1 : 0 }),
  );
  $id(`${prefix}-table`).replaceWith(
    (() => {
      const picker = makeTablePicker({
        id: at(P.TABLE),
        options: TABLE_NAMES,
        min: 0,
        max: TABLE_NAMES.length - 1,
        default: 0,
      });
      picker.id = `${prefix}-table`;
      return picker;
    })(),
  );
  const isA = base === P.OSC_A;
  $id(`${prefix}-warp`).append(
    makeSelect({
      id: at(P.WARP_MODE),
      options: WARP_MODES,
      min: 0,
      max: WARP_MODES.length - 1,
      default: 0,
    }),
  );
  const knobs = $id(`${prefix}-knobs`);
  const defs = [
    { id: at(P.WT_POS), label: "WT Pos", min: 0, max: 1, default: 0, fmt: fmt.pct, dest: isA ? 1 : 5 },
    { id: at(P.WARP_AMT), label: "Warp", min: 0, max: 1, default: 0, fmt: fmt.pct, dest: isA ? 12 : 13 },
    { id: at(P.UNISON), label: "Unison", min: 1, max: 8, default: 1, step: 1, fmt: fmt.int },
    { id: at(P.UNI_DETUNE), label: "Detune", min: 0, max: 1, default: 0.25, fmt: fmt.pct, dest: isA ? 15 : 16 },
    { id: at(P.UNI_BLEND), label: "Blend", min: 0, max: 1, default: 0.75, fmt: fmt.pct },
    { id: at(P.PHASE), label: "Phase", min: 0, max: 1, default: 0, fmt: fmt.pct },
    { id: at(P.RAND_PHASE), label: "Rand", min: 0, max: 1, default: 1, fmt: fmt.pct },
    { id: at(P.OCTAVE), label: "Oct", min: -4, max: 4, default: 0, step: 1, bipolar: true, fmt: fmt.int },
    { id: at(P.SEMI), label: "Semi", min: -12, max: 12, default: 0, step: 1, bipolar: true, fmt: fmt.int, dest: isA ? 2 : 6 },
    { id: at(P.FINE), label: "Fine", min: -100, max: 100, default: 0, bipolar: true, fmt: fmtCt },
    { id: at(P.PAN), label: "Pan", min: -1, max: 1, default: 0, bipolar: true, fmt: fmtPan, dest: isA ? 4 : 8 },
    { id: at(P.LEVEL), label: "Level", min: 0, max: 1, default: 0.75, fmt: fmt.pct, dest: isA ? 3 : 7 },
  ];
  for (const def of defs) {
    knobs.append(makeKnob({ ...def, small: true }));
  }
}

buildOsc(P.OSC_A, "osc-a");
buildOsc(P.OSC_B, "osc-b");

$id("filter-enable").append(
  makeSwitch({ id: P.FILTER_ENABLE, label: "On", min: 0, max: 1, default: 1 }),
);
$id("filter-type").append(
  makeSegmented({
    id: P.FILTER_TYPE,
    options: ["LP12", "LP24", "HP12", "BP12", "NT12", "CB+", "CB−", "FMT"],
    min: 0,
    max: 7,
    default: 0,
  }),
);
for (const def of [
  { id: P.CUTOFF, label: "Cutoff", min: 20, max: 20000, default: 20000, scale: "log", fmt: fmt.hz, dest: 9 },
  { id: P.RESO, label: "Reso", min: 0, max: 1, default: 0.15, fmt: fmt.pct, dest: 10 },
  { id: P.DRIVE, label: "Drive", min: 0, max: 1, default: 0, fmt: fmt.pct },
  { id: P.KEYTRACK, label: "Keytrk", min: 0, max: 1, default: 0, fmt: fmt.pct },
  { id: P.FILTER_MIX, label: "Mix", min: 0, max: 1, default: 1, fmt: fmt.pct },
]) {
  $id("filter-knobs").append(makeKnob(def));
}
$id("route-a").append(makeSwitch({ id: P.ROUTE_A, label: "A → Filt", min: 0, max: 1, default: 1 }));
$id("route-b").append(makeSwitch({ id: P.ROUTE_B, label: "B → Filt", min: 0, max: 1, default: 1 }));

function buildEnv(base, mountId, sustainDefault) {
  const at = (o) => base + o;
  const knobs = $id(mountId);
  for (const def of [
    { id: at(P.ATTACK), label: "Atk", min: 0, max: 5, default: 0.005, scale: "pow2", fmt: fmt.s },
    { id: at(P.DECAY), label: "Dec", min: 0, max: 5, default: 0.2, scale: "pow2", fmt: fmt.s },
    { id: at(P.SUSTAIN), label: "Sus", min: 0, max: 1, default: sustainDefault, fmt: fmt.pct },
    { id: at(P.RELEASE), label: "Rel", min: 0, max: 5, default: 0.15, scale: "pow2", fmt: fmt.s },
    { id: at(P.CURVE), label: "Curve", min: -1, max: 1, default: 0, bipolar: true, fmt: fmt.plain },
  ]) {
    knobs.append(makeKnob({ ...def, small: true }));
  }
}

buildEnv(P.ENV1, "env1-knobs", 0.7);
buildEnv(P.ENV2, "env2-knobs", 0.5);

function buildLfo(base, prefix) {
  const at = (o) => base + o;
  $id(`${prefix}-retrig`).append(
    makeSwitch({ id: at(P.RETRIG), label: "Retrig", min: 0, max: 1, default: 1 }),
  );
  $id(`${prefix}-wave`).append(
    makeSegmented({
      id: at(P.WAVE),
      options: ["Sin", "Tri", "Saw", "Sqr", "S&H"],
      min: 0,
      max: 4,
      default: 0,
    }),
  );
  const knobs = $id(`${prefix}-knobs`);
  knobs.append(
    makeKnob({
      id: at(P.RATE),
      label: "Rate",
      min: 0.01,
      max: 50,
      default: 2,
      scale: "log",
      fmt: fmt.hzLfo,
      small: true,
    }),
    makeKnob({
      id: at(P.LFO_PHASE),
      label: "Phase",
      min: 0,
      max: 1,
      default: 0,
      fmt: fmt.pct,
      small: true,
    }),
  );
}

buildLfo(P.LFO1, "lfo1");
buildLfo(P.LFO2, "lfo2");

$id("master-mount").append(
  makeKnob({ id: P.MASTER, label: "Master", min: 0, max: 1, default: 0.8, fmt: fmt.pct, dest: 11 }),
);

// --- Distortion (global, post-voice-sum) --------------------------------------

$id("dist-enable").append(
  makeSwitch({ id: P.DIST_ENABLE, label: "On", min: 0, max: 1, default: 0 }),
);
$id("dist-mode").append(
  makeSegmented({
    id: P.DIST_MODE,
    options: DIST_MODES,
    min: 0,
    max: DIST_MODES.length - 1,
    default: 0,
  }),
);
$id("dist-knobs").append(
  makeKnob({ id: P.DIST_DRIVE, label: "Drive", min: 0, max: 1, default: 0.3, fmt: fmt.pct, dest: 14 }),
  makeKnob({ id: P.DIST_MIX, label: "Mix", min: 0, max: 1, default: 1, fmt: fmt.pct }),
);
for (const def of [
  { id: P.POLYPHONY, label: "Voices", min: 1, max: 16, default: 8, step: 1, fmt: fmt.int },
  { id: P.BEND_RANGE, label: "Bend", min: 0, max: 24, default: 2, step: 1, fmt: fmtSt },
  { id: P.GLIDE, label: "Glide", min: 0, max: 2, default: 0, scale: "pow2", fmt: fmt.s },
]) {
  $id("global-knobs").append(makeKnob(def));
}

// --- Factory presets ------------------------------------------------------------
//
// Each preset is a diff against Init (every registered param back at its
// default), so the maps stay valid as the parameter surface grows. The
// "Vowel Growl" map is mirrored in src/lib.rs tests to guard id drift.

const PRESETS = [
  { name: "Init", set: {} },
  {
    // Growl table through the formant filter, LFO1 sweeping the vowel.
    name: "Vowel Growl",
    set: {
      511: 5, 512: 0.3, 513: -1, 516: 5, 517: 0.18,
      551: 7, 552: 900, 553: 0.5,
      571: 4.5,
      580: 2, 581: 9, 582: 0.6,
      583: 2, 584: 1, 585: 0.35,
      604: 1, 605: 0, 606: 0.45, 607: 0.8,
    },
  },
  {
    // FM Growl table, osc B as a silent FM modulator two octaves down,
    // Env2 kicks the FM depth per note, LFO2 talks through the formants.
    name: "FM Talk Bass",
    set: {
      511: 6, 512: 0.4, 513: -1, 523: 7, 524: 0.5,
      530: 1, 533: -2, 542: 0,
      551: 7, 552: 600, 553: 0.4,
      566: 0.35, 567: 0.2,
      575: 6,
      580: 3, 581: 9, 582: 0.5,
      583: 1, 584: 12, 585: 0.6,
      586: 3, 587: 1, 588: 0.3,
      604: 1, 605: 0, 606: 0.35, 607: 1,
    },
  },
  {
    // Detuned sync-saw stack over a growl layer, dark LP24, fold dist.
    name: "Reese Sync",
    set: {
      511: 7, 512: 0.25, 513: -1, 516: 7, 517: 0.35, 518: 0.9,
      523: 3, 524: 0.3,
      530: 1, 531: 5, 532: 0.2, 533: -1, 535: 12, 542: 0.55,
      551: 1, 552: 400, 553: 0.25,
      571: 0.8,
      580: 2, 581: 9, 582: 0.4,
      604: 1, 605: 2, 606: 0.5, 607: 0.7,
    },
  },
  {
    // Digital Grit through a key-tracked comb, morph wobble, hard clip.
    name: "Grit Comb",
    set: {
      511: 8, 512: 0.5, 513: -1,
      551: 5, 552: 110, 553: 0.6, 555: 1,
      571: 3,
      580: 2, 581: 1, 582: 0.5,
      604: 1, 605: 1, 606: 0.4, 607: 0.85,
    },
  },
];

function applyPreset(index) {
  const preset = PRESETS[index];
  if (!preset) return;
  for (const [id, c] of registry) setParam(id, c.def.default);
  for (const [id, v] of Object.entries(preset.set)) setParam(Number(id), v);
}

{
  const select = document.createElement("select");
  for (const [i, p] of PRESETS.entries()) select.add(new Option(p.name, i));
  select.addEventListener("change", () => applyPreset(Number(select.value)));
  $id("preset-mount").append(select);
}

// --- Mod matrix -----------------------------------------------------------------------

function buildMatrix() {
  const mountEl = $id("matrix");
  for (let slot = 0; slot < P.MOD_SLOTS; slot++) {
    const base = P.MOD_BASE + slot * P.MOD_FIELDS;
    const row = document.createElement("div");
    row.className = "matrix-row";

    const index = document.createElement("span");
    index.className = "slot-index";
    index.textContent = `${slot + 1}`;

    const source = document.createElement("select");
    for (const [i, n] of MOD_SOURCES.entries()) source.add(new Option(n, i));
    const dest = document.createElement("select");
    for (const [i, n] of MOD_DESTS.entries()) dest.add(new Option(n, i));

    const amountWrap = document.createElement("span");
    amountWrap.className = "amount-wrap";
    const amount = document.createElement("input");
    amount.type = "range";
    amount.min = -100;
    amount.max = 100;
    amount.step = 1;
    amount.value = 0;
    amountWrap.append(amount);

    const readout = document.createElement("span");
    readout.className = "readout";
    readout.textContent = "0 %";

    const engage = () =>
      row.classList.toggle(
        "engaged",
        Number(source.value) > 0 && Number(dest.value) > 0 && Number(amount.value) !== 0,
      );

    source.addEventListener("change", () => {
      sendSet(base + 0, Number(source.value));
      engage();
    });
    dest.addEventListener("change", () => {
      sendSet(base + 1, Number(dest.value));
      engage();
    });
    amount.addEventListener("input", () => {
      readout.textContent = `${amount.value} %`;
      sendSet(base + 2, Number(amount.value) / 100);
      engage();
    });
    amount.addEventListener("dblclick", () => {
      amount.value = 0;
      readout.textContent = "0 %";
      sendSet(base + 2, 0);
      engage();
    });

    row.append(index, source, dest, amountWrap, readout);
    mountEl.append(row);

    register(
      { id: base + 0, min: 0, max: MOD_SOURCES.length - 1, default: 0 },
      {
        get: () => Number(source.value),
        set: (v) => {
          source.value = String(Math.round(v));
          engage();
        },
      },
    );
    register(
      { id: base + 1, min: 0, max: MOD_DESTS.length - 1, default: 0 },
      {
        get: () => Number(dest.value),
        set: (v) => {
          dest.value = String(Math.round(v));
          engage();
        },
      },
    );
    register(
      { id: base + 2, min: -1, max: 1, default: 0 },
      {
        get: () => Number(amount.value) / 100,
        set: (v) => {
          const pct = Math.round(v * 100);
          amount.value = String(pct);
          readout.textContent = `${pct} %`;
          engage();
        },
      },
    );
  }
}

buildMatrix();

// --- Drag-to-assign (Serum-style source chips) ------------------------------
//
// Pointer-drag a source chip onto anything with data-dest (knobs, the osc
// wave views, the filter view) to connect it through the mod matrix.

const SVG_NS = "http://www.w3.org/2000/svg";

function beginModDrag(chip, src, event) {
  event.preventDefault();
  chip.setPointerCapture(event.pointerId);
  document.body.classList.add("mod-drag");

  const color = MOD_COLORS[src] || css("--accent");
  const overlay = document.createElementNS(SVG_NS, "svg");
  overlay.setAttribute("class", "mod-drag-overlay");
  const line = document.createElementNS(SVG_NS, "line");
  line.setAttribute("stroke", color);
  line.setAttribute("stroke-width", "2");
  line.setAttribute("stroke-dasharray", "6 4");
  const dot = document.createElementNS(SVG_NS, "circle");
  dot.setAttribute("r", "5");
  dot.setAttribute("fill", color);
  overlay.append(line, dot);
  document.body.append(overlay);

  const rect = chip.getBoundingClientRect();
  const x0 = rect.left + rect.width / 2;
  const y0 = rect.top + rect.height / 2;
  line.setAttribute("x1", String(x0));
  line.setAttribute("y1", String(y0));

  let hover = null;
  const targetAt = (e) => {
    const el = document.elementFromPoint(e.clientX, e.clientY);
    return el && el.closest ? el.closest("[data-dest]") : null;
  };
  const track = (e) => {
    line.setAttribute("x2", String(e.clientX));
    line.setAttribute("y2", String(e.clientY));
    dot.setAttribute("cx", String(e.clientX));
    dot.setAttribute("cy", String(e.clientY));
    const target = targetAt(e);
    if (target !== hover) {
      if (hover) hover.classList.remove("mod-drop-hover");
      hover = target;
      if (hover) hover.classList.add("mod-drop-hover");
    }
  };
  track(event);

  const finish = (e) => {
    if (chip.hasPointerCapture?.(e.pointerId)) chip.releasePointerCapture(e.pointerId);
    overlay.remove();
    document.body.classList.remove("mod-drag");
    if (hover) hover.classList.remove("mod-drop-hover");
    chip.removeEventListener("pointermove", track);
    chip.removeEventListener("pointerup", drop);
    chip.removeEventListener("pointercancel", finish);
  };
  const drop = (e) => {
    const target = targetAt(e);
    finish(e);
    if (!target) return;
    const dest = Number(target.dataset.dest);
    const base = assignMod(src, dest);
    showTip(
      e.clientX,
      e.clientY,
      base < 0
        ? "Mod matrix full (8 slots)"
        : `${MOD_SOURCES[src]} → ${MOD_DESTS[dest]}: ${Math.round(val(base + 2) * 100)} %`,
    );
    setTimeout(hideTip, 1400);
  };
  chip.addEventListener("pointermove", track);
  chip.addEventListener("pointerup", drop);
  chip.addEventListener("pointercancel", finish);
}

for (const chip of document.querySelectorAll(".mod-chip")) {
  const src = Number(chip.dataset.src);
  chip.style.setProperty("--chip-color", MOD_COLORS[src] || "#7e93a3");
  chip.addEventListener("pointerdown", (e) => beginModDrag(chip, src, e));
}

// --- Live packet state --------------------------------------------------------------------

const state = {
  wave: [new Float32Array(0), new Float32Array(0)],
  stack: [[], []], // per-osc arrays of Float32Array frames
  env1: 0,
  env2: 0,
  lfo1: 0,
  lfo2: 0,
  voices: 0,
};

// --- Oscillator stack view -------------------------------------------------------------------

function drawOscStack(canvas, oscIndex, base) {
  const ctx = canvas.getContext("2d");
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const accent = css("--accent");
  const enabled = val(base + P.ENABLE) >= 0.5;
  const pos = val(base + P.WT_POS);
  const frames = state.stack[oscIndex];
  const nFrames = frames.length;

  const ox = w * 0.16;
  const oy = h * 0.34;
  const x0 = w * 0.05;
  const baseline = h * 0.72;
  const amp = h * 0.2;
  const span = w * 0.62;

  const polyline = (data, t, color, width, glow) => {
    const dx = x0 + t * ox;
    const dy = baseline - t * oy;
    ctx.beginPath();
    for (let i = 0; i < data.length; i++) {
      const x = dx + (i / (data.length - 1)) * span;
      const y = dy - data[i] * amp;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = color;
    ctx.lineWidth = width;
    if (glow) {
      ctx.shadowColor = accent;
      ctx.shadowBlur = h / 26;
    }
    ctx.stroke();
    ctx.shadowBlur = 0;
  };

  // Back-to-front stack of table frames.
  if (nFrames > 1) {
    for (let f = nFrames - 1; f >= 0; f--) {
      const t = f / (nFrames - 1);
      const near = 1 - Math.min(Math.abs(t - pos) * (nFrames - 1), 1);
      const alpha = 0.1 + near * 0.22;
      polyline(
        frames[f],
        t,
        enabled ? `rgba(163, 224, 74, ${alpha})` : `rgba(126, 147, 163, ${alpha})`,
        Math.max(1, h / 120),
        false,
      );
    }
  }

  // The live morphed cycle rides at its interpolated depth.
  const wave = state.wave[oscIndex];
  if (wave.length > 1) {
    polyline(
      wave,
      pos,
      enabled ? accent : "rgba(126, 147, 163, 0.7)",
      Math.max(1.6, h / 62),
      enabled,
    );
  }

  // WT position rail along the bottom.
  const railY = h - Math.max(4, h * 0.045);
  ctx.strokeStyle = "rgba(126, 147, 163, 0.3)";
  ctx.lineWidth = Math.max(1.5, h / 90);
  ctx.beginPath();
  ctx.moveTo(w * 0.05, railY);
  ctx.lineTo(w * 0.95, railY);
  ctx.stroke();
  ctx.fillStyle = accent;
  ctx.beginPath();
  ctx.arc(w * (0.05 + 0.9 * pos), railY, Math.max(3, h / 40), 0, Math.PI * 2);
  ctx.fill();
}

function wireOscCanvas(canvasId, base) {
  const canvas = $id(canvasId);
  const oscIndex = base === P.OSC_A ? 0 : 1;
  canvas.dataset.dest = base === P.OSC_A ? "1" : "5"; // drop target: WT Pos
  const view = setupCanvas(canvas, () => drawOscStack(canvas, oscIndex, base));
  redrawFns.push(view.redraw);

  const posFromEvent = (e) => {
    const rect = canvas.getBoundingClientRect();
    return clamp((e.clientX - rect.left) / rect.width / 0.9 - 0.055, 0, 1);
  };
  canvas.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    canvas.setPointerCapture(e.pointerId);
    setParam(base + P.WT_POS, posFromEvent(e));
    showTip(e.clientX, e.clientY, `WT Pos: ${fmt.pct(val(base + P.WT_POS))}`);
  });
  canvas.addEventListener("pointermove", (e) => {
    if (e.buttons & 1) {
      setParam(base + P.WT_POS, posFromEvent(e));
      showTip(e.clientX, e.clientY, `WT Pos: ${fmt.pct(val(base + P.WT_POS))}`);
    }
  });
  canvas.addEventListener("pointerup", (e) => {
    canvas.releasePointerCapture(e.pointerId);
    hideTip();
  });
  canvas.addEventListener("dblclick", () => setParam(base + P.WT_POS, 0));
}

wireOscCanvas("viz-osc-a", P.OSC_A);
wireOscCanvas("viz-osc-b", P.OSC_B);

// --- Filter view ------------------------------------------------------------------------------

const FREQ_LO = Math.log(20);
const FREQ_HI = Math.log(20000);

// Mirrors src/wavetable.rs VOWEL_FORMANTS / vowel_at for the formant curve.
const VOWEL_FORMANTS = [
  [730, 1090, 2440],
  [530, 1840, 2480],
  [270, 2290, 3010],
  [570, 840, 2410],
  [300, 870, 2240],
];
const VOWEL_AMPS = [1.0, 0.63, 0.32];

function vowelFreqsAt(pos) {
  const x = clamp(pos, 0, 1) * (VOWEL_FORMANTS.length - 1);
  const i0 = Math.min(Math.floor(x), VOWEL_FORMANTS.length - 2);
  const t = x - i0;
  return VOWEL_FORMANTS[i0].map((a, k) => a + (VOWEL_FORMANTS[i0 + 1][k] - a) * t);
}

function filterMagnitudeDb(freq, cutoff, reso, type) {
  const k = 2 - 1.9 * clamp(reso, 0, 1);
  const svf = (fc, kind) => {
    const s = freq / Math.max(fc, 1);
    const denom = Math.sqrt((1 - s * s) ** 2 + (k * s) ** 2);
    if (kind === "lp") return 1 / denom;
    if (kind === "hp") return (s * s) / denom;
    if (kind === "bp") return (k * s) / denom;
    return Math.abs(1 - s * s) / denom; // notch
  };
  let mag;
  if (type === 0) mag = svf(cutoff, "lp");
  else if (type === 1) mag = svf(cutoff, "lp") ** 2;
  else if (type === 2) mag = svf(cutoff, "hp");
  else if (type === 3) mag = svf(cutoff, "bp");
  else if (type === 4) mag = svf(cutoff, "nt");
  else if (type === 5 || type === 6) {
    // Feedback comb ripple: |H| = comp / |1 ∓ fb·e^{-jωd}| with d = sr/fc.
    const fb = 0.5 + 0.48 * clamp(reso, 0, 1);
    const phase = (2 * Math.PI * freq) / Math.max(cutoff, 1);
    const sign = type === 5 ? 1 : -1;
    const re = 1 - sign * fb * Math.cos(phase);
    const im = sign * fb * Math.sin(phase);
    mag = (1 - 0.5 * fb) / Math.max(Math.hypot(re, im), 1e-4);
  } else {
    // Formant: sum of three band-passes at the vowel's F1/F2/F3.
    const t = (Math.log(clamp(cutoff, 200, 4000)) - Math.log(200)) / (Math.log(4000) - Math.log(200));
    mag = vowelFreqsAt(t).reduce((acc, fc, i) => acc + svf(fc, "bp") * VOWEL_AMPS[i], 0);
  }
  return 20 * Math.log10(Math.max(mag, 1e-6));
}

function drawFilter(canvas) {
  const ctx = canvas.getContext("2d");
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const cutoff = val(P.CUTOFF) || 20000;
  const reso = val(P.RESO);
  const type = Math.round(val(P.FILTER_TYPE));
  const enabled = val(P.FILTER_ENABLE) >= 0.5;
  const accent = css("--accent");
  const dbTop = 24;
  const dbBottom = -48;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.14)";
  ctx.lineWidth = 1;
  for (const f of [100, 1000, 10000]) {
    const x = ((Math.log(f) - FREQ_LO) / (FREQ_HI - FREQ_LO)) * w;
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
    const f = Math.exp(FREQ_LO + ((FREQ_HI - FREQ_LO) * i) / steps);
    const db = filterMagnitudeDb(f, cutoff, reso, type);
    const x = (i / steps) * w;
    const y = ((dbTop - clamp(db, dbBottom, dbTop)) / (dbTop - dbBottom)) * h;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = enabled ? accent : "rgba(126, 147, 163, 0.5)";
  ctx.lineWidth = Math.max(1.6, h / 62);
  if (enabled) {
    ctx.shadowColor = accent;
    ctx.shadowBlur = h / 26;
  }
  ctx.stroke();
  ctx.shadowBlur = 0;

  // Cutoff handle.
  const hx = ((Math.log(clamp(cutoff, 20, 20000)) - FREQ_LO) / (FREQ_HI - FREQ_LO)) * w;
  const hy = ((dbTop - clamp(filterMagnitudeDb(cutoff, cutoff, reso, type), dbBottom, dbTop)) /
    (dbTop - dbBottom)) * h;
  ctx.fillStyle = accent;
  ctx.beginPath();
  ctx.arc(hx, hy, Math.max(3.5, h / 34), 0, Math.PI * 2);
  ctx.fill();
}

{
  const canvas = $id("viz-filter");
  canvas.dataset.dest = "9"; // drop target: Cutoff
  const view = setupCanvas(canvas, () => drawFilter(canvas));
  redrawFns.push(view.redraw);

  const apply = (e) => {
    const rect = canvas.getBoundingClientRect();
    const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
    const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
    setParam(P.CUTOFF, Math.exp(FREQ_LO + tx * (FREQ_HI - FREQ_LO)));
    setParam(P.RESO, clamp(1 - ty * 1.35, 0, 1));
    showTip(e.clientX, e.clientY, `${fmt.hz(val(P.CUTOFF))} · Reso ${fmt.pct(val(P.RESO))}`);
  };
  canvas.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    canvas.setPointerCapture(e.pointerId);
    apply(e);
  });
  canvas.addEventListener("pointermove", (e) => {
    if (e.buttons & 1) apply(e);
  });
  canvas.addEventListener("pointerup", (e) => {
    canvas.releasePointerCapture(e.pointerId);
    hideTip();
  });
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    setParam(P.RESO, clamp(val(P.RESO) - Math.sign(e.deltaY) * 0.04, 0, 1));
    showTip(e.clientX, e.clientY, `Reso ${fmt.pct(val(P.RESO))}`);
  });
}

// --- Envelope views (draggable handles) ----------------------------------------------------------

function envShape(x, curve) {
  return Math.pow(clamp(x, 0, 1), Math.pow(2, 3 * curve));
}

const SUSTAIN_W = 0.16; // fixed visual plateau fraction

function envGeometry(base, w, h) {
  const a = val(base + P.ATTACK);
  const d = val(base + P.DECAY);
  const s = val(base + P.SUSTAIN);
  const r = val(base + P.RELEASE);
  const total = Math.max(a + d + r, 1e-3);
  const ax = (a / total) * (1 - SUSTAIN_W);
  const dx = (d / total) * (1 - SUSTAIN_W);
  const rx = (r / total) * (1 - SUSTAIN_W);
  const pad = h * 0.1;
  const y = (v) => h - pad - v * (h - 2 * pad);
  return { a, d, s, r, total, ax, dx, rx, y, pad };
}

function drawEnv(canvas, base, level) {
  const ctx = canvas.getContext("2d");
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const g = envGeometry(base, w, h);
  const curve = val(base + P.CURVE);
  const accent = css("--accent");

  ctx.beginPath();
  ctx.moveTo(0, g.y(0));
  const steps = 28;
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    ctx.lineTo(t * g.ax * w, g.y(envShape(t, curve)));
  }
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    ctx.lineTo((g.ax + t * g.dx) * w, g.y(g.s + (1 - g.s) * envShape(1 - t, curve)));
  }
  ctx.lineTo((g.ax + g.dx + SUSTAIN_W) * w, g.y(g.s));
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    ctx.lineTo((g.ax + g.dx + SUSTAIN_W + t * g.rx) * w, g.y(g.s * envShape(1 - t, curve)));
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = Math.max(1.6, h / 52);
  ctx.stroke();
  // Fill under the curve.
  ctx.lineTo((g.ax + g.dx + SUSTAIN_W + g.rx) * w, g.y(0));
  ctx.lineTo(0, g.y(0));
  ctx.closePath();
  ctx.fillStyle = css("--accent-soft");
  ctx.fill();

  // Handles: attack peak, decay→sustain, release end.
  const handles = envHandles(g, w);
  ctx.fillStyle = accent;
  for (const [hx, hy] of handles) {
    ctx.beginPath();
    ctx.arc(hx, hy, Math.max(3, h / 26), 0, Math.PI * 2);
    ctx.fill();
  }

  // Live level marker.
  if (level > 0.001) {
    ctx.globalAlpha = 0.9;
    ctx.beginPath();
    ctx.arc(w * 0.03, g.y(clamp(level, 0, 1)), Math.max(2.5, h / 30), 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
  }
}

function envHandles(g, w) {
  return [
    [g.ax * w, g.y(1)],
    [(g.ax + g.dx) * w, g.y(g.s)],
    [(g.ax + g.dx + SUSTAIN_W + g.rx) * w, g.y(0)],
  ];
}

function wireEnvCanvas(canvasId, base, levelKey) {
  const canvas = $id(canvasId);
  const view = setupCanvas(canvas, () => drawEnv(canvas, base, state[levelKey]));
  redrawFns.push(view.redraw);

  let handle = -1;
  let lastX = 0;
  let lastY = 0;

  canvas.addEventListener("pointerdown", (e) => {
    const rect = canvas.getBoundingClientRect();
    const dpr = canvas.width / rect.width;
    const px = (e.clientX - rect.left) * dpr;
    const py = (e.clientY - rect.top) * dpr;
    const g = envGeometry(base, canvas.width, canvas.height);
    const handles = envHandles(g, canvas.width);
    handle = -1;
    let best = 20 * dpr;
    handles.forEach(([hx, hy], i) => {
      const dist = Math.hypot(px - hx, py - hy);
      if (dist < best) {
        best = dist;
        handle = i;
      }
    });
    if (handle >= 0) {
      e.preventDefault();
      canvas.setPointerCapture(e.pointerId);
      lastX = e.clientX;
      lastY = e.clientY;
    }
  });
  canvas.addEventListener("pointermove", (e) => {
    if (handle < 0 || !(e.buttons & 1)) return;
    const rect = canvas.getBoundingClientRect();
    const dxn = (e.clientX - lastX) / rect.width;
    const dyn = (e.clientY - lastY) / rect.height;
    lastX = e.clientX;
    lastY = e.clientY;
    const g = envGeometry(base, canvas.width, canvas.height);
    const scale = Math.max(g.total, 0.4) / (1 - SUSTAIN_W);
    if (handle === 0) {
      setParam(base + P.ATTACK, clamp(g.a + dxn * scale, 0, 5));
      showTip(e.clientX, e.clientY, `Attack: ${fmt.s(val(base + P.ATTACK))}`);
    } else if (handle === 1) {
      setParam(base + P.DECAY, clamp(g.d + dxn * scale, 0, 5));
      setParam(base + P.SUSTAIN, clamp(g.s - dyn * 1.25, 0, 1));
      showTip(
        e.clientX,
        e.clientY,
        `Dec ${fmt.s(val(base + P.DECAY))} · Sus ${fmt.pct(val(base + P.SUSTAIN))}`,
      );
    } else {
      setParam(base + P.RELEASE, clamp(g.r + dxn * scale, 0, 5));
      showTip(e.clientX, e.clientY, `Release: ${fmt.s(val(base + P.RELEASE))}`);
    }
  });
  canvas.addEventListener("pointerup", (e) => {
    if (handle >= 0) canvas.releasePointerCapture(e.pointerId);
    handle = -1;
    hideTip();
  });
}

wireEnvCanvas("viz-env1", P.ENV1, "env1");
wireEnvCanvas("viz-env2", P.ENV2, "env2");

// --- LFO views --------------------------------------------------------------------------------------

function lfoShape(x, wave) {
  const t = x - Math.floor(x);
  if (wave === 0) return Math.sin(2 * Math.PI * t);
  if (wave === 1) return t < 0.5 ? 4 * t - 1 : 3 - 4 * t;
  if (wave === 2) return 2 * t - 1;
  if (wave === 3) return t < 0.5 ? 1 : -1;
  const stepIndex = Math.floor(x * 8);
  const r = Math.sin(stepIndex * 127.1) * 43758.5453;
  return (r - Math.floor(r)) * 2 - 1;
}

function drawLfo(canvas, base, liveValue) {
  const ctx = canvas.getContext("2d");
  const { width: w, height: h } = canvas;
  ctx.clearRect(0, 0, w, h);
  const wave = Math.round(val(base + P.WAVE));
  const phase = val(base + P.LFO_PHASE);
  const accent = css("--accent");
  const midY = h / 2;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.22)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  ctx.beginPath();
  const steps = 96;
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const v = lfoShape(t + phase, wave);
    const x = t * w;
    const y = midY - v * (h * 0.36);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = Math.max(1.5, h / 44);
  ctx.stroke();

  ctx.fillStyle = accent;
  ctx.beginPath();
  ctx.arc(w * 0.045, midY - clamp(liveValue, -1, 1) * (h * 0.36), Math.max(2.5, h / 24), 0, Math.PI * 2);
  ctx.fill();
}

for (const [canvasId, base, key] of [
  ["viz-lfo1", P.LFO1, "lfo1"],
  ["viz-lfo2", P.LFO2, "lfo2"],
]) {
  const canvas = $id(canvasId);
  const view = setupCanvas(canvas, () => drawLfo(canvas, base, state[key]));
  redrawFns.push(view.redraw);
}

// --- Preview keyboard ---------------------------------------------------------------------------------

const KEY_LO = 36; // C2
const KEY_HI = 84; // C6
const BLACK = new Set([1, 3, 6, 8, 10]);

function buildKeyboard() {
  const keyboardEl = $id("keyboard");
  const whites = [];
  for (let key = KEY_LO; key <= KEY_HI; key++) {
    if (!BLACK.has(key % 12)) whites.push(key);
  }
  const whiteW = 100 / whites.length;

  let whiteIndex = 0;
  for (let key = KEY_LO; key <= KEY_HI; key++) {
    const el = document.createElement("div");
    const black = BLACK.has(key % 12);
    el.className = `key ${black ? "black" : "white"}`;
    el.dataset.key = key;
    if (black) {
      el.style.left = `${whiteIndex * whiteW - whiteW * 0.32}%`;
      el.style.width = `${whiteW * 0.64}%`;
    } else {
      if (key % 12 === 0) {
        el.classList.add("c-key");
        el.dataset.label = `C${Math.floor(key / 12) - 1}`;
      }
      whiteIndex += 1;
    }
    keyboardEl.append(el);
  }

  let heldKey = -1;

  const keyFromPoint = (x, y) => {
    const el = document.elementFromPoint(x, y);
    const keyEl = el && el.closest ? el.closest(".key") : null;
    return keyEl ? Number(keyEl.dataset.key) : -1;
  };

  const press = (key, velocity) => {
    if (key === heldKey || key < 0) return;
    release();
    heldKey = key;
    sendNote(true, key, velocity);
    const el = keyboardEl.querySelector(`[data-key="${key}"]`);
    if (el) el.classList.add("held");
  };

  const release = () => {
    if (heldKey < 0) return;
    sendNote(false, heldKey, 0);
    const el = keyboardEl.querySelector(`[data-key="${heldKey}"]`);
    if (el) el.classList.remove("held");
    heldKey = -1;
  };

  const velocityFromEvent = (e) => {
    const keyEl = e.target.closest ? e.target.closest(".key") : null;
    if (!keyEl) return 100;
    const rect = keyEl.getBoundingClientRect();
    return Math.round(clamp(45 + (82 * (e.clientY - rect.top)) / rect.height, 30, 127));
  };

  keyboardEl.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    keyboardEl.setPointerCapture(e.pointerId);
    press(keyFromPoint(e.clientX, e.clientY), velocityFromEvent(e));
  });
  keyboardEl.addEventListener("pointermove", (e) => {
    if (e.buttons & 1) press(keyFromPoint(e.clientX, e.clientY), velocityFromEvent(e));
  });
  keyboardEl.addEventListener("pointerup", release);
  keyboardEl.addEventListener("pointercancel", release);
  window.addEventListener("blur", release);
}

buildKeyboard();

// --- Binary packets from the plugin -----------------------------------------------------------------------

function handleBinary(data) {
  let ab = data;
  if (NATIVE && data && data.type === "bin" && typeof data.data === "string") {
    ab = base64ToBytes(data.data).buffer;
  }
  if (!(ab instanceof ArrayBuffer) || ab.byteLength < 5) return;
  const view = new DataView(ab);
  const magic = String.fromCharCode(
    view.getUint8(0),
    view.getUint8(1),
    view.getUint8(2),
    view.getUint8(3),
  );
  if (magic === "ZWTW") {
    const osc = view.getUint8(4) === 1 ? 1 : 0;
    const n = view.getUint16(5, true);
    if (ab.byteLength < 7 + n * 4) return;
    const samples = new Float32Array(n);
    for (let i = 0; i < n; i++) samples[i] = view.getFloat32(7 + i * 4, true);
    state.wave[osc] = samples;
    invalidate();
  } else if (magic === "ZWTS") {
    const osc = view.getUint8(4) === 1 ? 1 : 0;
    const frameCount = view.getUint8(5);
    const n = view.getUint16(6, true);
    if (ab.byteLength < 8 + frameCount * n * 4) return;
    const frames = [];
    for (let f = 0; f < frameCount; f++) {
      const frame = new Float32Array(n);
      for (let i = 0; i < n; i++) frame[i] = view.getFloat32(8 + (f * n + i) * 4, true);
      frames.push(frame);
    }
    state.stack[osc] = frames;
    invalidate();
  } else if (magic === "ZWTM") {
    state.voices = view.getUint8(4);
    state.env1 = view.getFloat32(5, true);
    state.env2 = view.getFloat32(9, true);
    state.lfo1 = view.getFloat32(13, true);
    state.lfo2 = view.getFloat32(17, true);
    const note = $id("voice-note");
    if (note) note.textContent = `${state.voices} voice${state.voices === 1 ? "" : "s"}`;
    invalidate();
  }
}

// --- Connect ------------------------------------------------------------------------------------------------

sendSet = connect({
  onSnapshot(map) {
    markConnected();
    applySnapshot(map);
  },
  onMessage: handleBinary,
});

// Tiny read-only hook for automated UI tests.
window.__waveSynth = { val };

invalidate();
