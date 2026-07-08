// Z Audio Gate UI — static transfer curve.
//
// The canvas plots input level vs output level in dB. Drag horizontally
// to set the threshold, vertically to set the range floor.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  threshold: 900,
  attack: 901,
  hold: 902,
  release: 903,
  range: 904,
  output: 905,
};

const PARAMS = [
  { id: P.threshold, label: "Threshold", kind: "slider", min: -70, max: 0, default: -40, step: 0.5, fmt: fmt.db, mount: "#sec-detector" },
  { id: P.attack, label: "Attack", kind: "slider", min: 0.1, max: 100, default: 1, scale: "log", fmt: fmt.ms, mount: "#sec-detector" },
  { id: P.hold, label: "Hold", kind: "slider", min: 0, max: 500, default: 50, step: 1, fmt: fmt.ms, mount: "#sec-detector" },
  { id: P.release, label: "Release", kind: "slider", min: 5, max: 2000, default: 150, scale: "log", fmt: fmt.ms, mount: "#sec-detector" },
  { id: P.range, label: "Range", kind: "slider", min: -80, max: 0, default: -80, step: 0.5, fmt: fmt.db, mount: "#sec-output" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-output" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

const FLOOR = -80; // dB — both axes span FLOOR..0.

// Mirrors the steady-state behavior of src/lib.rs: with the envelope
// held below the threshold the gain settles at `range`, so the output
// sits at input + range dB (clamped to the floor); above the threshold
// the gate is open and output = input. The small smoothstep knee is
// purely cosmetic — the DSP switches at the threshold and relies on
// attack/hold/release for smoothness in time.
function transfer(inDb, thresholdDb, rangeDb) {
  const open = inDb;
  const closed = Math.max(inDb + rangeDb, FLOOR);
  const knee = 6; // dB
  if (inDb >= thresholdDb + knee / 2) return open;
  if (inDb <= thresholdDb - knee / 2) return closed;
  const t = (inDb - (thresholdDb - knee / 2)) / knee;
  const s = t * t * (3 - 2 * t);
  return closed + (open - closed) * s;
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const threshold = params.get(P.threshold);
  const range = params.get(P.range);

  const pad = 8 * dpr;
  const xFor = (db) => pad + ((db - FLOOR) / -FLOOR) * (w - 2 * pad);
  const yFor = (db) => h - pad - ((db - FLOOR) / -FLOOR) * (h - 2 * pad);

  // Unity (y = x) reference diagonal.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(xFor(FLOOR), yFor(FLOOR));
  ctx.lineTo(xFor(0), yFor(0));
  ctx.stroke();

  // Threshold marker.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.5)";
  ctx.lineWidth = 1.2 * dpr;
  ctx.setLineDash([4 * dpr, 4 * dpr]);
  ctx.beginPath();
  ctx.moveTo(xFor(threshold), pad);
  ctx.lineTo(xFor(threshold), h - pad);
  ctx.stroke();
  ctx.setLineDash([]);

  // Transfer curve.
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const inDb = FLOOR + (px / w) * -FLOOR;
    const x = xFor(inDb);
    const y = yFor(transfer(inDb, threshold, range));
    if (px === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
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
  ctx.fillText(`thr ${fmt.db(threshold)} · range ${fmt.db(range)}`, w - 6 * dpr, 12 * dpr);
  ctx.textAlign = "left";
});

let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const threshold = -70 + tx * 70;
  const range = -ty * 80;
  params.set(P.threshold, threshold);
  params.set(P.range, range);
  sendSet(P.threshold, threshold);
  sendSet(P.range, range);
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
