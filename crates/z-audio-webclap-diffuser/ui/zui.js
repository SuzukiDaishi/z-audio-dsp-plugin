// Z Audio WebCLAP UI kit — shared transport + controls + canvas helpers.
//
// Every Z Audio plugin UI talks the same protocol as the wclap-plugin
// scaffold: it posts CBOR text "ready" once on boot, receives a
// `{params:{<id>:<f64>}}` snapshot back (and again whenever the plugin
// pushes one), and posts `{set:[<u32 id>, <f64 value>]}` per edit.
//
// This file is copied verbatim into each plugin bundle (bundles are
// self-contained; there is no cross-bundle import path).

"use strict";

// ---------------------------------------------------------------------------
// Transport.
// ---------------------------------------------------------------------------

function encodeReady() {
  return new Uint8Array([0x65, 0x72, 0x65, 0x61, 0x64, 0x79]).buffer;
}

function encodeSet(id, value) {
  const buf = new ArrayBuffer(20);
  const view = new DataView(buf);
  view.setUint8(0, 0xa1); // map(1)
  view.setUint8(1, 0x63); // text(3)
  view.setUint8(2, 0x73); // "set"
  view.setUint8(3, 0x65);
  view.setUint8(4, 0x74);
  view.setUint8(5, 0x82); // array(2)
  view.setUint8(6, 0x1a); // u32
  view.setUint32(7, id, false);
  view.setUint8(11, 0xfb); // f64
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

/**
 * Opens the plugin connection. `onSnapshot(map)` fires for every params
 * snapshot; `onMessage(arrayBuffer)` (optional) sees any other binary
 * message. Returns `sendSet(id, value)`.
 */
export function connect({ onSnapshot, onMessage } = {}) {
  window.addEventListener("message", (event) => {
    if (!(event.data instanceof ArrayBuffer)) return;
    const snapshot = decodeParamsSnapshot(event.data);
    if (snapshot) {
      if (onSnapshot) onSnapshot(snapshot);
      return;
    }
    if (onMessage) onMessage(event.data);
  });
  window.parent.postMessage(encodeReady(), "*");
  return (id, value) => window.parent.postMessage(encodeSet(id, value), "*");
}

// ---------------------------------------------------------------------------
// Formatting.
// ---------------------------------------------------------------------------

export const fmt = {
  db: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(1)} dB`,
  hz: (v) => (v >= 1000 ? `${(v / 1000).toFixed(2)} kHz` : `${v.toFixed(0)} Hz`),
  hzLfo: (v) => `${v.toFixed(2)} Hz`,
  ms: (v) => (v >= 1000 ? `${(v / 1000).toFixed(2)} s` : `${v.toFixed(1)} ms`),
  s: (v) => (v < 1 ? `${(v * 1000).toFixed(0)} ms` : `${v.toFixed(2)} s`),
  pct: (v) => `${(v * 100).toFixed(0)} %`,
  ratio: (v) => `${v.toFixed(1)}:1`,
  x: (v) => `×${v.toFixed(2)}`,
  int: (v) => `${Math.round(v)}`,
  plain: (v) => v.toFixed(2),
};

export function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

// ---------------------------------------------------------------------------
// Param controls.
//
// Def shape: { id, label, kind: "slider"|"select"|"toggle",
//              min, max, default, scale?: "log", fmt?, options?: [..] }
// ---------------------------------------------------------------------------

const NORM_STEPS = 1000;

function toNorm(def, v) {
  if (def.scale === "log") {
    const lo = Math.log(def.min);
    const hi = Math.log(def.max);
    return Math.round(((Math.log(clamp(v, def.min, def.max)) - lo) / (hi - lo)) * NORM_STEPS);
  }
  return Math.round(((v - def.min) / (def.max - def.min)) * NORM_STEPS);
}

function fromNorm(def, n) {
  const t = n / NORM_STEPS;
  let v;
  if (def.scale === "log") {
    v = def.min * Math.pow(def.max / def.min, t);
  } else {
    v = def.min + t * (def.max - def.min);
  }
  if (def.step) v = Math.round(v / def.step) * def.step;
  return clamp(v, def.min, def.max);
}

function sliderControl(def, emit) {
  const row = document.createElement("label");
  row.className = "control";
  const name = document.createElement("span");
  name.className = "control-label";
  name.textContent = def.label;
  const input = document.createElement("input");
  input.type = "range";
  input.min = 0;
  input.max = NORM_STEPS;
  input.step = 1;
  input.value = toNorm(def, def.default);
  const readout = document.createElement("span");
  readout.className = "readout";
  const format = def.fmt || fmt.plain;
  readout.textContent = format(def.default);
  input.addEventListener("input", () => {
    const v = fromNorm(def, Number(input.value));
    readout.textContent = format(v);
    emit(v);
  });
  input.addEventListener("dblclick", () => {
    input.value = toNorm(def, def.default);
    readout.textContent = format(def.default);
    emit(def.default);
  });
  row.append(name, input, readout);
  return {
    root: row,
    get: () => fromNorm(def, Number(input.value)),
    set: (v) => {
      input.value = toNorm(def, v);
      readout.textContent = format(v);
    },
  };
}

function segmentedControl(def, emit) {
  const row = document.createElement("div");
  row.className = "control control-seg";
  const name = document.createElement("span");
  name.className = "control-label";
  name.textContent = def.label;
  const seg = document.createElement("div");
  seg.className = "segmented";
  const buttons = def.options.map((opt, i) => {
    const value = typeof opt === "object" ? opt.value : i;
    const label = typeof opt === "object" ? opt.label : opt;
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    b.dataset.value = value;
    b.addEventListener("click", () => {
      mark(value);
      emit(value);
    });
    seg.append(b);
    return b;
  });
  function mark(value) {
    for (const b of buttons) {
      b.classList.toggle("active", Number(b.dataset.value) === Math.round(value));
    }
  }
  mark(def.default);
  row.append(name, seg);
  return {
    root: row,
    get: () => {
      const active = buttons.find((b) => b.classList.contains("active"));
      return active ? Number(active.dataset.value) : def.default;
    },
    set: mark,
  };
}

function toggleControl(def, emit) {
  const row = document.createElement("label");
  row.className = "control control-toggle";
  const name = document.createElement("span");
  name.className = "control-label";
  name.textContent = def.label;
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = def.default >= 0.5;
  const pill = document.createElement("span");
  pill.className = "toggle-pill";
  input.addEventListener("change", () => emit(input.checked ? 1 : 0));
  row.append(name, input, pill);
  return {
    root: row,
    get: () => (input.checked ? 1 : 0),
    set: (v) => {
      input.checked = v >= 0.5;
    },
  };
}

/**
 * Builds all controls for `defs`, appending each control to the element
 * matching its `def.mount` selector (or `fallbackMount`). Edits are sent
 * to the plugin and forwarded to `onChange(id, value)`. Returns a store
 * with `get(id)`, `values()` and `applySnapshot(map)`.
 */
export function createParams(defs, sendSet, onChange, fallbackMount) {
  const controls = new Map();
  for (const def of defs) {
    const emit = (v) => {
      sendSet(def.id, v);
      if (onChange) onChange(def.id, v);
    };
    const control =
      def.kind === "select"
        ? segmentedControl(def, emit)
        : def.kind === "toggle"
          ? toggleControl(def, emit)
          : sliderControl(def, emit);
    controls.set(def.id, { def, control });
    const mount = document.querySelector(def.mount || fallbackMount);
    if (mount) mount.append(control.root);
  }
  return {
    get(id) {
      const c = controls.get(id);
      return c ? c.control.get() : 0;
    },
    values() {
      const out = new Map();
      for (const [id, { control }] of controls) out.set(id, control.get());
      return out;
    },
    set(id, v) {
      const c = controls.get(id);
      if (c) c.control.set(clamp(v, c.def.min ?? v, c.def.max ?? v));
    },
    applySnapshot(map) {
      for (const [id, value] of map) {
        const c = controls.get(id);
        if (!c) continue;
        const lo = c.def.min ?? value;
        const hi = c.def.max ?? value;
        c.control.set(clamp(value, lo, hi));
      }
      if (onChange) onChange(null, null);
    },
  };
}

// ---------------------------------------------------------------------------
// Canvas helper — DPR-aware sizing plus a redraw hook on resize.
// ---------------------------------------------------------------------------

export function setupCanvas(canvas, draw) {
  const ctx = canvas.getContext("2d");
  function resize() {
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.round(rect.width * dpr));
    canvas.height = Math.max(1, Math.round(rect.height * dpr));
    draw();
  }
  window.addEventListener("resize", resize);
  // First layout pass may not be done yet.
  requestAnimationFrame(resize);
  return { ctx, redraw: () => draw(), resize };
}

/** Marks the status pill as live. */
export function markConnected() {
  const el = document.getElementById("status");
  if (el) el.textContent = "CONNECTED";
}
