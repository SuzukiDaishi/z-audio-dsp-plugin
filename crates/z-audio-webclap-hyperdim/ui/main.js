// Z Audio Hyper Dimension UI — animated view of the hyper voice cloud.
//
// The canvas mirrors src/lib.rs `hyper_delay_ms`: each unison voice is a
// dot sweeping the 10-18 ms delay field, panned alternately left/right;
// the dimension pair is drawn as two antiphase bars underneath.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  rate: 920,
  detune: 921,
  unison: 922,
  hyperWet: 923,
  dimSize: 924,
  dimWet: 925,
  output: 926,
};

const PARAMS = [
  { id: P.rate, label: "Rate", kind: "slider", min: 0.05, max: 8, default: 1.2, scale: "log", fmt: fmt.hzLfo, mount: "#sec-hyper" },
  { id: P.detune, label: "Detune", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-hyper" },
  {
    id: P.unison, label: "Unison", kind: "select", default: 4, mount: "#sec-hyper",
    options: [1, 2, 3, 4, 5, 6, 7].map((n) => ({ value: n, label: String(n) })),
  },
  { id: P.hyperWet, label: "Hyper Wet", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-hyper" },
  { id: P.dimSize, label: "Size", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-dim" },
  { id: P.dimWet, label: "Dim Wet", kind: "slider", min: 0, max: 1, default: 0.3, step: 0.01, fmt: fmt.pct, mount: "#sec-dim" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-dim" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Mirror of src/lib.rs hyper_delay_ms().
function hyperDelayMs(detune, voice, n, phase) {
  const offset = voice / Math.max(n, 1);
  const lfo = Math.sin(2 * Math.PI * (phase + offset));
  return 10 + voice * 1.3 + detune * 7 * (0.5 + 0.5 * lfo);
}

const canvas = document.getElementById("viz");
let phase = 0;
let lastTime = performance.now();

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();

  const n = Math.round(params.get(P.unison));
  const detune = params.get(P.detune);
  const wet = params.get(P.hyperWet);
  const dimWet = params.get(P.dimWet);
  const dimSize = params.get(P.dimSize);

  // Delay field 8..20 ms mapped across the width.
  const toX = (ms) => ((ms - 8) / 12) * w;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.2)";
  ctx.lineWidth = 1;
  const midY = h * 0.42;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  for (let v = 0; v < n; v++) {
    const ms = hyperDelayMs(detune, v, n, phase);
    const pan = n === 1 ? 0 : v % 2 === 0 ? -1 : 1;
    const y = midY + pan * h * 0.16;
    ctx.globalAlpha = 0.35 + wet * 0.65;
    ctx.fillStyle = accent;
    ctx.beginPath();
    ctx.arc(toX(ms), y, Math.max(3, h / 26), 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.globalAlpha = 1;

  // Dimension pair: two antiphase bars near the bottom.
  const dimBase = 3 + dimSize * 20;
  const dimLfo = Math.sin(2 * Math.PI * phase * 0.2);
  const barY = h * 0.84;
  ctx.fillStyle = accent;
  ctx.globalAlpha = 0.25 + dimWet * 0.6;
  const dl = clamp(((dimBase + 1.5 * dimLfo) - 2) / 24, 0, 1) * w;
  const dr = clamp(((dimBase - 1.5 * dimLfo) - 2) / 24, 0, 1) * w;
  ctx.fillRect(0, barY - 3, dl, 3);
  ctx.fillRect(0, barY + 3, dr, 3);
  ctx.globalAlpha = 1;
});

function animate(now) {
  const rate = params.get(P.rate) || 1.2;
  phase = (phase + ((now - lastTime) / 1000) * rate) % 1;
  lastTime = now;
  viz.redraw();
  requestAnimationFrame(animate);
}
requestAnimationFrame(animate);
