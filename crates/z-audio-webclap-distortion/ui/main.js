// Z Audio Distortion UI — waveshaper transfer curve.
//
// The canvas mirrors src/lib.rs `shape()` exactly: x is the input sample
// (with drive applied), y the shaped output. Drag vertically to set drive.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  drive: 640,
  type: 641,
  tone: 642,
  mix: 643,
  output: 644,
};

const PARAMS = [
  { id: P.drive, label: "Drive", kind: "slider", min: 0, max: 36, default: 12, step: 0.1, fmt: fmt.db, mount: "#sec-shape" },
  { id: P.type, label: "Type", kind: "select", options: ["Soft", "Hard", "Fold", "Asym"], default: 0, mount: "#sec-shape" },
  { id: P.tone, label: "Tone", kind: "slider", min: 200, max: 20000, default: 20000, scale: "log", fmt: fmt.hz, mount: "#sec-output" },
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

// Mirrors src/lib.rs shape().
function shape(type, x) {
  if (type === 1) return clamp(x, -1, 1);
  if (type === 2) return Math.sin((Math.PI / 2) * x);
  if (type === 3) {
    const t = Math.tanh(x);
    return Math.tanh(x + 0.35 * t * t);
  }
  return Math.tanh(x);
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const drive = Math.pow(10, params.get(P.drive) / 20);
  const type = Math.round(params.get(P.type));
  const mix = params.get(P.mix);

  const xOf = (v) => ((v + 1) / 2) * w; // input -1..1
  const yOf = (v) => h - ((v + 1.2) / 2.4) * h; // output -1.2..1.2

  // Grid.
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

  // Unity reference.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.35)";
  ctx.setLineDash([4 * dpr, 4 * dpr]);
  ctx.beginPath();
  ctx.moveTo(xOf(-1), yOf(-1));
  ctx.lineTo(xOf(1), yOf(1));
  ctx.stroke();
  ctx.setLineDash([]);

  // Transfer curve at the current drive (dry/wet blended like the DSP).
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const x = (px / w) * 2 - 1;
    const wet = shape(type, x * drive);
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
  ctx.fillText(`drive ${fmt.db(params.get(P.drive))}`, w - 6 * dpr, 12 * dpr);
  ctx.textAlign = "left";
});

let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = { y: e.clientY, drive: params.get(P.drive) };
  canvas.setPointerCapture(e.pointerId);
});
canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const drive = clamp(dragStart.drive + ((dragStart.y - e.clientY) / rect.height) * 36, 0, 36);
  params.set(P.drive, drive);
  sendSet(P.drive, drive);
  viz.redraw();
});
canvas.addEventListener("pointerup", () => {
  dragStart = null;
});
