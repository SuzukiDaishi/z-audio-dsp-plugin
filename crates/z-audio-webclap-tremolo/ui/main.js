// Z Audio Tremolo UI — LFO gain preview.
//
// The canvas plots the per-channel gain curves over one LFO cycle:
// L in the accent color, R dimmer and dashed (offset by the stereo
// phase). Drag horizontally to sweep the rate (log), vertically to set
// the depth.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  rate: 880,
  depth: 881,
  wave: 882,
  phase: 883,
  output: 884,
};

const PARAMS = [
  { id: P.rate, label: "Rate", kind: "slider", min: 0.1, max: 20, default: 4, scale: "log", fmt: fmt.hzLfo, mount: "#sec-lfo" },
  { id: P.depth, label: "Depth", kind: "slider", min: 0, max: 1, default: 0.6, step: 0.01, fmt: fmt.pct, mount: "#sec-lfo" },
  { id: P.wave, label: "Wave", kind: "select", options: ["Sine", "Tri", "Square"], default: 0, mount: "#sec-lfo" },
  { id: P.phase, label: "Stereo Phase", kind: "slider", min: 0, max: 180, default: 0, step: 1, fmt: (v) => `${v.toFixed(0)}°`, mount: "#sec-lfo" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-output" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Mirrors src/lib.rs lfo(): value in [-1, 1] for a phase in cycles; the
// square wave is sign(sin) and gets smoothed separately below.
function lfo(wave, phase) {
  const t = phase - Math.floor(phase);
  if (wave === 1) return t < 0.5 ? 4 * t - 1 : 3 - 4 * t;
  if (wave === 2) return t < 0.5 ? 1 : -1;
  return Math.sin(2 * Math.PI * t);
}

// Mirrors src/lib.rs process(): gain = 1 - depth * (0.5 + 0.5 * lfo),
// with the square LFO run through the same ~2 ms one-pole. One cycle is
// sampled at n points; a warm-up pass settles the smoother so the drawn
// curve is periodic.
function gainCurve(wave, depth, rate, offset, n) {
  const out = new Float32Array(n + 1);
  const dt = 1 / rate / n; // seconds per point across one cycle
  const a = 1 - Math.exp(-dt / 0.002);
  let sm = lfo(wave, offset);
  for (let pass = 0; pass < 2; pass++) {
    for (let i = 0; i <= n; i++) {
      const raw = lfo(wave, i / n + offset);
      const v = wave === 2 ? (sm += a * (raw - sm)) : raw;
      if (pass === 1) out[i] = 1 - depth * (0.5 + 0.5 * v);
    }
  }
  return out;
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const rate = params.get(P.rate);
  const depth = params.get(P.depth);
  const wave = Math.round(params.get(P.wave));
  const phase = params.get(P.phase);

  const pad = 8 * dpr;
  const yFor = (gain) => pad + (1 - gain) * (h - 2 * pad); // gain 0..1 → bottom..top

  // Reference lines at gain 1 and gain 0.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  for (const g of [0, 1]) {
    ctx.beginPath();
    ctx.moveTo(0, yFor(g));
    ctx.lineTo(w, yFor(g));
    ctx.stroke();
  }

  const trace = (curve, style, width, dash) => {
    ctx.beginPath();
    for (let px = 0; px <= w; px++) {
      const i = Math.round((px / w) * (curve.length - 1));
      const y = yFor(curve[i]);
      if (px === 0) ctx.moveTo(px, y);
      else ctx.lineTo(px, y);
    }
    ctx.strokeStyle = style;
    ctx.lineWidth = width;
    ctx.setLineDash(dash);
    ctx.stroke();
    ctx.setLineDash([]);
  };

  const n = 512;
  // Right channel first (dimmer, dashed) so the L curve draws on top.
  trace(gainCurve(wave, depth, rate, phase / 360, n), "rgba(126, 147, 163, 0.6)", 1.4 * dpr, [4 * dpr, 4 * dpr]);

  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.shadowColor = accent;
  ctx.shadowBlur = 6 * dpr;
  trace(gainCurve(wave, depth, rate, 0, n), accent, 2 * dpr, []);
  ctx.shadowBlur = 0;

  ctx.fillStyle = "rgba(126, 147, 163, 0.7)";
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.textAlign = "right";
  ctx.fillText(`rate ${fmt.hzLfo(rate)} · depth ${fmt.pct(depth)}`, w - 6 * dpr, 12 * dpr);
  ctx.textAlign = "left";
});

const RATE_LO = Math.log(0.1);
const RATE_HI = Math.log(20);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const rate = Math.exp(RATE_LO + tx * (RATE_HI - RATE_LO));
  const depth = 1 - ty;
  params.set(P.rate, rate);
  params.set(P.depth, depth);
  sendSet(P.rate, rate);
  sendSet(P.depth, depth);
  viz.redraw();
}

canvas.addEventListener("pointerdown", (e) => {
  dragging = true;
  canvas.setPointerCapture(e.pointerId);
  applyDrag(e);
});
canvas.addEventListener("pointermove", (e) => {
  if (dragging) applyDrag(e);
});
canvas.addEventListener("pointerup", () => {
  dragging = false;
});
