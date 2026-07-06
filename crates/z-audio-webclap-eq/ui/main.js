// Z Audio Simple EQ UI — interactive frequency-response editor.
//
// Three bands on a log-frequency / dB canvas. Each band is a draggable
// node: horizontal = frequency, vertical = gain (shelf/bell), mouse wheel
// = Q, double-click = enable/disable. Editing any band control while the
// band is off switches it on (same behaviour the old UI documented).
// The plotted curves are exact RBJ biquad magnitude responses at 48 kHz,
// matching the DSP.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const SAMPLE_RATE = 48000;
const FREQ_MIN = 20;
const FREQ_MAX = 20000;
const DB_MIN = -24;
const DB_MAX = 24;
const TYPES = ["Low Shelf", "Bell", "High Shelf", "High Pass", "Low Pass"];
const TYPE_SHORT = ["LS", "Bell", "HS", "HP", "LP"];

const BANDS = [
  { key: "low", title: "Low", color: "#4fd1a5", enabled: 40, freq: 41, type: 42, gain: 49, q: 50, freqMin: 20, freqMax: 2000, freqDefault: 200, typeDefault: 0 },
  { key: "mid", title: "Mid", color: "#8b7cf6", enabled: 43, freq: 44, type: 45, gain: 51, q: 52, freqMin: 80, freqMax: 8000, freqDefault: 1000, typeDefault: 1 },
  { key: "high", title: "High", color: "#f5b64c", enabled: 46, freq: 47, type: 48, gain: 53, q: 54, freqMin: 1000, freqMax: 20000, freqDefault: 5000, typeDefault: 2 },
];

// Build band sections + param defs.
const defs = [];
const bandsRoot = document.getElementById("bands");
for (const band of BANDS) {
  const section = document.createElement("section");
  section.className = "section band";
  section.id = `band-${band.key}`;
  section.style.setProperty("--band", band.color);
  const head = document.createElement("div");
  head.className = "band-head";
  const title = document.createElement("h2");
  title.className = "section-title";
  title.textContent = band.title;
  head.append(title);
  section.append(head);
  bandsRoot.append(section);

  const mount = `#band-${band.key}`;
  defs.push(
    { id: band.enabled, label: "In", kind: "toggle", default: 0, mount, band },
    { id: band.type, label: "Type", kind: "select", options: TYPE_SHORT, default: band.typeDefault, mount, band },
    { id: band.freq, label: "Freq", kind: "slider", min: band.freqMin, max: band.freqMax, default: band.freqDefault, scale: "log", fmt: fmt.hz, mount, band },
    { id: band.gain, label: "Gain", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount, band },
    { id: band.q, label: "Q", kind: "slider", min: 0.1, max: 10, default: 0.707, scale: "log", fmt: fmt.plain, mount, band },
  );
}

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

// Any edit to a disabled band's non-enable controls switches the band on.
function onEdit(id) {
  if (id == null) return;
  const def = defs.find((d) => d.id === id);
  if (!def || id === def.band.enabled) return;
  if (params.get(def.band.enabled) < 0.5) {
    params.set(def.band.enabled, 1);
    sendSet(def.band.enabled, 1);
  }
}

const params = createParams(defs, sendSet, (id) => {
  onEdit(id);
  viz.redraw();
}, "#bands");

// ---------------------------------------------------------------------------
// RBJ biquad magnitude response (Audio EQ Cookbook), fs = 48 kHz.
// ---------------------------------------------------------------------------

function coeffs(type, f0, q, gainDb) {
  const A = Math.pow(10, gainDb / 40);
  const w0 = (2 * Math.PI * clamp(f0, 10, SAMPLE_RATE / 2 - 1)) / SAMPLE_RATE;
  const cw = Math.cos(w0);
  const sw = Math.sin(w0);
  const alpha = sw / (2 * Math.max(q, 0.01));
  let b0, b1, b2, a0, a1, a2;
  switch (type) {
    case 0: {
      // low shelf
      const s = 2 * Math.sqrt(A) * alpha;
      b0 = A * (A + 1 - (A - 1) * cw + s);
      b1 = 2 * A * (A - 1 - (A + 1) * cw);
      b2 = A * (A + 1 - (A - 1) * cw - s);
      a0 = A + 1 + (A - 1) * cw + s;
      a1 = -2 * (A - 1 + (A + 1) * cw);
      a2 = A + 1 + (A - 1) * cw - s;
      break;
    }
    case 2: {
      // high shelf
      const s = 2 * Math.sqrt(A) * alpha;
      b0 = A * (A + 1 + (A - 1) * cw + s);
      b1 = -2 * A * (A - 1 + (A + 1) * cw);
      b2 = A * (A + 1 + (A - 1) * cw - s);
      a0 = A + 1 - (A - 1) * cw + s;
      a1 = 2 * (A - 1 - (A + 1) * cw);
      a2 = A + 1 - (A - 1) * cw - s;
      break;
    }
    case 3: {
      // high pass
      b0 = (1 + cw) / 2;
      b1 = -(1 + cw);
      b2 = (1 + cw) / 2;
      a0 = 1 + alpha;
      a1 = -2 * cw;
      a2 = 1 - alpha;
      break;
    }
    case 4: {
      // low pass
      b0 = (1 - cw) / 2;
      b1 = 1 - cw;
      b2 = (1 - cw) / 2;
      a0 = 1 + alpha;
      a1 = -2 * cw;
      a2 = 1 - alpha;
      break;
    }
    default: {
      // bell
      b0 = 1 + alpha * A;
      b1 = -2 * cw;
      b2 = 1 - alpha * A;
      a0 = 1 + alpha / A;
      a1 = -2 * cw;
      a2 = 1 - alpha / A;
    }
  }
  return { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 };
}

function magDb(c, f) {
  const w = (2 * Math.PI * f) / SAMPLE_RATE;
  const cw = Math.cos(w);
  const sw = Math.sin(w);
  const c2 = Math.cos(2 * w);
  const s2 = Math.sin(2 * w);
  const nr = c.b0 + c.b1 * cw + c.b2 * c2;
  const ni = -(c.b1 * sw + c.b2 * s2);
  const dr = 1 + c.a1 * cw + c.a2 * c2;
  const di = -(c.a1 * sw + c.a2 * s2);
  const mag = Math.sqrt((nr * nr + ni * ni) / Math.max(dr * dr + di * di, 1e-12));
  return 20 * Math.log10(Math.max(mag, 1e-6));
}

function bandResponse(band, f) {
  return magDb(
    coeffs(Math.round(params.get(band.type)), params.get(band.freq), params.get(band.q), params.get(band.gain)),
    f,
  );
}

// ---------------------------------------------------------------------------
// Canvas.
// ---------------------------------------------------------------------------

const canvas = document.getElementById("viz");

function xOfFreq(f, w) {
  return ((Math.log10(f) - Math.log10(FREQ_MIN)) / (Math.log10(FREQ_MAX) - Math.log10(FREQ_MIN))) * w;
}

function freqOfX(x, w) {
  return Math.pow(10, Math.log10(FREQ_MIN) + (x / w) * (Math.log10(FREQ_MAX) - Math.log10(FREQ_MIN)));
}

function yOfDb(db, h) {
  return h - ((db - DB_MIN) / (DB_MAX - DB_MIN)) * h;
}

function dbOfY(y, h) {
  return DB_MIN + ((h - y) / h) * (DB_MAX - DB_MIN);
}

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);
  ctx.font = `${9 * dpr}px sans-serif`;

  // Grid: decades + dB lines.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.10)";
  ctx.fillStyle = "rgba(126, 147, 163, 0.55)";
  for (const f of [30, 50, 100, 200, 500, 1000, 2000, 5000, 10000]) {
    const x = xOfFreq(f, w);
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
    ctx.fillText(f >= 1000 ? `${f / 1000}k` : `${f}`, x + 2 * dpr, h - 4 * dpr);
  }
  for (let db = -18; db <= 18; db += 6) {
    const y = yOfDb(db, h);
    ctx.strokeStyle = db === 0 ? "rgba(126, 147, 163, 0.3)" : "rgba(126, 147, 163, 0.10)";
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
    ctx.fillText(`${db > 0 ? "+" : ""}${db}`, 4 * dpr, y - 2 * dpr);
  }

  // Per-band faint curves + summed bold curve.
  const active = BANDS.filter((b) => params.get(b.enabled) >= 0.5);
  for (const band of BANDS) {
    if (params.get(band.enabled) < 0.5) continue;
    ctx.beginPath();
    for (let px = 0; px <= w; px += 2) {
      const f = freqOfX(px, w);
      const y = yOfDb(clamp(bandResponse(band, f), DB_MIN, DB_MAX), h);
      if (px === 0) ctx.moveTo(px, y);
      else ctx.lineTo(px, y);
    }
    ctx.strokeStyle = band.color;
    ctx.globalAlpha = 0.35;
    ctx.stroke();
    ctx.globalAlpha = 1;
  }

  // Sum.
  ctx.beginPath();
  for (let px = 0; px <= w; px += 1) {
    const f = freqOfX(px, w);
    let db = 0;
    for (const band of active) db += bandResponse(band, f);
    const y = yOfDb(clamp(db, DB_MIN, DB_MAX), h);
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = "#eef2f0";
  ctx.lineWidth = 2 * dpr;
  ctx.shadowColor = "rgba(238, 242, 240, 0.4)";
  ctx.shadowBlur = 4 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;
  ctx.lineWidth = 1;

  // Fill under sum.
  ctx.beginPath();
  ctx.moveTo(0, yOfDb(0, h));
  for (let px = 0; px <= w; px += 2) {
    const f = freqOfX(px, w);
    let db = 0;
    for (const band of active) db += bandResponse(band, f);
    ctx.lineTo(px, yOfDb(clamp(db, DB_MIN, DB_MAX), h));
  }
  ctx.lineTo(w, yOfDb(0, h));
  ctx.closePath();
  ctx.fillStyle = "rgba(79, 209, 165, 0.08)";
  ctx.fill();

  // Band handles (also for disabled bands, dimmed).
  for (const band of BANDS) {
    const enabled = params.get(band.enabled) >= 0.5;
    const type = Math.round(params.get(band.type));
    const gain = type === 3 || type === 4 ? 0 : params.get(band.gain);
    const x = xOfFreq(params.get(band.freq), w);
    const y = yOfDb(clamp(gain, DB_MIN, DB_MAX), h);
    ctx.beginPath();
    ctx.arc(x, y, 6 * dpr, 0, Math.PI * 2);
    ctx.fillStyle = enabled ? band.color : "rgba(126, 147, 163, 0.35)";
    ctx.fill();
    ctx.strokeStyle = "#0a0f15";
    ctx.stroke();
    ctx.fillStyle = enabled ? "#0a0f15" : "#38444f";
    ctx.font = `bold ${8 * dpr}px sans-serif`;
    ctx.textAlign = "center";
    ctx.fillText(band.title[0], x, y + 3 * dpr);
    ctx.textAlign = "left";
    ctx.font = `${9 * dpr}px sans-serif`;
  }
});

// ---------------------------------------------------------------------------
// Handle interactions.
// ---------------------------------------------------------------------------

let drag = null; // { band }

function handleAt(e) {
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const x = (e.clientX - rect.left) * dpr;
  const y = (e.clientY - rect.top) * dpr;
  let best = null;
  let bestDist = 14 * dpr;
  for (const band of BANDS) {
    const type = Math.round(params.get(band.type));
    const gain = type === 3 || type === 4 ? 0 : params.get(band.gain);
    const hx = xOfFreq(params.get(band.freq), canvas.width);
    const hy = yOfDb(clamp(gain, DB_MIN, DB_MAX), canvas.height);
    const d = Math.hypot(hx - x, hy - y);
    if (d < bestDist) {
      bestDist = d;
      best = band;
    }
  }
  return best;
}

canvas.addEventListener("pointerdown", (e) => {
  const band = handleAt(e);
  if (!band) return;
  drag = { band };
  canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  if (!drag) {
    canvas.style.cursor = handleAt(e) ? "grab" : "default";
    return;
  }
  const band = drag.band;
  const x = (e.clientX - rect.left) * dpr;
  const y = (e.clientY - rect.top) * dpr;
  const freq = clamp(freqOfX(x, canvas.width), band.freqMin, band.freqMax);
  params.set(band.freq, freq);
  sendSet(band.freq, freq);
  const type = Math.round(params.get(band.type));
  if (type !== 3 && type !== 4) {
    const gain = clamp(dbOfY(y, canvas.height), -24, 24);
    params.set(band.gain, gain);
    sendSet(band.gain, gain);
  }
  onEdit(band.freq);
  viz.redraw();
});

canvas.addEventListener("pointerup", () => {
  drag = null;
});

canvas.addEventListener("dblclick", (e) => {
  const band = handleAt(e);
  if (!band) return;
  const next = params.get(band.enabled) >= 0.5 ? 0 : 1;
  params.set(band.enabled, next);
  sendSet(band.enabled, next);
  viz.redraw();
});

canvas.addEventListener(
  "wheel",
  (e) => {
    const band = handleAt(e) || (drag && drag.band);
    if (!band) return;
    e.preventDefault();
    const q = clamp(params.get(band.q) * (e.deltaY < 0 ? 1.12 : 1 / 1.12), 0.1, 10);
    params.set(band.q, q);
    sendSet(band.q, q);
    onEdit(band.q);
    viz.redraw();
  },
  { passive: false },
);
