// Z Audio Bitcrusher UI — crushed-sine preview.
//
// The canvas runs a reference sine through the exact quantize +
// sample-and-hold the DSP applies. Drag vertically for bit depth,
// horizontally for the downsample factor.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  bits: 680,
  downsample: 681,
  mix: 682,
  output: 683,
};

const PARAMS = [
  { id: P.bits, label: "Bits", kind: "slider", min: 1, max: 16, default: 8, step: 0.1, fmt: (v) => `${v.toFixed(1)} bit`, mount: "#sec-crush" },
  { id: P.downsample, label: "Downsample", kind: "slider", min: 1, max: 64, default: 4, scale: "log", fmt: fmt.x, mount: "#sec-crush" },
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

// Mirrors src/lib.rs quantize().
function quantize(bits, x) {
  const step = Math.pow(2, 1 - clamp(bits, 1, 16));
  return clamp(Math.round(x / step) * step, -1, 1);
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const bits = params.get(P.bits);
  const factor = params.get(P.downsample);
  const mix = params.get(P.mix);
  const midY = h / 2;
  const amp = h * 0.38;

  // Simulate the crusher over a virtual 48 kHz stream showing 20 ms.
  const sampleRate = 48000;
  const samples = Math.floor(sampleRate * 0.02);
  const inputHz = 150;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  // Dry input, faint.
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const t = (px / w) * 0.02;
    const y = midY - Math.sin(2 * Math.PI * inputHz * t) * amp * 0.9;
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = "rgba(126, 147, 163, 0.4)";
  ctx.lineWidth = 1.2 * dpr;
  ctx.stroke();

  // Crushed result.
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  let counter = 0;
  let held = 0;
  for (let i = 0; i < samples; i++) {
    const t = i / sampleRate;
    const dry = Math.sin(2 * Math.PI * inputHz * t) * 0.9;
    counter += 1;
    if (counter >= factor) {
      counter -= factor;
      held = quantize(bits, dry);
    }
    const out = dry * (1 - mix) + held * mix;
    const px = (i / samples) * w;
    const y = midY - out * amp;
    if (i === 0) ctx.moveTo(px, y);
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
  ctx.fillText(`${bits.toFixed(1)} bit · ${factor.toFixed(1)}x down`, w - 6 * dpr, 12 * dpr);
  ctx.textAlign = "left";
});

const DS_LO = Math.log(1);
const DS_HI = Math.log(64);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const downsample = Math.exp(DS_LO + tx * (DS_HI - DS_LO));
  const bits = 16 - ty * 15;
  params.set(P.downsample, downsample);
  params.set(P.bits, bits);
  sendSet(P.downsample, downsample);
  sendSet(P.bits, bits);
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
