// Z Audio Phaser UI — allpass-cascade notch response preview.
//
// The canvas plots |H(f)| of the dry+wet mix with the LFO at zero (sweep
// frequency = center), plus the shaded sweep range set by Depth. Drag
// horizontally to move the center frequency (log), vertically the depth.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  stages: 860,
  rate: 861,
  center: 862,
  depth: 863,
  feedback: 864,
  spread: 865,
  mix: 866,
  output: 867,
};

const PARAMS = [
  { id: P.stages, label: "Stages", kind: "slider", min: 1, max: 6, default: 3, step: 1, fmt: (v) => `${Math.round(v) * 2}`, mount: "#sec-sweep" },
  { id: P.rate, label: "Rate", kind: "slider", min: 0.02, max: 5, default: 0.4, scale: "log", fmt: fmt.hzLfo, mount: "#sec-sweep" },
  { id: P.center, label: "Center", kind: "slider", min: 100, max: 8000, default: 1000, scale: "log", fmt: fmt.hz, mount: "#sec-sweep" },
  { id: P.depth, label: "Depth", kind: "slider", min: 0, max: 1, default: 0.7, step: 0.01, fmt: fmt.pct, mount: "#sec-sweep" },
  { id: P.spread, label: "Spread", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-sweep" },
  { id: P.feedback, label: "Feedback", kind: "slider", min: 0, max: 0.9, default: 0.3, step: 0.01, fmt: fmt.pct, mount: "#sec-output" },
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

const FS = 48000; // display sample rate; matches the engine default

// Mirrors src/lib.rs: coefficient a = (t-1)/(t+1), t = tan(pi*f/fs), with the
// sweep at its LFO-zero point (f = center clamped to 30..fs*0.45). Each stage
// is H(z) = (a + z^-1)/(1 + a z^-1); the cascade is H^stages, the feedback
// loop makes the wet path H/(1 - fb*H), and the output mixes
// |(1-mix) + mix*wet|.
function responseDb(f, center, stages, fb, mix) {
  const sweep = clamp(center, 30, FS * 0.45);
  const t = Math.tan((Math.PI * sweep) / FS);
  const a = (t - 1) / (t + 1);
  const w = (2 * Math.PI * f) / FS;
  const zr = Math.cos(-w); // z^-1 = e^{-jω}
  const zi = Math.sin(-w);
  // One stage: (a + z^-1) / (1 + a z^-1)
  const nr = a + zr;
  const ni = zi;
  const dr = 1 + a * zr;
  const di = a * zi;
  const dm = dr * dr + di * di;
  const h1r = (nr * dr + ni * di) / dm;
  const h1i = (ni * dr - nr * di) / dm;
  // Cascade: multiply the complex response `stages` times.
  let hr = 1;
  let hi = 0;
  for (let k = 0; k < stages; k++) {
    const r = hr * h1r - hi * h1i;
    hi = hr * h1i + hi * h1r;
    hr = r;
  }
  // Feedback around the cascade: wet = H / (1 - fb*H).
  const fr = 1 - fb * hr;
  const fi = -fb * hi;
  const fm = fr * fr + fi * fi;
  const wr = (hr * fr + hi * fi) / fm;
  const wi = (hi * fr - hr * fi) / fm;
  const outR = 1 - mix + mix * wr;
  const outI = mix * wi;
  return 20 * Math.log10(Math.max(Math.hypot(outR, outI), 1e-6));
}

const FREQ_MIN = 20;
const FREQ_MAX = 20000;
const LOG_LO = Math.log(FREQ_MIN);
const LOG_HI = Math.log(FREQ_MAX);
const DB_RANGE = 30; // vertical axis ±30 dB

const xOfFreq = (f, w) => ((Math.log(clamp(f, FREQ_MIN, FREQ_MAX)) - LOG_LO) / (LOG_HI - LOG_LO)) * w;

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const stages = Math.round(params.get(P.stages)) * 2;
  const center = params.get(P.center);
  const depth = params.get(P.depth);
  const fb = params.get(P.feedback);
  const mix = params.get(P.mix);
  const yOfDb = (db) => h / 2 - (clamp(db, -DB_RANGE, DB_RANGE) / DB_RANGE) * (h / 2);

  // Frequency grid.
  ctx.font = `${9 * dpr}px sans-serif`;
  for (const f of [50, 100, 200, 500, 1000, 2000, 5000, 10000]) {
    const x = xOfFreq(f, w);
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

  // Sweep range (center * 2^(±depth*2)), shaded.
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  const accentSoft = getComputedStyle(document.documentElement)
    .getPropertyValue("--accent-soft")
    .trim();
  const x0 = xOfFreq(center * Math.pow(2, -depth * 2), w);
  const x1 = xOfFreq(center * Math.pow(2, depth * 2), w);
  ctx.fillStyle = accentSoft || "rgba(160, 108, 224, 0.16)";
  ctx.fillRect(x0, 0, Math.max(x1 - x0, 1), h);

  // Notch curve at the LFO-zero sweep point.
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const f = Math.exp(LOG_LO + (px / w) * (LOG_HI - LOG_LO));
    const y = yOfDb(responseDb(f, center, stages, fb, mix));
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
    `${stages} stages · center ${fmt.hz(center)} · depth ${fmt.pct(depth)}`,
    w - 6 * dpr,
    12 * dpr,
  );
  ctx.textAlign = "left";
});

const CENTER_LO = Math.log(100);
const CENTER_HI = Math.log(8000);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const center = Math.exp(CENTER_LO + tx * (CENTER_HI - CENTER_LO));
  const depth = 1 - ty;
  params.set(P.center, center);
  params.set(P.depth, depth);
  sendSet(P.center, center);
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
