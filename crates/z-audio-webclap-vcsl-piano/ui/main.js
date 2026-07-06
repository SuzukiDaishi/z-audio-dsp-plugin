// Z Audio VCSL Piano UI — velocity-response editor + tone/release.
//
// The canvas plots how MIDI velocity maps to loudness for the sampled
// piano. Dragging the curve up makes soft playing louder (compressed
// response); down demands harder playing (expanded). Tone is drawn as a
// brightness tint on the curve itself.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  masterGain: 180,
  tone: 181,
  velocityCurve: 182,
  releaseLevel: 183,
  releaseTime: 184,
  stereoWidth: 185,
};

const PARAMS = [
  { id: P.masterGain, label: "Master", kind: "slider", min: -24, max: 12, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-tone" },
  { id: P.tone, label: "Tone", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-tone" },
  { id: P.stereoWidth, label: "Width", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-tone" },
  { id: P.velocityCurve, label: "Vel Curve", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-release" },
  { id: P.releaseLevel, label: "Rel Level", kind: "slider", min: -24, max: 12, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-release" },
  { id: P.releaseTime, label: "Rel Time", kind: "slider", min: 0.05, max: 5, default: 0.35, scale: "log", fmt: fmt.s, mount: "#sec-release" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Velocity shaping: curve 0.5 is linear-ish; lower = harder (expanded),
// higher = softer playing gets louder (compressed). Exponent mirrors the
// synth-side shaping.
function shape(v01, curve) {
  const exponent = 2 - clamp(curve, 0, 1) * 2 + 0.0001;
  return Math.pow(clamp(v01, 0, 1), exponent);
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const curve = params.get(P.velocityCurve);
  const tone = params.get(P.tone);
  const pad = 10 * dpr;

  // Grid quarters.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.12)";
  ctx.fillStyle = "rgba(126, 147, 163, 0.5)";
  ctx.font = `${9 * dpr}px sans-serif`;
  for (let i = 1; i < 4; i++) {
    const x = pad + ((w - 2 * pad) * i) / 4;
    const y = h - pad - ((h - 2 * pad) * i) / 4;
    ctx.beginPath();
    ctx.moveTo(x, pad);
    ctx.lineTo(x, h - pad);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(pad, y);
    ctx.lineTo(w - pad, y);
    ctx.stroke();
  }
  ctx.fillText("pp", pad + 2 * dpr, h - pad - 3 * dpr);
  ctx.fillText("ff", w - pad - 12 * dpr, h - pad - 3 * dpr);

  // Diagonal reference.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.3)";
  ctx.setLineDash([4 * dpr, 4 * dpr]);
  ctx.beginPath();
  ctx.moveTo(pad, h - pad);
  ctx.lineTo(w - pad, pad);
  ctx.stroke();
  ctx.setLineDash([]);

  // Velocity curve — hue shifts warmer/darker as Tone rolls off.
  const bright = Math.round(197 + tone * 40);
  const color = `rgb(232, ${bright}, ${Math.round(106 + (1 - tone) * 40)})`;
  ctx.beginPath();
  for (let px = 0; px <= w - 2 * pad; px++) {
    const v = px / (w - 2 * pad);
    const y = h - pad - shape(v, curve) * (h - 2 * pad);
    if (px === 0) ctx.moveTo(pad + px, y);
    else ctx.lineTo(pad + px, y);
  }
  ctx.strokeStyle = color;
  ctx.lineWidth = 2 * dpr;
  ctx.shadowColor = color;
  ctx.shadowBlur = 6 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;
  ctx.lineWidth = 1;

  // Fill under curve.
  ctx.beginPath();
  ctx.moveTo(pad, h - pad);
  for (let px = 0; px <= w - 2 * pad; px += 2) {
    const v = px / (w - 2 * pad);
    ctx.lineTo(pad + px, h - pad - shape(v, curve) * (h - 2 * pad));
  }
  ctx.lineTo(w - pad, h - pad);
  ctx.closePath();
  ctx.fillStyle = "rgba(232, 197, 106, 0.08)";
  ctx.fill();

  // Mid-velocity handle dot.
  const hx = pad + (w - 2 * pad) * 0.5;
  const hy = h - pad - shape(0.5, curve) * (h - 2 * pad);
  ctx.beginPath();
  ctx.arc(hx, hy, 4.5 * dpr, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
});

// Vertical drag anywhere on the canvas bends the curve.
let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = { y: e.clientY, curve: params.get(P.velocityCurve) };
  canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const curve = clamp(dragStart.curve - (e.clientY - dragStart.y) / rect.height, 0, 1);
  params.set(P.velocityCurve, curve);
  sendSet(P.velocityCurve, curve);
  viz.redraw();
});

canvas.addEventListener("pointerup", () => {
  dragStart = null;
});
