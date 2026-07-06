// Z Audio Parametric Reverb UI — stylized impulse-response display.
//
// The viz draws what the parameters *mean*: a pre-delay gap, sparse early
// reflections (spacing set by Room, density by Diffusion), and a late
// tail whose length follows Decay, brightness decay follows Damping, and
// stereo spread follows Width. Dragging the tail edits Decay (horizontal)
// and Damping (vertical).

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  mix: 100,
  room: 101,
  decay: 102,
  preDelay: 103,
  diffusion: 104,
  damping: 105,
  lowCut: 106,
  highCut: 107,
  modRate: 108,
  modDepth: 109,
  width: 110,
  earlyLate: 111,
  output: 112,
};

const PARAMS = [
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 0.35, step: 0.01, fmt: fmt.pct, mount: "#sec-space" },
  { id: P.room, label: "Room", kind: "slider", min: 0, max: 1, default: 0.55, step: 0.01, fmt: fmt.pct, mount: "#sec-space" },
  { id: P.decay, label: "Decay", kind: "slider", min: 0.1, max: 20, default: 2.2, scale: "log", fmt: fmt.s, mount: "#sec-space" },
  { id: P.preDelay, label: "Pre Delay", kind: "slider", min: 0, max: 250, default: 18, step: 0.1, fmt: fmt.ms, mount: "#sec-space" },
  { id: P.earlyLate, label: "Early/Late", kind: "slider", min: 0, max: 1, default: 0.35, step: 0.01, fmt: fmt.pct, mount: "#sec-space" },
  { id: P.diffusion, label: "Diffusion", kind: "slider", min: 0, max: 1, default: 0.65, step: 0.01, fmt: fmt.pct, mount: "#sec-texture" },
  { id: P.damping, label: "Damping", kind: "slider", min: 0, max: 1, default: 0.35, step: 0.01, fmt: fmt.pct, mount: "#sec-texture" },
  { id: P.width, label: "Width", kind: "slider", min: 0, max: 1, default: 0.9, step: 0.01, fmt: fmt.pct, mount: "#sec-texture" },
  { id: P.modRate, label: "Mod Rate", kind: "slider", min: 0, max: 2, default: 0, step: 0.01, fmt: fmt.hzLfo, mount: "#sec-texture" },
  { id: P.modDepth, label: "Mod Depth", kind: "slider", min: 0, max: 1, default: 0, step: 0.01, fmt: fmt.pct, mount: "#sec-texture" },
  { id: P.lowCut, label: "Low Cut", kind: "slider", min: 20, max: 1000, default: 80, scale: "log", fmt: fmt.hz, mount: "#sec-tone" },
  { id: P.highCut, label: "High Cut", kind: "slider", min: 1000, max: 20000, default: 12000, scale: "log", fmt: fmt.hz, mount: "#sec-tone" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-tone" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// ---------------------------------------------------------------------------
// Impulse-response drawing. Time axis spans preDelayMax + 20 s (log-ish
// squash so short rooms still read); amplitude decays exponentially.
// ---------------------------------------------------------------------------

// Deterministic pseudo-random for reflection placement (stable redraws).
function mulberry(seed) {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const decay = params.get(P.decay);
  const preDelay = params.get(P.preDelay) / 1000;
  const room = params.get(P.room);
  const diffusion = params.get(P.diffusion);
  const damping = params.get(P.damping);
  const width = params.get(P.width);
  const earlyLate = params.get(P.earlyLate);
  const mix = params.get(P.mix);
  const modDepth = params.get(P.modDepth);

  // Time span: squash long decays so the shape always fits.
  const span = Math.max(0.8, preDelay + decay * 1.15);
  const xOf = (t) => (t / span) * w;
  const mid = h / 2;
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();

  // Center line.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.2)";
  ctx.beginPath();
  ctx.moveTo(0, mid);
  ctx.lineTo(w, mid);
  ctx.stroke();

  // Dry impulse at t=0.
  ctx.strokeStyle = "#cfe7db";
  ctx.lineWidth = 2 * dpr;
  ctx.beginPath();
  ctx.moveTo(2 * dpr, mid - (h / 2) * 0.92 * (1 - mix * 0.5));
  ctx.lineTo(2 * dpr, mid + (h / 2) * 0.92 * (1 - mix * 0.5));
  ctx.stroke();
  ctx.lineWidth = 1;

  // Pre-delay gap marker.
  if (preDelay > 0.002) {
    ctx.strokeStyle = "rgba(157, 123, 240, 0.35)";
    ctx.setLineDash([3 * dpr, 3 * dpr]);
    ctx.beginPath();
    ctx.moveTo(xOf(preDelay), mid - h * 0.42);
    ctx.lineTo(xOf(preDelay), mid + h * 0.42);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  const wet = 0.25 + 0.75 * mix;

  // Early reflections: count by diffusion, spacing by room size.
  const rand = mulberry(42);
  const earlyCount = Math.round(4 + diffusion * 20);
  const earlySpan = 0.02 + room * 0.09;
  const earlyGain = wet * (1 - earlyLate * 0.7);
  ctx.strokeStyle = "rgba(207, 231, 219, 0.75)";
  for (let i = 0; i < earlyCount; i++) {
    const t = preDelay + earlySpan * Math.pow((i + 1) / earlyCount, 1.4) * (0.8 + rand() * 0.4);
    const amp = earlyGain * (1 - i / earlyCount) * (0.5 + rand() * 0.5);
    const side = (rand() * 2 - 1) * width;
    const top = amp * (1 + side * 0.4);
    const bottom = amp * (1 - side * 0.4);
    ctx.beginPath();
    ctx.moveTo(xOf(t), mid - (h / 2) * 0.9 * top);
    ctx.lineTo(xOf(t), mid + (h / 2) * 0.9 * bottom);
    ctx.stroke();
  }

  // Late tail: dense strokes under an exp envelope. Damping darkens and
  // shortens the "bright" inner band; modulation wobbles stroke placement.
  const tailStart = preDelay + earlySpan * 0.8;
  const strokes = 160;
  const lateGain = wet * (0.35 + earlyLate * 0.65);
  for (let i = 0; i < strokes; i++) {
    const frac = i / strokes;
    const t = tailStart + frac * decay * 1.1;
    const env = Math.exp((-6.91 * (t - tailStart)) / decay); // -60 dB at `decay`
    if (env < 0.004) break;
    const jitter = (rand() * 2 - 1) * modDepth * 0.06;
    const amp = lateGain * env * (0.6 + rand() * 0.4 + jitter);
    const side = (rand() * 2 - 1) * width;
    const x = xOf(t);
    // Full-band stroke.
    ctx.strokeStyle = `rgba(157, 123, 240, ${0.16 + 0.4 * env})`;
    ctx.beginPath();
    ctx.moveTo(x, mid - (h / 2) * 0.9 * amp * (1 + side * 0.35));
    ctx.lineTo(x, mid + (h / 2) * 0.9 * amp * (1 - side * 0.35));
    ctx.stroke();
    // Bright band dies faster with damping.
    const brightEnv = Math.exp((-6.91 * (t - tailStart)) / (decay * (1 - damping * 0.85)));
    ctx.strokeStyle = `rgba(220, 205, 255, ${0.25 * brightEnv})`;
    ctx.beginPath();
    ctx.moveTo(x, mid - (h / 2) * 0.45 * amp * brightEnv);
    ctx.lineTo(x, mid + (h / 2) * 0.45 * amp * brightEnv);
    ctx.stroke();
  }

  // Decay envelope outline.
  ctx.beginPath();
  for (let px = xOf(tailStart); px <= w; px++) {
    const t = (px / w) * span;
    const env = Math.exp((-6.91 * (t - tailStart)) / decay);
    const y = mid - (h / 2) * 0.9 * lateGain * env;
    if (px === Math.round(xOf(tailStart))) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = 1.5 * dpr;
  ctx.stroke();
  ctx.lineWidth = 1;

  // Time labels.
  ctx.fillStyle = "rgba(126, 147, 163, 0.55)";
  ctx.font = `${9 * dpr}px sans-serif`;
  for (const t of [0.5, 1, 2, 5, 10, 15]) {
    if (t < span * 0.95) ctx.fillText(`${t}s`, xOf(t) + 2 * dpr, h - 4 * dpr);
  }
});

// Drag the tail: horizontal = decay, vertical = damping.
let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = {
    x: e.clientX,
    y: e.clientY,
    decay: params.get(P.decay),
    damping: params.get(P.damping),
  };
  canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const decay = clamp(dragStart.decay * Math.pow(2, (e.clientX - dragStart.x) / (rect.width / 3)), 0.1, 20);
  const damping = clamp(dragStart.damping + (e.clientY - dragStart.y) / rect.height, 0, 1);
  params.set(P.decay, decay);
  params.set(P.damping, damping);
  sendSet(P.decay, decay);
  sendSet(P.damping, damping);
  viz.redraw();
});

canvas.addEventListener("pointerup", () => {
  dragStart = null;
});
