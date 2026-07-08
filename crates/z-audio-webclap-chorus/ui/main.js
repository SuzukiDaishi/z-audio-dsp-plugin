// Z Audio Chorus UI — per-voice delay LFO curves.
//
// The canvas plots each voice's modulated delay time over one LFO cycle:
// left-channel voices as solid accent lines, right-channel voices dashed
// and dimmer (offset by spread). Drag horizontally to sweep the rate
// (log), vertically to set the depth.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  rate: 820,
  depth: 821,
  voices: 822,
  spread: 823,
  mix: 824,
  output: 825,
};

const PARAMS = [
  { id: P.rate, label: "Rate", kind: "slider", min: 0.05, max: 8, default: 0.8, scale: "log", fmt: fmt.hzLfo, mount: "#sec-mod" },
  { id: P.depth, label: "Depth", kind: "slider", min: 0, max: 1, default: 0.5, step: 0.01, fmt: fmt.pct, mount: "#sec-mod" },
  { id: P.voices, label: "Voices", kind: "select", options: [{ value: 1, label: "1" }, { value: 2, label: "2" }, { value: 3, label: "3" }], default: 2, mount: "#sec-mod" },
  { id: P.spread, label: "Spread", kind: "slider", min: 0, max: 1, default: 0.7, step: 0.01, fmt: fmt.pct, mount: "#sec-mod" },
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

// Mirrors src/lib.rs voice_delay_ms(): 7 ms base + depth-scaled 0..8 ms
// sine sweep + 3 ms static per-voice offset; voice v runs at phase offset
// v/voices, and right-channel voices add spread*0.5 cycles.
function voiceDelayMs(depth, voice, phase) {
  return 7 + depth * 8 * (0.5 + 0.5 * Math.sin(2 * Math.PI * phase)) + voice * 3;
}

const canvas = document.getElementById("viz");
const MAX_MS = 24; // y axis 0..24 ms (worst case tap: 7 + 8 + 2*3 = 21 ms)

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const rate = params.get(P.rate);
  const depth = params.get(P.depth);
  const voices = Math.round(params.get(P.voices));
  const spread = params.get(P.spread);

  const yOf = (ms) => h - (ms / MAX_MS) * h;

  // Grid every 8 ms.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.12)";
  ctx.lineWidth = 1;
  ctx.fillStyle = "rgba(126, 147, 163, 0.5)";
  ctx.font = `${9 * dpr}px sans-serif`;
  for (let ms = 8; ms < MAX_MS; ms += 8) {
    ctx.beginPath();
    ctx.moveTo(0, yOf(ms));
    ctx.lineTo(w, yOf(ms));
    ctx.stroke();
    ctx.fillText(`${ms} ms`, 6 * dpr, yOf(ms) - 3 * dpr);
  }

  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  // One LFO cycle across the canvas, per voice, both channels.
  for (const channel of ["L", "R"]) {
    for (let v = 0; v < voices; v++) {
      ctx.beginPath();
      for (let px = 0; px <= w; px++) {
        const phase = px / w + v / voices + (channel === "R" ? spread * 0.5 : 0);
        const y = yOf(voiceDelayMs(depth, v, phase));
        if (px === 0) ctx.moveTo(px, y);
        else ctx.lineTo(px, y);
      }
      if (channel === "R") {
        ctx.strokeStyle = "rgba(126, 199, 255, 0.5)";
        ctx.lineWidth = 1.4 * dpr;
        ctx.setLineDash([4 * dpr, 4 * dpr]);
        ctx.shadowBlur = 0;
      } else {
        ctx.strokeStyle = accent;
        ctx.lineWidth = 2 * dpr;
        ctx.setLineDash([]);
        ctx.shadowColor = accent;
        ctx.shadowBlur = 5 * dpr;
      }
      ctx.stroke();
    }
  }
  ctx.setLineDash([]);
  ctx.shadowBlur = 0;

  ctx.fillStyle = "rgba(126, 147, 163, 0.7)";
  ctx.textAlign = "right";
  ctx.fillText(
    `rate ${fmt.hzLfo(rate)} · depth ${fmt.pct(depth)} · ${voices} voice${voices > 1 ? "s" : ""}`,
    w - 6 * dpr,
    12 * dpr
  );
  ctx.textAlign = "left";
});

const RATE_LO = Math.log(0.05);
const RATE_HI = Math.log(8);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const rate = Math.exp(RATE_LO + tx * (RATE_HI - RATE_LO));
  const depth = 1 - ty;
  params.set(P.rate, rate);
  params.set(P.depth, depth);
  sendSet(P.rate, rate);
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
