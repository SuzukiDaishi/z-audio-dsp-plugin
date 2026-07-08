// Z Audio Delay UI — echo tap timeline.
//
// The canvas shows the predicted echo train on a 4 s timeline: L taps as
// bars above the midline, R taps below, each echo scaled by feedback^n.
// Drag horizontally to sweep Time L (log; drives both channels when Link
// is on), vertically to set the feedback.

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  timeL: 800,
  timeR: 801,
  link: 802,
  feedback: 803,
  pingpong: 804,
  dampLp: 805,
  dampHp: 806,
  mix: 807,
  output: 808,
};

const PARAMS = [
  { id: P.timeL, label: "Time L", kind: "slider", min: 1, max: 2000, default: 350, scale: "log", fmt: fmt.ms, mount: "#sec-time" },
  { id: P.timeR, label: "Time R", kind: "slider", min: 1, max: 2000, default: 350, scale: "log", fmt: fmt.ms, mount: "#sec-time" },
  { id: P.link, label: "Link", kind: "toggle", min: 0, max: 1, default: 1, mount: "#sec-time" },
  { id: P.pingpong, label: "Ping Pong", kind: "toggle", min: 0, max: 1, default: 0, mount: "#sec-time" },
  { id: P.feedback, label: "Feedback", kind: "slider", min: 0, max: 0.95, default: 0.4, step: 0.01, fmt: fmt.pct, mount: "#sec-feedback" },
  { id: P.dampLp, label: "Damp LP", kind: "slider", min: 500, max: 20000, default: 8000, scale: "log", fmt: fmt.hz, mount: "#sec-feedback" },
  { id: P.dampHp, label: "Damp HP", kind: "slider", min: 10, max: 2000, default: 60, scale: "log", fmt: fmt.hz, mount: "#sec-feedback" },
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 0.35, step: 0.01, fmt: fmt.pct, mount: "#sec-feedback" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-feedback" },
];

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
  },
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// Mirrors src/lib.rs DelayEngine::process(): with Link the left time drives
// both lines; normal mode echoes each channel at n*time with amplitude
// feedback^(n-1); ping-pong feeds the mono input into the L line and
// cross-feeds the taps, so echoes alternate L, R, L, R… at cumulative
// timeL/timeR steps.
function echoTaps(timeL, timeR, link, pingpong, feedback) {
  const tR = link ? timeL : timeR;
  const taps = []; // { t (ms), amp, channel: "L"|"R" }
  const maxTaps = 32;
  if (pingpong) {
    let t = 0;
    for (let n = 0; n < maxTaps; n++) {
      const left = n % 2 === 0;
      t += left ? timeL : tR;
      const amp = Math.pow(feedback, n);
      if (amp < 0.02 && n > 0) break;
      taps.push({ t, amp, channel: left ? "L" : "R" });
    }
  } else {
    for (const [time, channel] of [
      [timeL, "L"],
      [tR, "R"],
    ]) {
      for (let n = 1; n <= maxTaps; n++) {
        const amp = Math.pow(feedback, n - 1);
        if (amp < 0.02 && n > 1) break;
        taps.push({ t: n * time, amp, channel });
      }
    }
  }
  return taps;
}

const canvas = document.getElementById("viz");
const WINDOW_MS = 4000;

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const timeL = params.get(P.timeL);
  const timeR = params.get(P.timeR);
  const link = params.get(P.link) >= 0.5;
  const pingpong = params.get(P.pingpong) >= 0.5;
  const feedback = params.get(P.feedback);

  const midY = h / 2;
  const amp = h * 0.4;

  // Time grid every second.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.12)";
  ctx.lineWidth = 1;
  for (let s = 1; s < WINDOW_MS / 1000; s++) {
    const x = (s * 1000 / WINDOW_MS) * w;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
  }

  // Midline.
  ctx.strokeStyle = "rgba(126, 147, 163, 0.35)";
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(w, midY);
  ctx.stroke();

  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  ctx.lineWidth = 2 * dpr;
  ctx.shadowColor = accent;
  ctx.shadowBlur = 4 * dpr;
  for (const tap of echoTaps(timeL, timeR, link, pingpong, feedback)) {
    if (tap.t > WINDOW_MS) continue;
    const x = (tap.t / WINDOW_MS) * w;
    // L bars point up, R bars point down.
    const y = tap.channel === "L" ? midY - tap.amp * amp : midY + tap.amp * amp;
    ctx.strokeStyle = tap.channel === "L" ? accent : "rgba(126, 199, 255, 0.85)";
    ctx.beginPath();
    ctx.moveTo(x, midY);
    ctx.lineTo(x, y);
    ctx.stroke();
  }
  ctx.shadowBlur = 0;

  ctx.fillStyle = "rgba(126, 147, 163, 0.7)";
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.textAlign = "right";
  const timeText = link ? `time ${fmt.ms(timeL)} (linked)` : `L ${fmt.ms(timeL)} · R ${fmt.ms(timeR)}`;
  ctx.fillText(`${timeText} · fb ${fmt.pct(feedback)}${pingpong ? " · ping-pong" : ""}`, w - 6 * dpr, 12 * dpr);
  ctx.textAlign = "left";

  ctx.fillText("L ↑", 6 * dpr, 12 * dpr);
  ctx.fillText("R ↓", 6 * dpr, h - 6 * dpr);
});

const TIME_LO = Math.log(1);
const TIME_HI = Math.log(2000);
let dragging = false;

function applyDrag(e) {
  const rect = canvas.getBoundingClientRect();
  const tx = clamp((e.clientX - rect.left) / rect.width, 0, 1);
  const ty = clamp((e.clientY - rect.top) / rect.height, 0, 1);
  const timeL = Math.exp(TIME_LO + tx * (TIME_HI - TIME_LO));
  const feedback = (1 - ty) * 0.95;
  params.set(P.timeL, timeL);
  sendSet(P.timeL, timeL);
  if (params.get(P.link) >= 0.5) {
    params.set(P.timeR, timeL);
    sendSet(P.timeR, timeL);
  }
  params.set(P.feedback, feedback);
  sendSet(P.feedback, feedback);
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
