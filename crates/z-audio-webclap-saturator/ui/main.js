// Z Audio Saturator UI — level-compensated saturation curve.
//
// The canvas mirrors src/lib.rs `saturate()` exactly. Drag vertically to
// set drive, horizontally to set warmth (even-harmonic asymmetry — watch
// the curve skew).

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  drive: 660,
  warmth: 661,
  tone: 662,
  mix: 663,
  output: 664,
};

const PARAMS = [
  { id: P.drive, label: "Drive", kind: "slider", min: 0, max: 24, default: 6, step: 0.1, fmt: fmt.db, mount: "#sec-drive" },
  { id: P.warmth, label: "Warmth", kind: "slider", min: 0, max: 1, default: 0.3, step: 0.01, fmt: fmt.pct, mount: "#sec-drive" },
  { id: P.tone, label: "Tone", kind: "slider", min: -1, max: 1, default: 0, step: 0.01, fmt: (v) => (Math.abs(v) < 0.005 ? "flat" : v < 0 ? `dark ${(-v * 100).toFixed(0)}%` : `bright ${(v * 100).toFixed(0)}%`), mount: "#sec-output" },
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-output" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-output" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Mirrors src/lib.rs saturate().
function saturate(g, warmth, x) {
  const driven = g * x;
  const t = Math.tanh(driven);
  const bias = warmth * 0.4 * t * t;
  return Math.tanh(driven + bias) / Math.max(Math.tanh(g), 1e-3);
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const g = Math.max(1, Math.pow(10, params.get(P.drive) / 20));
  const warmth = params.get(P.warmth);
  const mix = params.get(P.mix);

  const xOf = (v) => ((v + 1) / 2) * w;
  const yOf = (v) => h - ((v + 1.4) / 2.8) * h;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.12)";
  for (const v of [-1, -0.5, 0, 0.5, 1]) {
    ctx.beginPath();
    ctx.moveTo(xOf(v), 0);
    ctx.lineTo(xOf(v), h);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(0, yOf(v));
    ctx.lineTo(w, yOf(v));
    ctx.stroke();
  }

  ctx.strokeStyle = "rgba(126, 147, 163, 0.35)";
  ctx.setLineDash([4 * dpr, 4 * dpr]);
  ctx.beginPath();
  ctx.moveTo(xOf(-1), yOf(-1));
  ctx.lineTo(xOf(1), yOf(1));
  ctx.stroke();
  ctx.setLineDash([]);

  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const x = (px / w) * 2 - 1;
    const wet = saturate(g, warmth, x);
    const y = yOf(x * (1 - mix) + wet * mix);
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
    `drive ${fmt.db(params.get(P.drive))} · warmth ${fmt.pct(warmth)}`,
    w - 6 * dpr,
    12 * dpr,
  );
  ctx.textAlign = "left";
});

let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = {
    x: e.clientX,
    y: e.clientY,
    drive: params.get(P.drive),
    warmth: params.get(P.warmth),
  };
  canvas.setPointerCapture(e.pointerId);
});
canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const drive = clamp(dragStart.drive + ((dragStart.y - e.clientY) / rect.height) * 24, 0, 24);
  const warmth = clamp(dragStart.warmth + (e.clientX - dragStart.x) / rect.width, 0, 1);
  params.set(P.drive, drive);
  params.set(P.warmth, warmth);
  sendSet(P.drive, drive);
  sendSet(P.warmth, warmth);
  viz.redraw();
});
canvas.addEventListener("pointerup", () => {
  dragStart = null;
});
