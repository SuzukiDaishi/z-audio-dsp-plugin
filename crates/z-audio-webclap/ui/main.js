// Z Audio Simple Synth UI — three live scopes (oscillator shape, amp
// envelope, LFO) that redraw from the control values, over four grouped
// control sections. What you see is what the DSP is set to produce.

"use strict";

import { connect, createParams, setupCanvas, markConnected, fmt } from "./zui.js";

const P = {
  master: 0,
  shape: 2,
  level: 10,
  pulseWidth: 11,
  attack: 20,
  decay: 21,
  sustain: 22,
  release: 23,
  curve: 24,
  lfoWave: 31,
  lfoRate: 32,
  lfoDepth: 33,
  lfoRoute: 34,
};

const PARAMS = [
  { id: P.shape, label: "Shape", kind: "select", options: ["Sin", "Tri", "Saw", "Pls", "Nse"], default: 0, mount: "#sec-osc" },
  { id: P.level, label: "Level", kind: "slider", min: 0, max: 2, default: 1, step: 0.001, fmt: fmt.x, mount: "#sec-osc" },
  { id: P.pulseWidth, label: "Pulse W", kind: "slider", min: 0.05, max: 0.95, default: 0.5, step: 0.001, fmt: fmt.pct, mount: "#sec-osc" },
  { id: P.attack, label: "Attack", kind: "slider", min: 0.001, max: 10, default: 0.01, scale: "log", fmt: fmt.s, mount: "#sec-env" },
  { id: P.decay, label: "Decay", kind: "slider", min: 0.001, max: 10, default: 0.1, scale: "log", fmt: fmt.s, mount: "#sec-env" },
  { id: P.sustain, label: "Sustain", kind: "slider", min: 0, max: 1, default: 0.7, step: 0.001, fmt: fmt.pct, mount: "#sec-env" },
  { id: P.release, label: "Release", kind: "slider", min: 0.001, max: 10, default: 0.2, scale: "log", fmt: fmt.s, mount: "#sec-env" },
  { id: P.curve, label: "Curve", kind: "select", options: ["Linear", "Expo"], default: 1, mount: "#sec-env" },
  { id: P.lfoWave, label: "Wave", kind: "select", options: ["Sin", "Tri", "Up", "Dn", "Sq", "Rnd"], default: 0, mount: "#sec-lfo" },
  { id: P.lfoRate, label: "Rate", kind: "slider", min: 0.01, max: 20, default: 5, scale: "log", fmt: fmt.hzLfo, mount: "#sec-lfo" },
  { id: P.lfoDepth, label: "Depth", kind: "slider", min: 0, max: 12, default: 0, step: 0.001, fmt: fmt.plain, mount: "#sec-lfo" },
  { id: P.lfoRoute, label: "Route", kind: "select", options: [{ value: 0, label: "None" }, { value: 1, label: "Gain" }, { value: 2, label: "Pitch" }], default: 0, mount: "#sec-lfo" },
  { id: P.master, label: "Master", kind: "slider", min: 0, max: 2, default: 1, step: 0.001, fmt: fmt.x, mount: "#sec-out" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
    redrawAll();
  },
});

const params = createParams(PARAMS, sendSet, () => redrawAll(), ".panels");

const accent = () =>
  getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();

// Deterministic noise for the noise/random shapes (stable redraws).
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

function oscSample(shape, phase, pw, rand) {
  switch (shape) {
    case 1:
      return phase < 0.5 ? phase * 4 - 1 : 3 - phase * 4;
    case 2:
      return phase * 2 - 1;
    case 3:
      return phase < pw ? 1 : -1;
    case 4:
      return rand() * 2 - 1;
    default:
      return Math.sin(phase * Math.PI * 2);
  }
}

function lfoSample(wave, phase, rand) {
  switch (wave) {
    case 1:
      return phase < 0.5 ? phase * 4 - 1 : 3 - phase * 4;
    case 2:
      return phase * 2 - 1;
    case 3:
      return 1 - phase * 2;
    case 4:
      return phase < 0.5 ? 1 : -1;
    case 5:
      return rand() * 2 - 1;
    default:
      return Math.sin(phase * Math.PI * 2);
  }
}

function strokeScope(canvas, sampleAt) {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);
  ctx.strokeStyle = "rgba(126, 147, 163, 0.2)";
  ctx.beginPath();
  ctx.moveTo(0, h / 2);
  ctx.lineTo(w, h / 2);
  ctx.stroke();
  ctx.beginPath();
  for (let px = 0; px <= w; px++) {
    const y = h / 2 - sampleAt(px / w) * (h / 2 - 8 * dpr);
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent();
  ctx.lineWidth = 1.6 * dpr;
  ctx.shadowColor = accent();
  ctx.shadowBlur = 5 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;
  ctx.lineWidth = 1;
}

// Oscillator scope: two cycles, scaled by level, tremolo ghost if the LFO
// routes to gain.
const oscCanvas = document.getElementById("viz-osc");
const oscViz = setupCanvas(oscCanvas, () => {
  const shape = Math.round(params.get(P.shape));
  const pw = params.get(P.pulseWidth);
  const level = params.get(P.level) / 2;
  const rand = mulberry(3);
  strokeScope(oscCanvas, (t) => oscSample(shape, (t * 2) % 1, pw, rand) * level);
});

// Envelope scope: A-D-S plateau-R with the selected curve.
const envCanvas = document.getElementById("viz-env");
const envViz = setupCanvas(envCanvas, () => {
  const a = params.get(P.attack);
  const d = params.get(P.decay);
  const s = params.get(P.sustain);
  const r = params.get(P.release);
  const expo = Math.round(params.get(P.curve)) === 1;
  const hold = Math.max(0.08, (a + d + r) * 0.25);
  const total = a + d + hold + r;
  const shape = (t01) => {
    const t = t01 * total;
    let v;
    if (t < a) v = t / Math.max(a, 1e-4);
    else if (t < a + d) {
      const k = (t - a) / Math.max(d, 1e-4);
      v = 1 - (1 - s) * (expo ? 1 - Math.pow(1 - k, 2.5) : k);
    } else if (t < a + d + hold) v = s;
    else {
      const k = (t - a - d - hold) / Math.max(r, 1e-4);
      v = s * (expo ? Math.pow(1 - Math.min(k, 1), 2.5) : 1 - Math.min(k, 1));
    }
    return v * 1.8 - 0.9; // map 0..1 to scope range
  };
  strokeScope(envCanvas, shape);
});

// LFO scope: cycles scale with rate (so faster literally looks faster),
// amplitude with depth.
const lfoCanvas = document.getElementById("viz-lfo");
const lfoViz = setupCanvas(lfoCanvas, () => {
  const wave = Math.round(params.get(P.lfoWave));
  const rate = params.get(P.lfoRate);
  const depth = Math.min(1, params.get(P.lfoDepth) / 12);
  const cycles = 1 + Math.min(7, Math.log2(1 + rate));
  const rand = mulberry(11);
  let held = rand() * 2 - 1;
  let lastStep = -1;
  strokeScope(lfoCanvas, (t) => {
    const phase = (t * cycles) % 1;
    if (wave === 5) {
      const step = Math.floor(t * cycles * 4);
      if (step !== lastStep) {
        lastStep = step;
        held = rand() * 2 - 1;
      }
      return held * Math.max(depth, 0.05);
    }
    return lfoSample(wave, phase, rand) * Math.max(depth, 0.05);
  });
});

function redrawAll() {
  oscViz.redraw();
  envViz.redraw();
  lfoViz.redraw();
}
