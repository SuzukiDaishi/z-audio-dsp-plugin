const GRAPH = {
  width: 760,
  height: 250,
  left: 52,
  right: 22,
  top: 18,
  bottom: 34,
};
const DB_MIN = -60;
const DB_MAX = 12;

const GROUPS = [
  {
    title: "Level",
    params: [
      { id: 120, key: "inputGain", label: "Input", min: -24, max: 24, default: 0, step: 0.1, unit: "dB" },
      { id: 121, key: "threshold", label: "Threshold", min: -24, max: 0, default: -0.1, step: 0.1, unit: "dB" },
      { id: 122, key: "ceiling", label: "Ceiling", min: -24, max: 0, default: -0.1, step: 0.1, unit: "dB" },
      { id: 127, key: "outputGain", label: "Output", min: -24, max: 24, default: 0, step: 0.1, unit: "dB" },
    ],
  },
  {
    title: "Timing",
    params: [
      { id: 124, key: "lookahead", label: "Lookahead", min: 0, max: 10, default: 3, step: 0.01, unit: "ms" },
      { id: 123, key: "release", label: "Release", min: 1, max: 1000, default: 80, step: 0.0001, scale: "log", unit: "ms" },
      { id: 125, key: "stereoLink", label: "Link", min: 0, max: 1, default: 1, step: 0.01, unit: "%" },
      { id: 126, key: "truePeak", label: "True Peak", kind: "toggle", default: 0 },
    ],
  },
];

const PARAMS = GROUPS.flatMap((group) => group.params);
const controls = new Map();
const state = new Map(PARAMS.map((param) => [param.key, param.default]));
const status = document.querySelector("#status");
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
  if (view.byteLength < 9 || view.getUint8(p++) !== 0xa1 || view.getUint8(p++) !== 0x66) return null;
  if (String.fromCharCode(...new Uint8Array(ab, p, 6)) !== "params") return null;
  p += 6;
  const head = view.getUint8(p++);
  if ((head & 0xe0) !== 0xa0) return null;
  let count = head & 0x1f;
  if (count === 24) count = view.getUint8(p++);
  const out = new Map();
  for (let i = 0; i < count; i++) {
    if (p + 13 > view.byteLength || view.getUint8(p++) !== 0x1a) return null;
    const key = view.getUint32(p, false);
    p += 4;
    if (view.getUint8(p++) !== 0xfb) return null;
    const value = view.getFloat64(p, false);
    p += 8;
    out.set(key, value);
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
  return 10 ** (Math.log10(min) + clamp01(norm) * (Math.log10(max) - Math.log10(min)));
}

function valueToInput(def, value) {
  return def.scale === "log" ? logNorm(value, def.min, def.max) : value;
}

function inputToValue(def, value) {
  return def.scale === "log" ? fromLogNorm(value, def.min, def.max) : Number(value);
}

function formatValue(def, value) {
  if (def.kind === "toggle") return value >= 0.5 ? "ON" : "OFF";
  if (def.unit === "dB") return `${value >= 0 ? "+" : ""}${value.toFixed(1)} dB`;
  if (def.unit === "ms") return value >= 100 ? `${Math.round(value)} ms` : `${value.toFixed(1)} ms`;
  if (def.unit === "%") return `${Math.round(value * 100)}%`;
  return value.toFixed(2);
}

function setState(def, value) {
  const next = def.kind === "toggle" ? (value >= 0.5 ? 1 : 0) : clamp(Number(value), def.min, def.max);
  state.set(def.key, next);
  queueGraphUpdate();
}

function sendSet(id, value) {
  window.parent.postMessage(encodeSet(id, value), "*");
}

function queueGraphUpdate() {
  if (graphQueued) return;
  graphQueued = true;
  requestAnimationFrame(() => {
    graphQueued = false;
    paintGraph();
  });
}

function createControl(def) {
  const kind = def.kind ?? "range";
  const wrap = document.createElement("label");
  wrap.className = `control control-${kind}`;

  const title = document.createElement("span");
  title.className = "control-label";
  title.textContent = def.label;

  const readout = document.createElement("span");
  readout.className = "readout";

  let input;
  if (kind === "toggle") {
    input = document.createElement("input");
    input.type = "checkbox";
    const switchBody = document.createElement("span");
    switchBody.className = "switch-body";
    switchBody.append(input, document.createElement("span"));
    wrap.append(title, switchBody, readout);
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

  function paint(value, options = {}) {
    const next = kind === "toggle" ? (Number(value) >= 0.5 ? 1 : 0) : Number(value);
    if (kind === "toggle") {
      input.checked = next >= 0.5;
    } else {
      const inputValue = valueToInput(def, next);
      input.value = String(inputValue);
      const norm = def.scale === "log" ? inputValue : (next - def.min) / (def.max - def.min);
      input.style.setProperty("--value", `${clamp01(norm) * 100}%`);
    }
    readout.textContent = formatValue(def, next);
    if (options.updateState !== false) setState(def, next);
  }

  input.addEventListener(kind === "toggle" ? "change" : "input", () => {
    const next = kind === "toggle" ? (input.checked ? 1 : 0) : inputToValue(def, Number(input.value));
    paint(next);
    sendSet(def.id, next);
  });

  paint(def.default);
  controls.set(def.id, { def, paint });
  return wrap;
}

function buildControls() {
  const root = document.querySelector("#controls");
  for (const group of GROUPS) {
    const section = document.createElement("section");
    section.className = "control-section";
    const title = document.createElement("p");
    title.className = "section-title";
    title.textContent = group.title;
    section.append(title, ...group.params.map(createControl));
    root.append(section);
  }
}

function xForDb(db) {
  return GRAPH.left + ((db - DB_MIN) / (DB_MAX - DB_MIN)) * (GRAPH.width - GRAPH.left - GRAPH.right);
}

function yForDb(db) {
  return GRAPH.top + ((DB_MAX - clamp(db, DB_MIN, DB_MAX)) / (DB_MAX - DB_MIN)) * (GRAPH.height - GRAPH.top - GRAPH.bottom);
}

function path(points) {
  return points.map((p, i) => `${i === 0 ? "M" : "L"}${p.x.toFixed(1)} ${p.y.toFixed(1)}`).join(" ");
}

function limiterOut(inputDb) {
  const driven = inputDb + state.get("inputGain");
  const threshold = state.get("threshold");
  const ceiling = state.get("ceiling");
  const limited = driven > threshold ? threshold + (driven - threshold) * 0.06 : driven;
  return Math.min(limited, ceiling) + state.get("outputGain");
}

function drawGrid() {
  const grid = document.querySelector("#grid");
  if (!grid || grid.childElementCount) return;
  for (const db of [-60, -48, -36, -24, -12, 0, 12]) {
    const y = yForDb(db);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", GRAPH.left);
    line.setAttribute("x2", GRAPH.width - GRAPH.right);
    line.setAttribute("y1", y);
    line.setAttribute("y2", y);
    line.classList.add("grid-line");
    grid.append(line);
    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.setAttribute("x", 12);
    label.setAttribute("y", y + 4);
    label.textContent = db > 0 ? `+${db}` : String(db);
    label.classList.add("axis-label");
    grid.append(label);
  }
  for (const db of [-60, -36, -12, 12]) {
    const x = xForDb(db);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", x);
    line.setAttribute("x2", x);
    line.setAttribute("y1", GRAPH.top);
    line.setAttribute("y2", GRAPH.height - GRAPH.bottom);
    line.classList.add("grid-line");
    grid.append(line);
  }
}

function paintGraph() {
  drawGrid();
  const points = [];
  for (let i = 0; i <= 120; i++) {
    const input = DB_MIN + (DB_MAX - DB_MIN) * (i / 120);
    points.push({ x: xForDb(input), y: yForDb(limiterOut(input)) });
  }
  document.querySelector("#curve-line")?.setAttribute("d", path(points));
  document.querySelector("#unity-line")?.setAttribute("d", path([
    { x: xForDb(DB_MIN), y: yForDb(DB_MIN) },
    { x: xForDb(DB_MAX), y: yForDb(DB_MAX) },
  ]));

  const thresholdX = xForDb(state.get("threshold") - state.get("inputGain"));
  const threshold = document.querySelector("#threshold-line");
  threshold?.setAttribute("x1", thresholdX);
  threshold?.setAttribute("x2", thresholdX);
  threshold?.setAttribute("y1", GRAPH.top);
  threshold?.setAttribute("y2", GRAPH.height - GRAPH.bottom);

  const ceilingY = yForDb(state.get("ceiling") + state.get("outputGain"));
  const ceiling = document.querySelector("#ceiling-line");
  ceiling?.setAttribute("x1", GRAPH.left);
  ceiling?.setAttribute("x2", GRAPH.width - GRAPH.right);
  ceiling?.setAttribute("y1", ceilingY);
  ceiling?.setAttribute("y2", ceilingY);
}

function applySnapshot(snapshot) {
  for (const [id, value] of snapshot) {
    controls.get(id)?.paint(value);
  }
  status.textContent = "CONNECTED";
}

window.addEventListener("message", (event) => {
  if (!(event.data instanceof ArrayBuffer)) return;
  const snapshot = decodeParamsSnapshot(event.data);
  if (snapshot) applySnapshot(snapshot);
});

buildControls();
paintGraph();
window.parent.postMessage(encodeReady(), "*");
