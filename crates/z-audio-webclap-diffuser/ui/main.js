// Z Audio Diffuser UI — echo-density scatter.
//
// The diffuser is a chain of up to 100 allpasses; what you hear is an
// impulse smearing into a dense cloud. The viz draws exactly that: one
// dot per emerging echo — horizontal position is its arrival time (spread
// by Size, count by AP Count), vertical position is its stereo placement
// (Width), and dot density/opacity follows Diffusion and Mix. Drag the
// cloud to reshape it: horizontal = Size, vertical = Diffusion.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  mix: 220,
  diffusion: 221,
  size: 222,
  width: 223,
  output: 224,
  allpassCount: 225,
};

const PARAMS = [
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-diff" },
  { id: P.diffusion, label: "Diffusion", kind: "slider", min: 0, max: 1, default: 0.04, step: 0.01, fmt: fmt.pct, mount: "#sec-diff" },
  { id: P.allpassCount, label: "AP Count", kind: "slider", min: 1, max: 100, default: 100, step: 1, fmt: fmt.int, mount: "#sec-diff" },
  { id: P.size, label: "Size", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-diff" },
  { id: P.width, label: "Width", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-out" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-out" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

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

  const mix = params.get(P.mix);
  const diffusion = params.get(P.diffusion);
  const size = params.get(P.size);
  const width = params.get(P.width);
  const count = Math.round(params.get(P.allpassCount));
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  const mid = h / 2;

  // Stereo lane guides.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.15)";
  for (const frac of [0.5]) {
    ctx.beginPath();
    ctx.moveTo(0, h * frac);
    ctx.lineTo(w, h * frac);
    ctx.stroke();
  }
  ctx.fillStyle = "rgba(126, 147, 163, 0.5)";
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.fillText("L", 6 * dpr, 12 * dpr);
  ctx.fillText("R", 6 * dpr, h - 6 * dpr);

  // Dry impulse.
  ctx.strokeStyle = "#cfe7db";
  ctx.lineWidth = 2 * dpr;
  ctx.beginPath();
  ctx.moveTo(3 * dpr, mid - h * 0.42 * (1 - mix * 0.6));
  ctx.lineTo(3 * dpr, mid + h * 0.42 * (1 - mix * 0.6));
  ctx.stroke();
  ctx.lineWidth = 1;

  // Echo cloud: each allpass stage emits echoes further out; diffusion
  // multiplies how many, size stretches arrival times.
  const rand = mulberry(7);
  const echoes = Math.round(count * (2 + diffusion * 10));
  const spanBoost = 0.15 + size * 0.85;
  for (let i = 0; i < echoes; i++) {
    const stage = Math.floor(rand() * count) + 1;
    const frac = stage / count;
    // Later stages arrive later and denser; jitter keeps it organic.
    const t = Math.pow(frac, 1.2) * spanBoost * (0.75 + rand() * 0.5);
    const x = 8 * dpr + t * (w - 16 * dpr);
    const pan = (rand() * 2 - 1) * width;
    const y = mid + pan * h * 0.42;
    const decay = Math.exp(-2.2 * t);
    const alpha = mix * (0.12 + 0.55 * decay) * (0.4 + diffusion * 0.6);
    const r = dpr * (1 + 2.2 * decay);
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(79, 200, 209, ${alpha.toFixed(3)})`;
    ctx.fill();
  }

  // Density envelope hint.
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const t = px / w;
    const env = Math.exp(-2.2 * t) * mix;
    const y = mid - h * 0.45 * env;
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent;
  ctx.globalAlpha = 0.5;
  ctx.stroke();
  ctx.globalAlpha = 1;
});

let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = {
    x: e.clientX,
    y: e.clientY,
    size: params.get(P.size),
    diffusion: params.get(P.diffusion),
  };
  canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const size = clamp(dragStart.size + (e.clientX - dragStart.x) / rect.width, 0, 1);
  const diffusion = clamp(dragStart.diffusion - (e.clientY - dragStart.y) / rect.height, 0, 1);
  params.set(P.size, size);
  params.set(P.diffusion, diffusion);
  sendSet(P.size, size);
  sendSet(P.diffusion, diffusion);
  viz.redraw();
});

canvas.addEventListener("pointerup", () => {
  dragStart = null;
});
