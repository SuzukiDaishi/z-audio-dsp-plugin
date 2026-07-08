// Z Audio EQ — Pro-Q-style 8-band parametric EQ UI.
//
// The big canvas is the whole instrument: real-time pre/post spectra
// (pushed by the plugin as "ZEQS" packets), the summed EQ curve, one
// colored dot per enabled band. Double-click adds a band, dragging a dot
// moves freq/gain, the wheel adjusts Q, double-clicking a dot removes it.
// The panel under the canvas edits the selected band (type, slope,
// placement, solo listen).
//
// All filter-response math here mirrors src/engine.rs exactly, so the
// drawn curve is what the DSP applies.

"use strict";

import { connect, fmt, clamp, setupCanvas, markConnected } from "./zui.js";

// --- Parameter ids (mirror src/params.rs) ---------------------------------

const P_OUTPUT = 700;
const BASE = 710;
const FIELDS = 8;
const BANDS = 8;
const F = { ENABLE: 0, TYPE: 1, FREQ: 2, GAIN: 3, Q: 4, SLOPE: 5, PLACE: 6, SOLO: 7 };

const TYPES = ["Bell", "Lo Shelf", "Hi Shelf", "Lo Cut", "Hi Cut", "Notch"];
const T = { BELL: 0, LOSHELF: 1, HISHELF: 2, LOCUT: 3, HICUT: 4, NOTCH: 5 };
const SLOPES = ["6", "12", "24", "48"];
const PLACES = ["Stereo", "Mid", "Side", "L", "R"];
const COLORS = [
  "#e0645c", "#e09c4a", "#e0d24a", "#7ed05c",
  "#4ac9e0", "#5c8ce0", "#a06ce0", "#e06cc0",
];
const DEFAULT_FREQS = [30, 80, 200, 500, 1200, 3000, 8000, 16000];

const FREQ_MIN = 10;
const FREQ_MAX = 24000;
const DB_RANGE = 30; // curve axis ±30 dB
const SPEC_TOP = 0; // spectrum axis 0 … -90 dB
const SPEC_BOTTOM = -90;

const id = (band, field) => BASE + band * FIELDS + field;
const $id = (x) => document.getElementById(x);
const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

// --- Value store -----------------------------------------------------------

let sendSet = () => {};
const values = new Map();

values.set(P_OUTPUT, 0);
for (let b = 0; b < BANDS; b++) {
  values.set(id(b, F.ENABLE), 0);
  values.set(id(b, F.TYPE), 0);
  values.set(id(b, F.FREQ), DEFAULT_FREQS[b]);
  values.set(id(b, F.GAIN), 0);
  values.set(id(b, F.Q), 0.71);
  values.set(id(b, F.SLOPE), 1);
  values.set(id(b, F.PLACE), 0);
  values.set(id(b, F.SOLO), 0);
}

const val = (i) => values.get(i) ?? 0;

/**
 * Writes a value and repaints. `structural` re-renders the chips and the
 * band panel too — never set it from a panel-slider input handler, or the
 * rebuild would tear the slider out from under the drag.
 */
function setParam(i, v, { silent = false, structural = false } = {}) {
  values.set(i, v);
  if (!silent) sendSet(i, v);
  if (structural) refreshAll();
  else refresh();
}

function band(bi) {
  return {
    enabled: val(id(bi, F.ENABLE)) >= 0.5,
    type: Math.round(val(id(bi, F.TYPE))),
    freq: val(id(bi, F.FREQ)),
    gain: val(id(bi, F.GAIN)),
    q: val(id(bi, F.Q)),
    slope: Math.round(val(id(bi, F.SLOPE))),
    place: Math.round(val(id(bi, F.PLACE))),
    solo: val(id(bi, F.SOLO)) >= 0.5,
  };
}

// --- Filter response (mirrors src/engine.rs) --------------------------------

const FS = () => spec.sampleRate || 48000;
const Q_24 = [0.5412, 1.3066];
const Q_48 = [0.5098, 0.6013, 0.9, 2.5629];

function angular(fs, f0) {
  return (2 * Math.PI * clamp(f0, 1, fs * 0.49)) / fs;
}

function norm(b0, b1, b2, a0, a1, a2) {
  return { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 };
}

function peaking(fs, f0, q, gainDb) {
  const A = Math.pow(10, gainDb / 40);
  const w0 = angular(fs, f0);
  const sw = Math.sin(w0), cw = Math.cos(w0);
  const al = sw / (2 * q);
  return norm(1 + al * A, -2 * cw, 1 - al * A, 1 + al / A, -2 * cw, 1 - al / A);
}

function lowShelf(fs, f0, q, gainDb) {
  const A = Math.pow(10, gainDb / 40);
  const w0 = angular(fs, f0);
  const sw = Math.sin(w0), cw = Math.cos(w0);
  const al = sw / (2 * q);
  const sq = 2 * Math.sqrt(A) * al;
  return norm(
    A * ((A + 1) - (A - 1) * cw + sq),
    2 * A * ((A - 1) - (A + 1) * cw),
    A * ((A + 1) - (A - 1) * cw - sq),
    (A + 1) + (A - 1) * cw + sq,
    -2 * ((A - 1) + (A + 1) * cw),
    (A + 1) + (A - 1) * cw - sq,
  );
}

function highShelf(fs, f0, q, gainDb) {
  const A = Math.pow(10, gainDb / 40);
  const w0 = angular(fs, f0);
  const sw = Math.sin(w0), cw = Math.cos(w0);
  const al = sw / (2 * q);
  const sq = 2 * Math.sqrt(A) * al;
  return norm(
    A * ((A + 1) + (A - 1) * cw + sq),
    -2 * A * ((A - 1) + (A + 1) * cw),
    A * ((A + 1) + (A - 1) * cw - sq),
    (A + 1) - (A - 1) * cw + sq,
    2 * ((A - 1) - (A + 1) * cw),
    (A + 1) - (A - 1) * cw - sq,
  );
}

function lowPass(fs, f0, q) {
  const w0 = angular(fs, f0);
  const sw = Math.sin(w0), cw = Math.cos(w0);
  const al = sw / (2 * q);
  return norm((1 - cw) / 2, 1 - cw, (1 - cw) / 2, 1 + al, -2 * cw, 1 - al);
}

function highPass(fs, f0, q) {
  const w0 = angular(fs, f0);
  const sw = Math.sin(w0), cw = Math.cos(w0);
  const al = sw / (2 * q);
  return norm((1 + cw) / 2, -(1 + cw), (1 + cw) / 2, 1 + al, -2 * cw, 1 - al);
}

function notch(fs, f0, q) {
  const w0 = angular(fs, f0);
  const sw = Math.sin(w0), cw = Math.cos(w0);
  const al = sw / (2 * q);
  return norm(1, -2 * cw, 1, 1 + al, -2 * cw, 1 - al);
}

function onePoleLow(fs, f0) {
  const p = Math.exp(-angular(fs, f0));
  return { b0: 1 - p, b1: 0, b2: 0, a1: -p, a2: 0 };
}

function onePoleHigh(fs, f0) {
  const p = Math.exp(-angular(fs, f0));
  const g = (1 + p) / 2;
  return { b0: g, b1: -g, b2: 0, a1: -p, a2: 0 };
}

function bandStages(b) {
  const fs = FS();
  const reso = Math.max(b.q / 0.7071, 0.14);
  const cut = (hp) => {
    const make = (q) => (hp ? highPass(fs, b.freq, q) : lowPass(fs, b.freq, q));
    if (b.slope === 0) return [hp ? onePoleHigh(fs, b.freq) : onePoleLow(fs, b.freq)];
    if (b.slope === 2) return [make(Q_24[0]), make(Q_24[1] * reso)];
    if (b.slope === 3) return [make(Q_48[0]), make(Q_48[1]), make(Q_48[2]), make(Q_48[3] * reso)];
    return [make(0.7071 * reso)];
  };
  switch (b.type) {
    case T.LOSHELF: return [lowShelf(fs, b.freq, Math.max(b.q, 0.3), b.gain)];
    case T.HISHELF: return [highShelf(fs, b.freq, Math.max(b.q, 0.3), b.gain)];
    case T.LOCUT: return cut(true);
    case T.HICUT: return cut(false);
    case T.NOTCH: return [notch(fs, b.freq, b.q)];
    default: return [peaking(fs, b.freq, b.q, b.gain)];
  }
}

function magnitude(c, fs, f) {
  const w = (2 * Math.PI * f) / fs;
  const cw = Math.cos(w), c2w = Math.cos(2 * w);
  const num =
    c.b0 * c.b0 + c.b1 * c.b1 + c.b2 * c.b2 +
    2 * (c.b0 * c.b1 + c.b1 * c.b2) * cw + 2 * c.b0 * c.b2 * c2w;
  const den =
    1 + c.a1 * c.a1 + c.a2 * c.a2 +
    2 * (c.a1 + c.a1 * c.a2) * cw + 2 * c.a2 * c2w;
  return Math.sqrt(Math.max(num, 0) / Math.max(den, 1e-12));
}

function bandResponseDb(b, f) {
  const fs = FS();
  let mag = 1;
  for (const stage of bandStages(b)) mag *= magnitude(stage, fs, f);
  return 20 * Math.log10(Math.max(mag, 1e-9));
}

// --- Spectrum state ---------------------------------------------------------

const spec = {
  pre: null, // Float32Array of dB per bin
  post: null,
  sampleRate: 48000,
  bins: 0,
  smoothPre: null, // per-pixel smoothed dB
  smoothPost: null,
};

function handleBinary(ab) {
  if (!(ab instanceof ArrayBuffer) || ab.byteLength < 12) return;
  const view = new DataView(ab);
  if (
    view.getUint8(0) !== 0x5a || view.getUint8(1) !== 0x45 ||
    view.getUint8(2) !== 0x51 || view.getUint8(3) !== 0x53
  ) {
    return; // not "ZEQS"
  }
  const kind = view.getUint8(4);
  const bins = view.getUint16(6, true);
  const rate = view.getFloat32(8, true);
  if (ab.byteLength < 12 + bins * 4) return;
  const data = new Float32Array(bins);
  for (let i = 0; i < bins; i++) data[i] = view.getFloat32(12 + i * 4, true);
  spec.bins = bins;
  spec.sampleRate = rate;
  if (kind === 0) spec.pre = data;
  else spec.post = data;
  refresh();
}

// --- Canvas -----------------------------------------------------------------

const canvas = $id("viz");
let selected = -1;

const LOG_LO = Math.log(FREQ_MIN);
const LOG_HI = Math.log(FREQ_MAX);
const xOfFreq = (f, w) => ((Math.log(clamp(f, FREQ_MIN, FREQ_MAX)) - LOG_LO) / (LOG_HI - LOG_LO)) * w;
const freqOfX = (x, w) => Math.exp(LOG_LO + (clamp(x, 0, w) / w) * (LOG_HI - LOG_LO));
const yOfDb = (db, h) => h / 2 - (db / DB_RANGE) * (h / 2);
const dbOfY = (y, h) => ((h / 2 - y) / (h / 2)) * DB_RANGE;
const yOfSpecDb = (db, h) => ((SPEC_TOP - clamp(db, SPEC_BOTTOM, SPEC_TOP)) / (SPEC_TOP - SPEC_BOTTOM)) * h;

function dotPosition(b, w, h) {
  const x = xOfFreq(b.freq, w);
  const usesGain = b.type === T.BELL || b.type === T.LOSHELF || b.type === T.HISHELF;
  const y = usesGain ? yOfDb(b.gain, h) : yOfDb(0, h);
  return [x, y];
}

function drawSpectrum(ctx, data, smoothKey, w, h, fill, line) {
  if (!data) return;
  const fs = spec.sampleRate;
  const pxCount = Math.max(2, Math.floor(w));
  if (!spec[smoothKey] || spec[smoothKey].length !== pxCount) {
    spec[smoothKey] = new Float32Array(pxCount).fill(SPEC_BOTTOM);
  }
  const smooth = spec[smoothKey];
  const binHz = fs / 2 / data.length;
  for (let px = 0; px < pxCount; px++) {
    const f0 = freqOfX(px, pxCount);
    const f1 = freqOfX(px + 1, pxCount);
    let lo = Math.max(1, Math.floor(f0 / binHz));
    let hi = Math.min(data.length - 1, Math.ceil(f1 / binHz));
    if (hi < lo) hi = lo;
    let peak = SPEC_BOTTOM;
    for (let i = lo; i <= hi; i++) if (data[i] > peak) peak = data[i];
    // fast attack, slow release
    smooth[px] += (peak - smooth[px]) * (peak > smooth[px] ? 0.55 : 0.16);
  }
  ctx.beginPath();
  ctx.moveTo(0, h);
  for (let px = 0; px < pxCount; px++) {
    ctx.lineTo((px / pxCount) * w, yOfSpecDb(smooth[px], h));
  }
  ctx.lineTo(w, h);
  ctx.closePath();
  ctx.fillStyle = fill;
  ctx.fill();
  if (line) {
    ctx.beginPath();
    for (let px = 0; px < pxCount; px++) {
      const x = (px / pxCount) * w;
      const y = yOfSpecDb(smooth[px], h);
      if (px === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = line;
    ctx.lineWidth = 1;
    ctx.stroke();
  }
}

function draw() {
  const ctx = canvas.getContext("2d");
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);
  const accent = css("--accent");

  // Frequency grid.
  ctx.font = `${9 * dpr}px sans-serif`;
  ctx.textAlign = "left";
  for (const f of [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]) {
    const x = xOfFreq(f, w);
    ctx.strokeStyle = "rgba(126, 147, 163, 0.10)";
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
    ctx.fillStyle = "rgba(126, 147, 163, 0.55)";
    ctx.fillText(f >= 1000 ? `${f / 1000}k` : `${f}`, x + 3 * dpr, h - 5 * dpr);
  }
  // dB grid.
  for (let db = -24; db <= 24; db += 6) {
    const y = yOfDb(db, h);
    ctx.strokeStyle = db === 0 ? "rgba(126, 147, 163, 0.32)" : "rgba(126, 147, 163, 0.10)";
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
    if (db !== 0) {
      ctx.fillStyle = "rgba(126, 147, 163, 0.45)";
      ctx.fillText(`${db > 0 ? "+" : ""}${db}`, 4 * dpr, y - 3 * dpr);
    }
  }

  // Spectra.
  if ($id("show-pre").checked) {
    drawSpectrum(ctx, spec.pre, "smoothPre", w, h, "rgba(126, 147, 163, 0.14)", null);
  }
  if ($id("show-post").checked) {
    drawSpectrum(ctx, spec.post, "smoothPost", w, h, "rgba(88, 176, 224, 0.15)", "rgba(88, 176, 224, 0.45)");
  }

  // Selected band's own curve.
  if (selected >= 0) {
    const b = band(selected);
    if (b.enabled) {
      ctx.beginPath();
      for (let px = 0; px <= w; px += 2) {
        const f = freqOfX(px, w);
        const y = yOfDb(clamp(bandResponseDb(b, f), -DB_RANGE, DB_RANGE), h);
        if (px === 0) ctx.moveTo(px, y);
        else ctx.lineTo(px, y);
      }
      ctx.strokeStyle = COLORS[selected];
      ctx.globalAlpha = 0.55;
      ctx.lineWidth = 1.4 * dpr;
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
  }

  // Summed curve.
  const bands = [];
  for (let i = 0; i < BANDS; i++) {
    const b = band(i);
    if (b.enabled) bands.push(b);
  }
  ctx.beginPath();
  for (let px = 0; px <= w; px += 2) {
    const f = freqOfX(px, w);
    let db = 0;
    for (const b of bands) db += bandResponseDb(b, f);
    const y = yOfDb(clamp(db, -DB_RANGE, DB_RANGE), h);
    if (px === 0) ctx.moveTo(px, y);
    else ctx.lineTo(px, y);
  }
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2.2 * dpr;
  ctx.shadowColor = accent;
  ctx.shadowBlur = 6 * dpr;
  ctx.stroke();
  ctx.shadowBlur = 0;

  // Band dots.
  for (let i = 0; i < BANDS; i++) {
    const b = band(i);
    if (!b.enabled) continue;
    const [x, y] = dotPosition(b, w, h);
    ctx.beginPath();
    ctx.arc(x, y, 7 * dpr, 0, Math.PI * 2);
    ctx.fillStyle = COLORS[i];
    ctx.globalAlpha = i === selected ? 1 : 0.8;
    ctx.fill();
    ctx.globalAlpha = 1;
    if (i === selected) {
      ctx.beginPath();
      ctx.arc(x, y, 10 * dpr, 0, Math.PI * 2);
      ctx.strokeStyle = COLORS[i];
      ctx.lineWidth = 1.5 * dpr;
      ctx.stroke();
    }
    if (b.solo) {
      ctx.fillStyle = "#0b0f14";
      ctx.font = `bold ${8 * dpr}px sans-serif`;
      ctx.textAlign = "center";
      ctx.fillText("S", x, y + 3 * dpr);
      ctx.textAlign = "left";
    }
    ctx.fillStyle = "#0b0f14";
    if (!b.solo) {
      ctx.font = `bold ${8 * dpr}px sans-serif`;
      ctx.textAlign = "center";
      ctx.fillText(String(i + 1), x, y + 3 * dpr);
      ctx.textAlign = "left";
    }
  }

  // Drag readout.
  if (drag && drag.band >= 0) {
    const b = band(drag.band);
    ctx.fillStyle = "rgba(238, 242, 240, 0.9)";
    ctx.font = `${10 * dpr}px sans-serif`;
    ctx.textAlign = "right";
    ctx.fillText(
      `${TYPES[b.type]} · ${fmt.hz(b.freq)} · ${b.gain >= 0 ? "+" : ""}${b.gain.toFixed(1)} dB · Q ${b.q.toFixed(2)}`,
      w - 8 * dpr,
      14 * dpr,
    );
    ctx.textAlign = "left";
  }
}

const view = setupCanvas(canvas, draw);
// The viz row is fluid: re-measure whenever the element itself resizes
// (setupCanvas only listens to window resize).
new ResizeObserver(() => view.resize()).observe(canvas);

let redrawQueued = false;
let panelQueued = false;
function refresh() {
  if (redrawQueued) return;
  redrawQueued = true;
  requestAnimationFrame(() => {
    const withPanel = panelQueued;
    redrawQueued = false;
    panelQueued = false;
    draw();
    if (withPanel) {
      renderChips();
      renderPanel();
    }
  });
}

function refreshAll() {
  panelQueued = true;
  refresh();
}

// --- Canvas interaction -----------------------------------------------------

let drag = null; // { band, moved }

function canvasPoint(e) {
  // Scale each axis independently: the buffer can briefly disagree with
  // the CSS box (e.g. right after the layout settles).
  const rect = canvas.getBoundingClientRect();
  return [
    ((e.clientX - rect.left) * canvas.width) / rect.width,
    ((e.clientY - rect.top) * canvas.height) / rect.height,
  ];
}

function hitBand(px, py) {
  const w = canvas.width;
  const h = canvas.height;
  const dpr = window.devicePixelRatio || 1;
  let best = -1;
  let bestDist = 14 * dpr;
  for (let i = 0; i < BANDS; i++) {
    const b = band(i);
    if (!b.enabled) continue;
    const [x, y] = dotPosition(b, w, h);
    const d = Math.hypot(px - x, py - y);
    if (d < bestDist) {
      bestDist = d;
      best = i;
    }
  }
  return best;
}

function firstFreeBand() {
  for (let i = 0; i < BANDS; i++) if (!band(i).enabled) return i;
  return -1;
}

canvas.addEventListener("pointerdown", (e) => {
  e.preventDefault();
  const [px, py] = canvasPoint(e);
  const hit = hitBand(px, py);
  if (hit >= 0) {
    selected = hit;
    drag = { band: hit, moved: false };
    canvas.setPointerCapture(e.pointerId);
  }
  refresh();
});

canvas.addEventListener("pointermove", (e) => {
  if (!drag) return;
  drag.moved = true;
  const [px, py] = canvasPoint(e);
  const w = canvas.width;
  const h = canvas.height;
  const b = band(drag.band);
  const freq = clamp(freqOfX(px, w), FREQ_MIN, FREQ_MAX);
  setParam(id(drag.band, F.FREQ), freq, { silent: true });
  sendSet(id(drag.band, F.FREQ), freq);
  if (b.type === T.BELL || b.type === T.LOSHELF || b.type === T.HISHELF) {
    const gain = clamp(dbOfY(py, h), -30, 30);
    setParam(id(drag.band, F.GAIN), gain, { silent: true });
    sendSet(id(drag.band, F.GAIN), gain);
  }
});

canvas.addEventListener("pointerup", (e) => {
  if (drag) canvas.releasePointerCapture(e.pointerId);
  drag = null;
  refreshAll();
});

canvas.addEventListener("dblclick", (e) => {
  const [px, py] = canvasPoint(e);
  const hit = hitBand(px, py);
  if (hit >= 0) {
    // Remove: disable and clear solo so it can't keep soloing silently.
    setParam(id(hit, F.ENABLE), 0, { structural: true });
    setParam(id(hit, F.SOLO), 0, { structural: true });
    if (selected === hit) selected = -1;
    refreshAll();
    return;
  }
  const free = firstFreeBand();
  if (free < 0) return;
  const w = canvas.width;
  const h = canvas.height;
  setParam(id(free, F.TYPE), T.BELL);
  setParam(id(free, F.FREQ), clamp(freqOfX(px, w), FREQ_MIN, FREQ_MAX));
  setParam(id(free, F.GAIN), clamp(dbOfY(py, h), -30, 30));
  setParam(id(free, F.ENABLE), 1, { structural: true });
  selected = free;
  refreshAll();
});

canvas.addEventListener("wheel", (e) => {
  if (selected < 0 || !band(selected).enabled) return;
  e.preventDefault();
  const q = val(id(selected, F.Q));
  const next = clamp(q * Math.pow(e.shiftKey ? 1.01 : 1.12, -Math.sign(e.deltaY)), 0.1, 30);
  setParam(id(selected, F.Q), next, { structural: true });
});

// --- Band chips --------------------------------------------------------------

function renderChips() {
  const mount = $id("band-chips");
  mount.replaceChildren();
  for (let i = 0; i < BANDS; i++) {
    const b = band(i);
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "band-chip";
    chip.style.setProperty("--band-color", COLORS[i]);
    chip.classList.toggle("enabled", b.enabled);
    chip.classList.toggle("selected", i === selected);
    chip.classList.toggle("soloed", b.enabled && b.solo);
    chip.textContent = String(i + 1);
    chip.title = b.enabled
      ? `${TYPES[b.type]} · ${fmt.hz(b.freq)}`
      : "Click to add this band";
    chip.addEventListener("click", () => {
      if (!b.enabled) setParam(id(i, F.ENABLE), 1, { structural: true });
      selected = i;
      refreshAll();
    });
    mount.append(chip);
  }
}

// --- Selected band panel -------------------------------------------------------

function segmented(labels, value, onPick) {
  const root = document.createElement("div");
  root.className = "segmented";
  labels.forEach((label, i) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = label;
    btn.classList.toggle("active", i === value);
    btn.addEventListener("click", () => onPick(i));
    root.append(btn);
  });
  return root;
}

function panelSlider(label, min, max, value, log, format, onInput) {
  const wrap = document.createElement("label");
  wrap.className = "panel-slider";
  const name = document.createElement("span");
  name.textContent = label;
  const input = document.createElement("input");
  input.type = "range";
  input.min = 0;
  input.max = 1000;
  input.step = 1;
  const toNorm = (v) =>
    log
      ? ((Math.log(v) - Math.log(min)) / (Math.log(max) - Math.log(min))) * 1000
      : ((v - min) / (max - min)) * 1000;
  const fromNorm = (n) =>
    log
      ? Math.exp(Math.log(min) + (n / 1000) * (Math.log(max) - Math.log(min)))
      : min + (n / 1000) * (max - min);
  input.value = toNorm(value);
  const out = document.createElement("output");
  out.textContent = format(value);
  input.addEventListener("input", () => {
    const v = fromNorm(Number(input.value));
    out.textContent = format(v);
    onInput(v);
  });
  wrap.append(name, input, out);
  return wrap;
}

function renderPanel() {
  const mount = $id("band-panel");
  mount.replaceChildren();
  if (selected < 0) {
    const hint = document.createElement("p");
    hint.className = "empty-state";
    hint.textContent = "Double-click the display (or click a numbered chip) to add a band.";
    mount.append(hint);
    return;
  }
  const bi = selected;
  const b = band(bi);

  const head = document.createElement("div");
  head.className = "panel-row";

  const onBtn = document.createElement("button");
  onBtn.type = "button";
  onBtn.className = `panel-toggle${b.enabled ? " on" : ""}`;
  onBtn.textContent = b.enabled ? "ON" : "OFF";
  onBtn.style.setProperty("--band-color", COLORS[bi]);
  onBtn.addEventListener("click", () =>
    setParam(id(bi, F.ENABLE), b.enabled ? 0 : 1, { structural: true }),
  );

  const solo = document.createElement("button");
  solo.type = "button";
  solo.className = `panel-toggle solo${b.solo ? " on" : ""}`;
  solo.textContent = "SOLO";
  solo.title = "Listen to just this band's region";
  solo.addEventListener("click", () =>
    setParam(id(bi, F.SOLO), b.solo ? 0 : 1, { structural: true }),
  );

  head.append(onBtn, solo);
  head.append(segmented(TYPES, b.type, (i) => setParam(id(bi, F.TYPE), i, { structural: true })));

  const rows = document.createElement("div");
  rows.className = "panel-row sliders";
  rows.append(
    panelSlider("Freq", FREQ_MIN, FREQ_MAX, b.freq, true, fmt.hz, (v) =>
      setParam(id(bi, F.FREQ), v),
    ),
    panelSlider("Gain", -30, 30, b.gain, false, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(1)} dB`, (v) =>
      setParam(id(bi, F.GAIN), v),
    ),
    panelSlider("Q", 0.1, 30, b.q, true, (v) => v.toFixed(2), (v) => setParam(id(bi, F.Q), v)),
  );

  const tail = document.createElement("div");
  tail.className = "panel-row";
  const slopeWrap = document.createElement("span");
  slopeWrap.className = "panel-group";
  slopeWrap.append("Slope ");
  slopeWrap.append(
    segmented(SLOPES, b.slope, (i) => setParam(id(bi, F.SLOPE), i, { structural: true })),
  );
  const placeWrap = document.createElement("span");
  placeWrap.className = "panel-group";
  placeWrap.append("Place ");
  placeWrap.append(
    segmented(PLACES, b.place, (i) => setParam(id(bi, F.PLACE), i, { structural: true })),
  );
  tail.append(slopeWrap, placeWrap);

  mount.append(head, rows, tail);
}

// --- Global output + analyzer toggles ---------------------------------------

const outputSlider = $id("output-gain");
const outputReadout = $id("output-readout");
outputSlider.addEventListener("input", () => {
  const v = Number(outputSlider.value);
  outputReadout.textContent = `${v >= 0 ? "+" : ""}${v.toFixed(1)} dB`;
  values.set(P_OUTPUT, v);
  sendSet(P_OUTPUT, v);
});
outputSlider.addEventListener("dblclick", () => {
  outputSlider.value = "0";
  outputReadout.textContent = "0.0 dB";
  values.set(P_OUTPUT, 0);
  sendSet(P_OUTPUT, 0);
});
$id("show-pre").addEventListener("change", refresh);
$id("show-post").addEventListener("change", refresh);

// --- Connect ------------------------------------------------------------------

function applySnapshot(map) {
  for (const [i, v] of map) {
    if (values.has(i)) values.set(i, v);
  }
  outputSlider.value = String(val(P_OUTPUT));
  outputReadout.textContent = `${val(P_OUTPUT) >= 0 ? "+" : ""}${val(P_OUTPUT).toFixed(1)} dB`;
  refreshAll();
}

sendSet = connect({
  onSnapshot(map) {
    markConnected();
    applySnapshot(map);
  },
  onMessage: handleBinary,
});

// Tiny read-only hook for automated UI tests.
window.__eq = {
  val,
  selected: () => selected,
  hasSpectrum: () => ({ pre: !!spec.pre, post: !!spec.post }),
};

refreshAll();
view.redraw();
