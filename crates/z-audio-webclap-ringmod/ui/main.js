// Z Audio Ring Mod UI — carrier × input preview.
//
// The canvas shows a reference sine multiplied by the carrier at its
// current wave/frequency/mix. Drag horizontally to sweep the carrier
// frequency (log), vertically to set the mix.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  freq: 620,
  wave: 621,
  stereo: 622,
  mix: 623,
  output: 624,
};

const PARAMS = [
  { id: P.freq, label: "Frequency", kind: "slider", min: 0.5, max: 8000, default: 440, scale: "log", fmt: fmt.hz, mount: "#sec-carrier" },
  { id: P.wave, label: "Wave", kind: "select", options: ["Sine", "Tri", "Saw", "Square"], default: 0, mount: "#sec-carrier" },
  { id: P.stereo, label: "Stereo", kind: "slider", min: 0, max: 180, default: 0, step: 1, fmt: (v) => `${v.toFixed(0)}°`, mount: "#sec-carrier" },
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

function carrier(wave, phase) {
  const t = phase - Math.floor(phase);
  if (wave === 1) return t < 0.5 ? 4 * t - 1 : 3 - 4 * t;
  if (wave === 2) return 2 * t - 1;
  if (wave === 3) return t < 0.5 ? 1 : -1;
  return Math.sin(2 * Math.PI * t);
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const freq = params.get(P.freq);
  const wave = Math.round(params.get(P.wave));
  const mix = params.get(P.mix);
  const midY = h / 2;
  const amp = h * 0.38;

  // Reference input: one low-frequency sine cycle across the window; the
  // carrier rides at its relative rate (windows shows 20 ms).
  const windowSeconds = 0.02;
  const inputHz = 100;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  // Dry input, faint.
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const t = (px / w) * windowSeconds;
    const y = midY - Math.sin(2 * Math.PI * inputHz * t) * amp;
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = "rgba(126, 147, 163, 0.4)";
  ctx.lineWidth = 1.2 * dpr;
  ctx.stroke();

  // Modulated result.
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const t = (px / w) * windowSeconds;
    const dry = Math.sin(2 * Math.PI * inputHz * t);
    const c = carrier(wave, freq * t);
    const out = dry * (1 - mix) + dry * c * mix;
    const y = midY - out * amp;
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
  ctx.fillText(`carrier ${fmt.hz(freq)} · mix ${fmt.pct(mix)}`, w - 6 * dpr, 12 * dpr);
  ctx.textAlign = "left";
});

const FREQ_LO = Math.log(0.5);
const FREQ_HI = Math.log(8000);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const freq = Math.exp(FREQ_LO + tx * (FREQ_HI - FREQ_LO));
  const mix = 1 - ty;
  params.set(P.freq, freq);
  params.set(P.mix, mix);
  sendSet(P.freq, freq);
  sendSet(P.mix, mix);
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
