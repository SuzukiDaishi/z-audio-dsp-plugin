// Z Audio Compressor UI — interactive transfer curve + grouped controls
// + live gain-reduction metering.
//
// The curve IS the compressor: drag horizontally to move the threshold,
// vertically to change the ratio, mouse-wheel to soften the knee. The
// static-curve math mirrors the DSP's soft-knee gain computer. A ZCGR
// meter packet (~30 Hz) drives the GR bar and the live dot on the curve.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  inputGain: 140,
  threshold: 141,
  ratio: 142,
  knee: 143,
  attack: 144,
  release: 145,
  makeup: 146,
  mix: 147,
  detector: 148,
  stereoLink: 149,
  scHpf: 980,
  lookahead: 981,
  autoRelease: 982,
  autoMakeup: 983,
  warmth: 984,
};

const SC_HPF_OFF = 20;

const fmtHpf = (v) => (v <= SC_HPF_OFF + 0.5 ? "Off" : fmt.hz(v));
const fmtLookahead = (v) => (v < 0.05 ? "Off" : `${v.toFixed(1)} ms`);

const PARAMS = [
  { id: P.threshold, label: "Threshold", kind: "slider", min: -60, max: 0, default: -18, step: 0.1, fmt: fmt.db, mount: "#sec-comp" },
  { id: P.ratio, label: "Ratio", kind: "slider", min: 1, max: 20, default: 4, step: 0.01, fmt: fmt.ratio, mount: "#sec-comp" },
  { id: P.knee, label: "Knee", kind: "slider", min: 0, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-comp" },
  { id: P.detector, label: "Detector", kind: "select", options: ["Peak", "RMS"], default: 0, mount: "#sec-detect" },
  { id: P.scHpf, label: "SC HPF", kind: "slider", min: 20, max: 500, default: 20, scale: "log", fmt: fmtHpf, mount: "#sec-detect" },
  { id: P.lookahead, label: "Lookahead", kind: "slider", min: 0, max: 10, default: 0, step: 0.1, fmt: fmtLookahead, mount: "#sec-detect" },
  { id: P.stereoLink, label: "Link", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-detect" },
  { id: P.attack, label: "Attack", kind: "slider", min: 0.1, max: 200, default: 10, scale: "log", fmt: fmt.ms, mount: "#sec-time" },
  { id: P.release, label: "Release", kind: "slider", min: 5, max: 2000, default: 120, scale: "log", fmt: fmt.ms, mount: "#sec-time" },
  { id: P.autoRelease, label: "Auto Release", kind: "toggle", default: 1, mount: "#sec-time" },
  { id: P.inputGain, label: "Input", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-level" },
  { id: P.makeup, label: "Makeup", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-level" },
  { id: P.autoMakeup, label: "Auto Makeup", kind: "toggle", default: 0, mount: "#sec-level" },
  { id: P.warmth, label: "Warmth", kind: "slider", min: 0, max: 1, default: 0.15, step: 0.01, fmt: fmt.pct, mount: "#sec-level" },
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-level" },
];

// Live meter state, fed by ZCGR packets from the plugin (wasm build only;
// the native webview simply never sends them and the overlay stays hidden).
const meter = { grDb: 0, inDb: -90, outDb: -90, alive: false };

function handleBinary(data) {
  if (!(data instanceof ArrayBuffer) || data.byteLength !== 16) return;
  const view = new DataView(data);
  if (
    view.getUint8(0) !== 0x5a || // 'Z'
    view.getUint8(1) !== 0x43 || // 'C'
    view.getUint8(2) !== 0x47 || // 'G'
    view.getUint8(3) !== 0x52 // 'R'
  ) {
    return;
  }
  meter.grDb = view.getFloat32(4, true);
  meter.inDb = view.getFloat32(8, true);
  meter.outDb = view.getFloat32(12, true);
  meter.alive = true;
  viz.redraw();
}

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
  onMessage: handleBinary,
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// ---------------------------------------------------------------------------
// Transfer curve.
// ---------------------------------------------------------------------------

const DB_LO = -60;
const DB_HI = 6;
const GR_METER_RANGE = 24;

/** Soft-knee gain computer output level (dB) for input level `x` (dB). */
function transfer(x, t, ratio, knee) {
  const over = x - t;
  if (knee > 0 && Math.abs(over) <= knee / 2) {
    const k = over + knee / 2;
    return x + ((1 / ratio - 1) * k * k) / (2 * knee);
  }
  return over > 0 ? t + over / ratio : x;
}

/** Total makeup shown on the curve — manual plus the DSP's auto term. */
function totalMakeup(t, ratio, knee, manual, autoOn) {
  if (!autoOn) return manual;
  return manual - 0.5 * (transfer(0, t, ratio, knee) - 0);
}

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const t = params.get(P.threshold);
  const ratio = params.get(P.ratio);
  const knee = params.get(P.knee);
  const makeup = totalMakeup(
    t,
    ratio,
    knee,
    params.get(P.makeup),
    params.get(P.autoMakeup) >= 0.5,
  );

  // Reserve a strip on the right for the GR meter.
  const meterW = 14 * dpr;
  const plotW = w - meterW - 6 * dpr;

  const xOf = (db) => ((db - DB_LO) / (DB_HI - DB_LO)) * plotW;
  const yOf = (db) => h - ((db - DB_LO) / (DB_HI - DB_LO)) * h;

  // Grid every 12 dB.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.12)";
  ctx.fillStyle = "rgba(126, 147, 163, 0.55)";
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.lineWidth = 1;
  for (let db = -60; db <= 0; db += 12) {
    ctx.beginPath();
    ctx.moveTo(xOf(db), 0);
    ctx.lineTo(xOf(db), h);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(0, yOf(db));
    ctx.lineTo(plotW, yOf(db));
    ctx.stroke();
    ctx.fillText(`${db}`, xOf(db) + 3 * dpr, h - 4 * dpr);
  }

  // Unity line.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.35)";
  ctx.setLineDash([4 * dpr, 4 * dpr]);
  ctx.beginPath();
  ctx.moveTo(xOf(DB_LO), yOf(DB_LO));
  ctx.lineTo(xOf(DB_HI), yOf(DB_HI));
  ctx.stroke();
  ctx.setLineDash([]);

  // Knee region shading.
  if (knee > 0) {
    ctx.fillStyle = "rgba(240, 168, 72, 0.08)";
    ctx.fillRect(xOf(t - knee / 2), 0, xOf(t + knee / 2) - xOf(t - knee / 2), h);
  }

  // Threshold marker.
  ctx.strokeStyle = "rgba(240, 168, 72, 0.5)";
  ctx.beginPath();
  ctx.moveTo(xOf(t), 0);
  ctx.lineTo(xOf(t), h);
  ctx.stroke();

  // Gain-reduction fill between the curve and the unity line.
  ctx.beginPath();
  for (let px = 0; px <= plotW; px++) {
    const x = DB_LO + (px / plotW) * (DB_HI - DB_LO);
    const y = yOf(transfer(x, t, ratio, knee) + makeup);
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  for (let px = plotW; px >= 0; px--) {
    const x = DB_LO + (px / plotW) * (DB_HI - DB_LO);
    ctx.lineTo(px, yOf(x));
  }
  ctx.closePath();
  ctx.fillStyle = "rgba(240, 168, 72, 0.10)";
  ctx.fill();

  // Transfer curve (with makeup).
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.beginPath();
  for (let px = 0; px <= plotW; px++) {
    const x = DB_LO + (px / plotW) * (DB_HI - DB_LO);
    const y = yOf(transfer(x, t, ratio, knee) + makeup);
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2 * dpr;
  ctx.shadowColor = accent;
  ctx.shadowBlur = 6 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;

  // Knee point dot.
  ctx.beginPath();
  ctx.arc(xOf(t), yOf(transfer(t, t, ratio, knee) + makeup), 4 * dpr, 0, Math.PI * 2);
  ctx.fillStyle = accent;
  ctx.fill();

  // GR meter strip (fills top-down, like a VU pulled from unity).
  const meterX = w - meterW;
  ctx.fillStyle = "rgba(126, 147, 163, 0.12)";
  ctx.fillRect(meterX, 0, meterW, h);
  if (meter.alive) {
    const grFrac = clamp(meter.grDb / GR_METER_RANGE, 0, 1);
    ctx.fillStyle = "rgba(235, 110, 88, 0.85)";
    ctx.fillRect(meterX, 0, meterW, grFrac * h);
    ctx.fillStyle = "rgba(126, 147, 163, 0.75)";
    ctx.font = `${8 * dpr}px sans-serif`;
    ctx.fillText("GR", meterX + 1 * dpr, h - 4 * dpr);

    // Live dot: current input level travelling the transfer curve.
    const inDb = meter.inDb + params.get(P.inputGain);
    if (inDb > DB_LO) {
      const x = clamp(inDb, DB_LO, DB_HI);
      ctx.beginPath();
      ctx.arc(xOf(x), yOf(transfer(x, t, ratio, knee) + makeup), 5 * dpr, 0, Math.PI * 2);
      ctx.fillStyle = "rgba(98, 184, 158, 0.9)";
      ctx.fill();
    }

    // Numeric readouts, top-left.
    ctx.fillStyle = "rgba(126, 147, 163, 0.9)";
    ctx.font = `${10 * dpr}px sans-serif`;
    ctx.fillText(
      `IN ${meter.inDb.toFixed(1)}  GR ${meter.grDb.toFixed(1)}  OUT ${meter.outDb.toFixed(1)} dB`,
      6 * dpr,
      12 * dpr,
    );
  }
});

// Drag: horizontal = threshold, vertical = ratio. Wheel = knee.
let dragStart = null;

canvas.addEventListener("pointerdown", (e) => {
  dragStart = {
    x: e.clientX,
    y: e.clientY,
    threshold: params.get(P.threshold),
    ratio: params.get(P.ratio),
  };
  canvas.setPointerCapture(e.pointerId);
});

canvas.addEventListener("pointermove", (e) => {
  if (!dragStart) return;
  const rect = canvas.getBoundingClientRect();
  const dbPerPx = (DB_HI - DB_LO) / rect.width;
  const threshold = clamp(dragStart.threshold + (e.clientX - dragStart.x) * dbPerPx, -60, 0);
  // Downward drag = higher ratio (squashes the curve toward horizontal).
  const ratio = clamp(dragStart.ratio * Math.pow(1.02, e.clientY - dragStart.y), 1, 20);
  params.set(P.threshold, threshold);
  params.set(P.ratio, ratio);
  sendSet(P.threshold, threshold);
  sendSet(P.ratio, ratio);
  viz.redraw();
});

canvas.addEventListener("pointerup", () => {
  dragStart = null;
});

canvas.addEventListener(
  "wheel",
  (e) => {
    e.preventDefault();
    const knee = clamp(params.get(P.knee) + (e.deltaY < 0 ? 1 : -1), 0, 24);
    params.set(P.knee, knee);
    sendSet(P.knee, knee);
    viz.redraw();
  },
  { passive: false },
);
