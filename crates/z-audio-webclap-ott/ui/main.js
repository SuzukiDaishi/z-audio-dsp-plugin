// Z Audio OTT UI — static transfer-curve view.
//
// The canvas mirrors src/lib.rs: input level (dBFS, x) vs output level
// (y) after the up/down squeeze toward the -30 dB target, honoring
// Depth / Upward / Downward. Band gains and crossovers are plain sliders.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  depth: 940,
  time: 941,
  inGain: 942,
  outGain: 943,
  lowGain: 944,
  midGain: 945,
  highGain: 946,
  upward: 947,
  downward: 948,
  xoverLow: 949,
  xoverHigh: 950,
};

const PARAMS = [
  { id: P.depth, label: "Depth", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-dynamics" },
  { id: P.time, label: "Time", kind: "slider", min: 0.1, max: 4, default: 1, step: 0.01, fmt: fmt.x, mount: "#sec-dynamics" },
  { id: P.upward, label: "Upward", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-dynamics" },
  { id: P.downward, label: "Downward", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-dynamics" },
  { id: P.inGain, label: "In Gain", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-dynamics" },
  { id: P.outGain, label: "Out Gain", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-dynamics" },
  { id: P.lowGain, label: "Low Gain", kind: "slider", min: -12, max: 12, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-bands" },
  { id: P.midGain, label: "Mid Gain", kind: "slider", min: -12, max: 12, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-bands" },
  { id: P.highGain, label: "High Gain", kind: "slider", min: -12, max: 12, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-bands" },
  { id: P.xoverLow, label: "Low X-Over", kind: "slider", min: 40, max: 400, default: 120, scale: "log", fmt: fmt.hz, mount: "#sec-bands" },
  { id: P.xoverHigh, label: "High X-Over", kind: "slider", min: 1000, max: 8000, default: 2500, scale: "log", fmt: fmt.hz, mount: "#sec-bands" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Mirror of the per-band gain law in src/lib.rs process().
const TARGET_DB = -30;
const DOWN_SLOPE = 1 - 1 / 4;
const UP_SLOPE = 1 - 1 / 2;
const MAX_GAIN_DB = 24;

function outputDb(inDb, depth, up, down) {
  let gain = 0;
  if (inDb > TARGET_DB) gain = -(inDb - TARGET_DB) * DOWN_SLOPE * depth * down;
  else gain = (TARGET_DB - inDb) * UP_SLOPE * depth * up;
  return inDb + clamp(gain, -MAX_GAIN_DB, MAX_GAIN_DB);
}

const DB_LO = -60;
const DB_HI = 0;

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  const toX = (db) => ((db - DB_LO) / (DB_HI - DB_LO)) * w;
  const toY = (db) => h - ((clamp(db, DB_LO, DB_HI) - DB_LO) / (DB_HI - DB_LO)) * h;

  // Unity diagonal + target marker.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(toX(DB_LO), toY(DB_LO));
  ctx.lineTo(toX(DB_HI), toY(DB_HI));
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(toX(TARGET_DB), 0);
  ctx.lineTo(toX(TARGET_DB), h);
  ctx.stroke();

  const depth = params.get(P.depth);
  const up = params.get(P.upward);
  const down = params.get(P.downward);
  ctx.beginPath();
  const steps = 120;
  for (let i = 0; i <= steps; i++) {
    const inDb = DB_LO + ((DB_HI - DB_LO) * i) / steps;
    const y = toY(outputDb(inDb, depth, up, down));
    if (i === 0) ctx.moveTo(toX(inDb), y);
    else ctx.lineTo(toX(inDb), y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = Math.max(1.6, h / 60);
  ctx.stroke();
});
