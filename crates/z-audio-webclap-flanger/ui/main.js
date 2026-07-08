// Z Audio Flanger UI — comb-filter magnitude response preview.
//
// The canvas plots |H(f)| of the dry+wet mix with the LFO at its center
// (delay τ = manual). Drag horizontally to set the manual delay (log),
// vertically the feedback (bipolar, center = 0).

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  manual: 840,
  rate: 841,
  depth: 842,
  feedback: 843,
  spread: 844,
  mix: 845,
  output: 846,
};

const PARAMS = [
  { id: P.manual, label: "Manual", kind: "slider", min: 0.5, max: 10, default: 2, scale: "log", fmt: fmt.ms, mount: "#sec-sweep" },
  { id: P.rate, label: "Rate", kind: "slider", min: 0.02, max: 5, default: 0.3, scale: "log", fmt: fmt.hzLfo, mount: "#sec-sweep" },
  { id: P.depth, label: "Depth", kind: "slider", min: 0, max: 1, default: 0.7, step: 0.01, fmt: fmt.pct, mount: "#sec-sweep" },
  { id: P.spread, label: "Spread", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-sweep" },
  { id: P.feedback, label: "Feedback", kind: "slider", min: -0.95, max: 0.95, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-output" },
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-output" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-output" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Mirrors src/lib.rs process(): the engine writes input + tap*feedback into
// the delay line (tap delayed by τ) and outputs dry*(1-mix) + tap*mix, so at
// the LFO center the transfer function is
//   H(f) = (1-mix) + mix * e^{-jωτ} / (1 - fb * e^{-jωτ}),  τ = manual.
function responseDb(f, manualMs, fb, mix) {
  const wt = 2 * Math.PI * f * manualMs * 0.001; // ωτ
  const er = Math.cos(wt);
  const ei = -Math.sin(wt);
  // denom = 1 - fb * e^{-jωτ}
  const dr = 1 - fb * er;
  const di = -fb * ei;
  const dm = dr * dr + di * di;
  // wet = e^{-jωτ} / denom
  const wr = (er * dr + ei * di) / dm;
  const wi = (ei * dr - er * di) / dm;
  const hr = 1 - mix + mix * wr;
  const hi = mix * wi;
  return 20 * Math.log10(Math.max(Math.hypot(hr, hi), 1e-6));
}

const FREQ_MIN = 20;
const FREQ_MAX = 20000;
const LOG_LO = Math.log(FREQ_MIN);
const LOG_HI = Math.log(FREQ_MAX);
const DB_RANGE = 30; // vertical axis ±30 dB

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const manual = params.get(P.manual);
  const fb = params.get(P.feedback);
  const mix = params.get(P.mix);
  const yOfDb = (db) => h / 2 - (clamp(db, -DB_RANGE, DB_RANGE) / DB_RANGE) * (h / 2);

  // Frequency grid.
  ctx.font = `${9 * dpr}px sans-serif`;
  for (const f of [50, 100, 200, 500, 1000, 2000, 5000, 10000]) {
    const x = ((Math.log(f) - LOG_LO) / (LOG_HI - LOG_LO)) * w;
    ctx.strokeStyle = "rgba(126, 147, 163, 0.10)";
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
    ctx.fillStyle = "rgba(126, 147, 163, 0.5)";
    ctx.fillText(f >= 1000 ? `${f / 1000}k` : `${f}`, x + 3 * dpr, h - 5 * dpr);
  }
  // 0 dB line.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.3)";
  ctx.beginPath();
  ctx.moveTo(0, h / 2);
  ctx.lineTo(w, h / 2);
  ctx.stroke();

  // Comb curve.
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const f = Math.exp(LOG_LO + (px / w) * (LOG_HI - LOG_LO));
    const y = yOfDb(responseDb(f, manual, fb, mix));
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2 * dpr;
  ctx.shadowColor = accent;
  ctx.shadowBlur = 6 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;

  ctx.fillStyle = "rgba(126, 147, 163, 0.7)";
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.textAlign = "right";
  ctx.fillText(
    `manual ${fmt.ms(manual)} · fb ${fmt.pct(fb)} · mix ${fmt.pct(mix)}`,
    w - 6 * dpr,
    12 * dpr,
  );
  ctx.textAlign = "left";
});

const MANUAL_LO = Math.log(0.5);
const MANUAL_HI = Math.log(10);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const manual = Math.exp(MANUAL_LO + tx * (MANUAL_HI - MANUAL_LO));
  const feedback = (0.5 - ty) * 2 * 0.95; // top = +0.95, center = 0, bottom = -0.95
  params.set(P.manual, manual);
  params.set(P.feedback, feedback);
  sendSet(P.manual, manual);
  sendSet(P.feedback, feedback);
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
