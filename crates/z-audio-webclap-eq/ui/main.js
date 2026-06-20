const SAMPLE_RATE = 48000;
const GRAPH = {
  width: 900,
  height: 304,
  left: 54,
  right: 18,
  top: 18,
  bottom: 34,
};
const DB_MIN = -24;
const DB_MAX = 24;
const FREQ_MIN = 20;
const FREQ_MAX = 20000;

const TYPE_LABELS = ["Low Pass", "Band Pass", "High Pass"];

const BANDS = [
  {
    key: "low",
    title: "Low",
    color: "#31d1a0",
    enabled: { id: 40, label: "In", kind: "toggle", default: 0 },
    freq: {
      id: 41,
      label: "Freq",
      kind: "range",
      scale: "log",
      min: 20,
      max: 2000,
      step: 0.0001,
      default: 200,
    },
    type: { id: 42, label: "Type", kind: "select", options: TYPE_LABELS, default: 0 },
    gain: {
      id: 49,
      label: "Gain",
      kind: "range",
      min: -24,
      max: 24,
      step: 0.1,
      default: 0,
      unit: "dB",
    },
    q: { id: 50, label: "Q", kind: "range", min: 0.1, max: 10, step: 0.01, default: 0.707 },
  },
  {
    key: "mid",
    title: "Mid",
    color: "#8b7cf6",
    enabled: { id: 43, label: "In", kind: "toggle", default: 0 },
    freq: {
      id: 44,
      label: "Freq",
      kind: "range",
      scale: "log",
      min: 80,
      max: 8000,
      step: 0.0001,
      default: 1000,
    },
    type: { id: 45, label: "Type", kind: "select", options: TYPE_LABELS, default: 1 },
    gain: {
      id: 51,
      label: "Gain",
      kind: "range",
      min: -24,
      max: 24,
      step: 0.1,
      default: 0,
      unit: "dB",
    },
    q: { id: 52, label: "Q", kind: "range", min: 0.1, max: 10, step: 0.01, default: 0.707 },
  },
  {
    key: "high",
    title: "High",
    color: "#f5b64c",
    enabled: { id: 46, label: "In", kind: "toggle", default: 0 },
    freq: {
      id: 47,
      label: "Freq",
      kind: "range",
      scale: "log",
      min: 1000,
      max: 20000,
      step: 0.0001,
      default: 5000,
    },
    type: { id: 48, label: "Type", kind: "select", options: TYPE_LABELS, default: 2 },
    gain: {
      id: 53,
      label: "Gain",
      kind: "range",
      min: -24,
      max: 24,
      step: 0.1,
      default: 0,
      unit: "dB",
    },
    q: { id: 54, label: "Q", kind: "range", min: 0.1, max: 10, step: 0.01, default: 0.707 },
  },
];

const controls = new Map();
const bandState = new Map(
  BANDS.map((band) => [
    band.key,
    {
      enabled: band.enabled.default,
      freq: band.freq.default,
      type: band.type.default,
      gain: band.gain.default,
      q: band.q.default,
    },
  ]),
);
const status = document.querySelector("#status");
const selectedBand = { key: "mid" };
let graphQueued = false;

function encodeReady() {
  return new Uint8Array([0x65, 0x72, 0x65, 0x61, 0x64, 0x79]).buffer;
}

function encodeSet(id, value) {
  const buf = new ArrayBuffer(20);
  const view = new DataView(buf);
  view.setUint8(0, 0xa1);
  view.setUint8(1, 0x63);
  view.setUint8(2, 0x73);
  view.setUint8(3, 0x65);
  view.setUint8(4, 0x74);
  view.setUint8(5, 0x82);
  view.setUint8(6, 0x1a);
  view.setUint32(7, id, false);
  view.setUint8(11, 0xfb);
  view.setFloat64(12, value, false);
  return buf;
}

function decodeParamsSnapshot(ab) {
  const view = new DataView(ab);
  let p = 0;
  if (view.byteLength < 9 || view.getUint8(p++) !== 0xa1 || view.getUint8(p++) !== 0x66) {
    return null;
  }
  if (String.fromCharCode(...new Uint8Array(ab, p, 6)) !== "params") return null;
  p += 6;

  const head = view.getUint8(p++);
  if ((head & 0xe0) !== 0xa0) return null;
  let count = head & 0x1f;
  if (count === 24) count = view.getUint8(p++);
  if (count > 255) return null;

  const out = new Map();
  for (let i = 0; i < count; i++) {
    if (p + 13 > view.byteLength || view.getUint8(p++) !== 0x1a) return null;
    const key = view.getUint32(p, false);
    p += 4;
    if (view.getUint8(p++) !== 0xfb) return null;
    const val = view.getFloat64(p, false);
    p += 8;
    out.set(key, val);
  }
  return out;
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function clamp01(value) {
  return clamp(value, 0, 1);
}

function logNorm(value, min, max) {
  return clamp01((Math.log10(value) - Math.log10(min)) / (Math.log10(max) - Math.log10(min)));
}

function fromLogNorm(norm, min, max) {
  const logMin = Math.log10(min);
  const logMax = Math.log10(max);
  return 10 ** (logMin + clamp01(norm) * (logMax - logMin));
}

function valueToInput(def, value) {
  if (def.scale === "log") return logNorm(value, def.min, def.max);
  return value;
}

function inputToValue(def, value) {
  if (def.scale === "log") return fromLogNorm(value, def.min, def.max);
  return Number(value);
}

function stateFor(band) {
  return bandState.get(band.key);
}

function setBandValue(band, role, value) {
  const state = stateFor(band);
  if (!state) return;
  state[role] = value;
}

function findBandByParam(id) {
  for (const band of BANDS) {
    for (const role of ["enabled", "freq", "type", "gain", "q"]) {
      if (band[role].id === id) return { band, role, def: band[role] };
    }
  }
  return null;
}

function formatFreq(value) {
  if (value >= 1000) {
    const decimals = value >= 10000 ? 1 : 2;
    return `${(value / 1000).toFixed(decimals)} kHz`;
  }
  return `${Math.round(value)} Hz`;
}

function formatValue(def, value) {
  if (def.kind === "toggle") return value >= 0.5 ? "IN" : "OFF";
  if (def.kind === "select") return def.options[Math.round(value)] ?? def.options[0];
  if (def.label === "Freq") return formatFreq(value);
  if (def.label === "Gain") return `${value >= 0 ? "+" : ""}${value.toFixed(1)} dB`;
  if (def.label === "Q") return value.toFixed(2);
  return Number(value).toFixed(2);
}

function sendSet(id, value) {
  window.parent.postMessage(encodeSet(id, value), "*");
}

function autoEnableBand(band) {
  const state = stateFor(band);
  if (!state || state.enabled >= 0.5) return;
  state.enabled = 1;
  const control = controls.get(band.enabled.id);
  control?.paint(1, { updateState: false });
  sendSet(band.enabled.id, 1);
}

function queueGraphUpdate() {
  if (graphQueued) return;
  graphQueued = true;
  requestAnimationFrame(() => {
    graphQueued = false;
    paintGraph();
    paintBandSelection();
  });
}

function createControl(band, role) {
  const def = band[role];
  const wrap = document.createElement("label");
  wrap.className = `control control-${def.kind}`;
  wrap.dataset.role = role;

  const title = document.createElement("span");
  title.className = "control-label";
  title.textContent = def.label;

  const readout = document.createElement("span");
  readout.className = "readout";

  let input;
  if (def.kind === "toggle") {
    input = document.createElement("input");
    input.type = "checkbox";
    input.checked = def.default >= 0.5;
    const switchBody = document.createElement("span");
    switchBody.className = "switch-body";
    switchBody.append(input, document.createElement("span"));
    wrap.append(title, switchBody, readout);
  } else if (def.kind === "select") {
    input = document.createElement("select");
    input.autocomplete = "off";
    def.options.forEach((name, index) => {
      const option = document.createElement("option");
      option.value = index;
      option.textContent = name;
      input.append(option);
    });
    wrap.append(title, input, readout);
  } else {
    input = document.createElement("input");
    input.type = "range";
    input.autocomplete = "off";
    if (def.scale === "log") {
      input.min = 0;
      input.max = 1;
      input.step = def.step;
    } else {
      input.min = def.min;
      input.max = def.max;
      input.step = def.step;
    }
    const rail = document.createElement("span");
    rail.className = "slider-rail";
    rail.append(input);
    wrap.append(title, rail, readout);
  }

  function read() {
    if (def.kind === "toggle") return input.checked ? 1 : 0;
    if (def.kind === "select") return Number(input.value);
    return inputToValue(def, Number(input.value));
  }

  function paint(next, options = {}) {
    const value = def.kind === "select" ? Math.round(next) : Number(next);
    if (def.kind === "toggle") {
      input.checked = value >= 0.5;
    } else if (def.kind === "range") {
      const inputValue = valueToInput(def, value);
      input.value = String(inputValue);
      const norm =
        def.scale === "log" ? inputValue : (value - def.min) / (def.max - def.min);
      input.style.setProperty("--value", `${clamp01(norm) * 100}%`);
    } else {
      input.value = String(value);
    }

    readout.textContent = formatValue(def, value);
    if (options.updateState !== false) setBandValue(band, role, value);
    queueGraphUpdate();
  }

  const eventName = def.kind === "range" ? "input" : "change";
  input.addEventListener(eventName, () => {
    selectedBand.key = band.key;
    const next = read();
    if (role !== "enabled") autoEnableBand(band);
    paint(next);
    sendSet(def.id, next);
  });

  paint(def.default);
  controls.set(def.id, { def, band, role, paint });
  return wrap;
}

function buildBandControls() {
  const root = document.querySelector("#controls");
  for (const band of BANDS) {
    const section = document.createElement("section");
    section.className = `band-card band-${band.key}`;
    section.dataset.band = band.key;
    section.style.setProperty("--band-color", band.color);

    const header = document.createElement("button");
    header.className = "band-header";
    header.type = "button";
    header.innerHTML = `<span>${band.title}</span><small></small>`;
    header.addEventListener("click", () => {
      selectedBand.key = band.key;
      paintBandSelection();
    });

    const fields = document.createElement("div");
    fields.className = "band-fields";
    fields.append(
      createControl(band, "enabled"),
      createControl(band, "type"),
      createControl(band, "freq"),
      createControl(band, "gain"),
      createControl(band, "q"),
    );

    section.append(header, fields);
    root.append(section);
  }
}

function xForFreq(freq) {
  const span = GRAPH.width - GRAPH.left - GRAPH.right;
  return GRAPH.left + logNorm(freq, FREQ_MIN, FREQ_MAX) * span;
}

function yForDb(db) {
  const span = GRAPH.height - GRAPH.top - GRAPH.bottom;
  return GRAPH.top + ((DB_MAX - clamp(db, DB_MIN, DB_MAX)) / (DB_MAX - DB_MIN)) * span;
}

function graphPath(points) {
  return points
    .map((point, index) => `${index === 0 ? "M" : "L"}${point.x.toFixed(1)} ${point.y.toFixed(1)}`)
    .join(" ");
}

function bandFillPath(points) {
  const zero = yForDb(0);
  if (!points.length) return "";
  const last = points[points.length - 1];
  const first = points[0];
  return `${graphPath(points)} L${last.x.toFixed(1)} ${zero.toFixed(1)} L${first.x.toFixed(
    1,
  )} ${zero.toFixed(1)} Z`;
}

function biquadCoefficients(type, freq, q) {
  const w0 = (2 * Math.PI * freq) / SAMPLE_RATE;
  const cos = Math.cos(w0);
  const alpha = Math.sin(w0) / (2 * q);
  let b0;
  let b1;
  let b2;

  if (type === 0) {
    b0 = (1 - cos) / 2;
    b1 = 1 - cos;
    b2 = (1 - cos) / 2;
  } else if (type === 1) {
    b0 = alpha;
    b1 = 0;
    b2 = -alpha;
  } else {
    b0 = (1 + cos) / 2;
    b1 = -(1 + cos);
    b2 = (1 + cos) / 2;
  }

  const a0 = 1 + alpha;
  return {
    b0: b0 / a0,
    b1: b1 / a0,
    b2: b2 / a0,
    a1: (-2 * cos) / a0,
    a2: (1 - alpha) / a0,
  };
}

function responseDbForState(state, freq) {
  if (!state || state.enabled < 0.5) return 0;
  const f0 = clamp(state.freq, FREQ_MIN, SAMPLE_RATE * 0.45);
  const q = clamp(state.q, 0.1, 10);
  const coeffs = biquadCoefficients(Math.round(state.type), f0, q);
  const w = (2 * Math.PI * freq) / SAMPLE_RATE;
  const c1 = Math.cos(-w);
  const s1 = Math.sin(-w);
  const c2 = Math.cos(-2 * w);
  const s2 = Math.sin(-2 * w);
  const nr = coeffs.b0 + coeffs.b1 * c1 + coeffs.b2 * c2;
  const ni = coeffs.b1 * s1 + coeffs.b2 * s2;
  const dr = 1 + coeffs.a1 * c1 + coeffs.a2 * c2;
  const di = coeffs.a1 * s1 + coeffs.a2 * s2;
  const numerator = Math.sqrt(nr * nr + ni * ni);
  const denominator = Math.max(1e-9, Math.sqrt(dr * dr + di * di));
  return 20 * Math.log10(Math.max(1e-9, numerator / denominator)) + state.gain;
}

function drawGrid() {
  const grid = document.querySelector("#grid");
  if (!grid || grid.childElementCount) return;

  for (const db of [-24, -12, 0, 12, 24]) {
    const y = yForDb(db);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", GRAPH.left);
    line.setAttribute("x2", GRAPH.width - GRAPH.right);
    line.setAttribute("y1", y);
    line.setAttribute("y2", y);
    line.classList.add(db === 0 ? "zero-line" : "grid-line");
    grid.append(line);

    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.setAttribute("x", 12);
    label.setAttribute("y", y + 4);
    label.textContent = db > 0 ? `+${db}` : String(db);
    label.classList.add("axis-label");
    grid.append(label);
  }

  for (const freq of [20, 200, 2000, 20000]) {
    const x = xForFreq(freq);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", x);
    line.setAttribute("x2", x);
    line.setAttribute("y1", GRAPH.top);
    line.setAttribute("y2", GRAPH.height - GRAPH.bottom);
    line.classList.add("grid-line");
    grid.append(line);

    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.setAttribute("x", x);
    label.setAttribute("y", GRAPH.height - 10);
    label.textContent = formatFreq(freq).replace(".00 ", " ");
    label.classList.add("axis-label", "freq-label");
    grid.append(label);
  }
}

function drawNodes() {
  const root = document.querySelector("#nodes");
  if (!root || root.childElementCount) return;
  for (const band of BANDS) {
    const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
    group.classList.add("eq-node", `node-${band.key}`);
    group.dataset.band = band.key;

    const halo = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    halo.setAttribute("r", 14);
    halo.classList.add("node-halo");

    const dot = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    dot.setAttribute("r", 8);
    dot.classList.add("node-dot");

    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.setAttribute("y", 4);
    label.textContent = band.title[0];
    label.classList.add("node-label");

    group.append(halo, dot, label);
    group.addEventListener("click", () => {
      selectedBand.key = band.key;
      paintBandSelection();
    });
    root.append(group);
  }
}

function responsePointsForBand(band) {
  const state = stateFor(band);
  const points = [];
  for (let i = 0; i <= 168; i++) {
    const norm = i / 168;
    const freq = fromLogNorm(norm, FREQ_MIN, FREQ_MAX);
    points.push({ x: xForFreq(freq), y: yForDb(responseDbForState(state, freq)) });
  }
  return points;
}

function compositePoints() {
  const points = [];
  for (let i = 0; i <= 220; i++) {
    const norm = i / 220;
    const freq = fromLogNorm(norm, FREQ_MIN, FREQ_MAX);
    const db = BANDS.reduce((sum, band) => sum + responseDbForState(stateFor(band), freq), 0);
    points.push({ x: xForFreq(freq), y: yForDb(db) });
  }
  return points;
}

function paintGraph() {
  drawGrid();
  drawNodes();

  for (const band of BANDS) {
    const points = responsePointsForBand(band);
    const state = stateFor(band);
    const fill = document.querySelector(`#${band.key}-fill`);
    const line = document.querySelector(`#${band.key}-line`);
    fill?.setAttribute("d", bandFillPath(points));
    line?.setAttribute("d", graphPath(points));
    fill?.classList.toggle("is-off", state.enabled < 0.5);
    line?.classList.toggle("is-off", state.enabled < 0.5);
    document.querySelector(`.band-${band.key}`)?.classList.toggle("is-off", state.enabled < 0.5);
  }

  document.querySelector("#composite-line")?.setAttribute("d", graphPath(compositePoints()));

  for (const band of BANDS) {
    const state = stateFor(band);
    const node = document.querySelector(`.node-${band.key}`);
    if (!node || !state) continue;
    const x = xForFreq(state.freq);
    const y = yForDb(state.enabled >= 0.5 ? state.gain : 0);
    node.setAttribute("transform", `translate(${x.toFixed(1)} ${y.toFixed(1)})`);
    node.classList.toggle("is-off", state.enabled < 0.5);
  }
}

function paintBandSelection() {
  for (const band of BANDS) {
    const selected = band.key === selectedBand.key;
    document.querySelector(`.band-${band.key}`)?.classList.toggle("is-selected", selected);
    document.querySelector(`.node-${band.key}`)?.classList.toggle("is-selected", selected);
    const card = document.querySelector(`.band-${band.key}`);
    const state = stateFor(band);
    const summary = card?.querySelector(".band-header small");
    if (summary && state) {
      summary.textContent = `${formatFreq(state.freq)} ${state.gain >= 0 ? "+" : ""}${state.gain.toFixed(
        1,
      )} dB`;
    }
  }
}

function applySnapshot(snapshot) {
  for (const [id, next] of snapshot) {
    const control = controls.get(id);
    if (!control) continue;
    control.paint(next);
  }
  status.textContent = "CONNECTED";
}

window.addEventListener("message", (event) => {
  if (!(event.data instanceof ArrayBuffer)) return;
  const snapshot = decodeParamsSnapshot(event.data);
  if (snapshot) applySnapshot(snapshot);
});

buildBandControls();
paintGraph();
paintBandSelection();
window.parent.postMessage(encodeReady(), "*");
