// Z Audio Vocoder UI — band-envelope meter + preview keyboard.
//
// The canvas shows the modulator band envelopes as live bars, with a
// dimmed outline of the formant-shifted mapping the carrier actually
// receives. The keyboard at the bottom plays carrier notes through the
// `ZVCN` webview packet (hardware MIDI routes through the host's note
// port instead).

"use strict";

import { connect, createParams, setupCanvas, markConnected, clamp, fmt } from "./zui.js";

const P = {
  bands: 960,
  wave: 961,
  pitch: 962,
  freeRun: 963,
  noise: 964,
  shift: 965,
  attack: 966,
  release: 967,
  mix: 968,
  output: 969,
};

const PARAMS = [
  { id: P.wave, label: "Wave", kind: "select", options: ["Saw", "Square", "Pulse"], default: 0, mount: "#sec-carrier" },
  { id: P.pitch, label: "Free Pitch", kind: "slider", min: 30, max: 1000, default: 110, scale: "log", fmt: fmt.hz, mount: "#sec-carrier" },
  { id: P.freeRun, label: "Free Run", kind: "toggle", default: 1, mount: "#sec-carrier" },
  { id: P.noise, label: "Noise", kind: "slider", min: 0, max: 1, default: 0.15, step: 0.01, fmt: fmt.pct, mount: "#sec-carrier" },
  { id: P.bands, label: "Bands", kind: "slider", min: 8, max: 32, default: 16, step: 1, fmt: fmt.int, mount: "#sec-vocoder" },
  { id: P.shift, label: "Formant", kind: "slider", min: -8, max: 8, default: 0, step: 0.1, fmt: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(1)} bd`, mount: "#sec-vocoder" },
  { id: P.attack, label: "Attack", kind: "slider", min: 0.1, max: 50, default: 5, scale: "log", fmt: fmt.ms, mount: "#sec-vocoder" },
  { id: P.release, label: "Release", kind: "slider", min: 1, max: 500, default: 80, scale: "log", fmt: fmt.ms, mount: "#sec-vocoder" },
  { id: P.mix, label: "Mix", kind: "slider", min: 0, max: 1, default: 1, step: 0.01, fmt: fmt.pct, mount: "#sec-output" },
  { id: P.output, label: "Output", kind: "slider", min: -24, max: 24, default: 0, step: 0.1, fmt: fmt.db, mount: "#sec-output" },
];

// --- Transport (binary packets ride beside zui's param traffic) -------------

const NATIVE = typeof window.sendToPlugin === "function";

function bytesToBase64(bytes) {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

function base64ToBytes(text) {
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function sendBinary(buffer) {
  if (NATIVE) {
    window.sendToPlugin({ type: "bin", data: bytesToBase64(new Uint8Array(buffer)) });
  } else {
    window.parent.postMessage(buffer, "*");
  }
}

function sendNote(on, key, velocity) {
  const buf = new ArrayBuffer(7);
  const b = new Uint8Array(buf);
  b[0] = 0x5a; // Z
  b[1] = 0x56; // V
  b[2] = 0x43; // C
  b[3] = 0x4e; // N
  b[4] = on ? 1 : 0;
  b[5] = key & 0x7f;
  b[6] = velocity & 0x7f;
  sendBinary(buf);
}

// --- Live meter state --------------------------------------------------------

const state = {
  voices: 0,
  env: new Float32Array(0),
};

function handleBinary(data) {
  let ab = data;
  if (NATIVE && data && data.type === "bin" && typeof data.data === "string") {
    ab = base64ToBytes(data.data).buffer;
  }
  if (!(ab instanceof ArrayBuffer) || ab.byteLength < 6) return;
  const view = new DataView(ab);
  const magic = String.fromCharCode(
    view.getUint8(0),
    view.getUint8(1),
    view.getUint8(2),
    view.getUint8(3),
  );
  if (magic !== "ZVCM") return;
  const voices = view.getUint8(4);
  const bands = view.getUint8(5);
  if (ab.byteLength < 6 + bands * 4) return;
  const env = new Float32Array(bands);
  for (let i = 0; i < bands; i++) env[i] = view.getFloat32(6 + i * 4, true);
  state.voices = voices;
  state.env = env;
  viz.redraw();
}

const sendSet = connect({
  onSnapshot: (snapshot) => {
    params.applySnapshot(snapshot);
    markConnected();
    viz.redraw();
  },
  onMessage: handleBinary,
});

const params = createParams(PARAMS, sendSet, () => viz.redraw(), ".panels");

// --- Band-envelope visualization ----------------------------------------------

const canvas = document.getElementById("viz");

const viz = setupCanvas(canvas, () => {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);

  const bands = state.env.length || Math.round(params.get(P.bands));
  const shift = params.get(P.shift);
  const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  const pad = 8 * dpr;
  const baseY = h - 14 * dpr;
  const slot = (w - pad * 2) / bands;
  const barW = Math.max(1, slot * 0.62);
  const amp = baseY - 16 * dpr;
  // sqrt scaling keeps quiet consonant bands visible.
  const height = (v) => Math.sqrt(clamp(v * 3, 0, 1)) * amp;

  ctx.strokeStyle = "rgba(126, 147, 163, 0.25)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(pad, baseY);
  ctx.lineTo(w - pad, baseY);
  ctx.stroke();

  // Raw modulator envelopes.
  ctx.fillStyle = accent;
  for (let k = 0; k < bands; k++) {
    const v = state.env[k] || 0;
    const x = pad + k * slot + (slot - barW) / 2;
    ctx.fillRect(x, baseY - height(v), barW, height(v));
  }

  // Formant-shifted mapping outline: the envelope carrier band k receives.
  if (Math.abs(shift) > 0.05 && state.env.length) {
    const envAt = (pos) => {
      if (pos <= -1 || pos >= bands) return 0;
      const i0 = Math.floor(pos);
      const frac = pos - i0;
      const a = i0 >= 0 && i0 < bands ? state.env[i0] : 0;
      const b = i0 + 1 >= 0 && i0 + 1 < bands ? state.env[i0 + 1] : 0;
      return a * (1 - frac) + b * frac;
    };
    ctx.strokeStyle = "rgba(238, 242, 240, 0.55)";
    ctx.lineWidth = 1.4 * dpr;
    ctx.beginPath();
    for (let k = 0; k < bands; k++) {
      const x = pad + (k + 0.5) * slot;
      const y = baseY - height(envAt(k - shift));
      if (k === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
  }

  ctx.fillStyle = "rgba(126, 147, 163, 0.7)";
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.textAlign = "left";
  ctx.fillText("80 Hz", pad, h - 4 * dpr);
  ctx.textAlign = "right";
  ctx.fillText("12 kHz", w - pad, h - 4 * dpr);
  const voiceText =
    state.voices > 0
      ? `${state.voices} voice${state.voices === 1 ? "" : "s"}`
      : params.get(P.freeRun) >= 0.5
        ? "free run"
        : "idle";
  ctx.fillText(voiceText, w - pad, 12 * dpr);
  ctx.textAlign = "left";
});

// --- Preview keyboard ---------------------------------------------------------

const KEY_LO = 36; // C2
const KEY_HI = 84; // C6
const BLACK = new Set([1, 3, 6, 8, 10]);

function buildKeyboard() {
  const keyboardEl = document.getElementById("keyboard");
  const whites = [];
  for (let key = KEY_LO; key <= KEY_HI; key++) {
    if (!BLACK.has(key % 12)) whites.push(key);
  }
  const whiteW = 100 / whites.length;

  let whiteIndex = 0;
  for (let key = KEY_LO; key <= KEY_HI; key++) {
    const el = document.createElement("div");
    const black = BLACK.has(key % 12);
    el.className = `key ${black ? "black" : "white"}`;
    el.dataset.key = key;
    if (black) {
      el.style.left = `${whiteIndex * whiteW - whiteW * 0.32}%`;
      el.style.width = `${whiteW * 0.64}%`;
    } else {
      if (key % 12 === 0) {
        el.classList.add("c-key");
        el.dataset.label = `C${Math.floor(key / 12) - 1}`;
      }
      whiteIndex += 1;
    }
    keyboardEl.append(el);
  }

  let heldKey = -1;

  const keyFromPoint = (x, y) => {
    const el = document.elementFromPoint(x, y);
    const keyEl = el && el.closest ? el.closest(".key") : null;
    return keyEl ? Number(keyEl.dataset.key) : -1;
  };

  const press = (key, velocity) => {
    if (key === heldKey || key < 0) return;
    release();
    heldKey = key;
    sendNote(true, key, velocity);
    const el = keyboardEl.querySelector(`[data-key="${key}"]`);
    if (el) el.classList.add("held");
  };

  const release = () => {
    if (heldKey < 0) return;
    sendNote(false, heldKey, 0);
    const el = keyboardEl.querySelector(`[data-key="${heldKey}"]`);
    if (el) el.classList.remove("held");
    heldKey = -1;
  };

  const velocityFromEvent = (e) => {
    const keyEl = e.target.closest ? e.target.closest(".key") : null;
    if (!keyEl) return 100;
    const rect = keyEl.getBoundingClientRect();
    return Math.round(clamp(45 + (82 * (e.clientY - rect.top)) / rect.height, 30, 127));
  };

  keyboardEl.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    keyboardEl.setPointerCapture(e.pointerId);
    press(keyFromPoint(e.clientX, e.clientY), velocityFromEvent(e));
  });
  keyboardEl.addEventListener("pointermove", (e) => {
    if (e.buttons & 1) press(keyFromPoint(e.clientX, e.clientY), velocityFromEvent(e));
  });
  keyboardEl.addEventListener("pointerup", release);
  keyboardEl.addEventListener("pointercancel", release);
  window.addEventListener("blur", release);
}

buildKeyboard();
