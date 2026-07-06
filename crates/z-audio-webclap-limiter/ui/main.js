// Z Audio Limiter UI — brickwall transfer curve + gain staging.
//
// Drag on the curve: horizontal moves the threshold (where limiting
// starts), vertical moves the ceiling (the flat top the output may never
// exceed). Lookahead/release/link/true-peak live in Behaviour.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  inputGain: 120,
  threshold: 121,
  ceiling: 122,
  release: 123,
  lookahead: 124,
  stereoLink: 125,
  truePeak: 126,
  outputGain: 127,
};

const PARAMS = [
  { id: P.inputGain, label: "Input", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-gain" },
  { id: P.threshold, label: "Threshold", kind: "slider", min: -24, max: 0, default: -0.1, step: 0.1, fmt: fmt.db, mount: "#sec-gain" },
  { id: P.ceiling, label: "Ceiling", kind: "slider", min: -24, max: 0, default: -0.1, step: 0.1, fmt: fmt.db, mount: "#sec-gain" },
  { id: P.outputGain, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-gain" },
  { id: P.lookahead, label: "Lookahead", kind: "slider", min: 0, max: 10, default: 3, step: 0.01, fmt: fmt.ms, mount: "#sec-time" },
  { id: P.release, label: "Release", kind: "slider", min: 1, max: 1000, default: 80, scale: "log", fmt: fmt.ms, mount: "#sec-time" },
  { id: P.stereoLink, label: "Link", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-time" },
  { id: P.truePeak, label: "True Peak", kind: "toggle", default: 0, mount: "#sec-time" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// ---------------------------------------------------------------------------
// Transfer curve: below threshold the signal passes (plus input gain and
// the threshold→ceiling makeup); above it the output is pinned at the
// ceiling. Output gain shifts the whole curve.
// ---------------------------------------------------------------------------

const DB_LO = -36;
const DB_HI = 12;

function transfer(x, input, threshold, ceiling, output) {
  const driven = x + input;
  const limited = Math.min(driven, threshold);
  return limited - threshold + ceiling + output;
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const input = params.get(P.inputGain);
  const threshold = params.get(P.threshold);
  const ceiling = params.get(P.ceiling);
  const output = params.get(P.outputGain);

  const xOf = (db) => ((db - DB_LO) / (DB_HI - DB_LO)) * w;
  const yOf = (db) => h - ((db - DB_LO) / (DB_HI - DB_LO)) * h;

  // Grid every 6 dB.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.12)";
  ctx.fillStyle = "rgba(126, 147, 163, 0.55)";
  ctx.font = `${9 * dpr}px sans-serif`;
  for (let db = -36; db <= 12; db += 6) {
    ctx.beginPath();
    ctx.moveTo(xOf(db), 0);
    ctx.lineTo(xOf(db), h);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(0, yOf(db));
    ctx.lineTo(w, yOf(db));
    ctx.stroke();
    if (db <= 0) ctx.fillText(`${db}`, xOf(db) + 3 * dpr, h - 4 * dpr);
  }

  // 0 dBFS danger zone above the ceiling line.
  ctx.fillStyle = "rgba(240, 106, 88, 0.06)";
  ctx.fillRect(0, 0, w, yOf(0));

  // Unity reference.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.35)";
  ctx.setLineDash([4 * dpr, 4 * dpr]);
  ctx.beginPath();
  ctx.moveTo(xOf(DB_LO), yOf(DB_LO));
  ctx.lineTo(xOf(DB_HI), yOf(DB_HI));
  ctx.stroke();
  ctx.setLineDash([]);

  // Ceiling line.
  const ceilOut = ceiling + output;
  ctx.strokeStyle = "rgba(240, 106, 88, 0.55)";
  ctx.setLineDash([2 * dpr, 3 * dpr]);
  ctx.beginPath();
  ctx.moveTo(0, yOf(ceilOut));
  ctx.lineTo(w, yOf(ceilOut));
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = "rgba(240, 106, 88, 0.8)";
  ctx.fillText(`ceiling ${ceilOut.toFixed(1)} dB`, 6 * dpr, yOf(ceilOut) - 4 * dpr);

  // Curve.
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const x = DB_LO + (px / w) * (DB_HI - DB_LO);
    const y = yOf(transfer(x, input, threshold, ceiling, output));
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2 * dpr;
  ctx.shadowColor = accent;
  ctx.shadowBlur = 6 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;

  // Limit-start knee dot (input level where limiting begins).
  const kneeIn = threshold - input;
  ctx.beginPath();
  ctx.arc(xOf(kneeIn), yOf(ceilOut), 4 * dpr, 0, Math.PI * 2);
  ctx.fillStyle = accent;
  ctx.fill();
});

let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = {
    x: e.clientX,
    y: e.clientY,
    threshold: params.get(P.threshold),
    ceiling: params.get(P.ceiling),
  };
  canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const dbPerPxX = (DB_HI - DB_LO) / rect.width;
  const dbPerPxY = (DB_HI - DB_LO) / rect.height;
  const threshold = clamp(dragStart.threshold + (e.clientX - dragStart.x) * dbPerPxX, -24, 0);
  const ceiling = clamp(dragStart.ceiling - (e.clientY - dragStart.y) * dbPerPxY, -24, 0);
  params.set(P.threshold, threshold);
  params.set(P.ceiling, ceiling);
  sendSet(P.threshold, threshold);
  sendSet(P.ceiling, ceiling);
  viz.redraw();
});

canvas.addEventListener("pointerup", () => {
  dragStart = null;
});
