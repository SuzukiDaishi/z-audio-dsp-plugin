const GRAPH = {
  width: 820,
  height: 286,
  left: 34,
  right: 34,
  top: 24,
  bottom: 28,
};

const GROUPS = [
  {
    title: "Space",
    params: [
      { id: 100, key: "mix", label: "Mix", min: 0, max: 1, default: 0.35, step: 0.01, unit: "%" },
      { id: 101, key: "room", label: "Room", min: 0, max: 1, default: 0.55, step: 0.01, unit: "%" },
      { id: 102, key: "decay", label: "Decay", min: 0.1, max: 20, default: 2.2, step: 0.0001, scale: "log", unit: "s" },
      { id: 103, key: "preDelay", label: "Pre Delay", min: 0, max: 250, default: 18, step: 0.1, unit: "ms" },
      { id: 111, key: "earlyLate", label: "Early/Late", min: 0, max: 1, default: 0.35, step: 0.01, unit: "%" },
    ],
  },
  {
    title: "Texture",
    params: [
      { id: 104, key: "diffusion", label: "Diffusion", min: 0, max: 1, default: 0.65, step: 0.01, unit: "%" },
      { id: 105, key: "damping", label: "Damping", min: 0, max: 1, default: 0.35, step: 0.01, unit: "%" },
      { id: 110, key: "width", label: "Width", min: 0, max: 1, default: 0.9, step: 0.01, unit: "%" },
      { id: 108, key: "modRate", label: "Mod Rate", min: 0, max: 2, default: 0, step: 0.01, unit: "Hz" },
      { id: 109, key: "modDepth", label: "Mod Depth", min: 0, max: 1, default: 0, step: 0.01, unit: "%" },
    ],
  },
  {
    title: "Tone",
    params: [
      { id: 106, key: "lowCut", label: "Low Cut", min: 20, max: 1000, default: 80, step: 0.0001, scale: "log", unit: "Hz" },
      { id: 107, key: "highCut", label: "High Cut", min: 1000, max: 20000, default: 12000, step: 0.0001, scale: "log", unit: "Hz" },
      { id: 112, key: "output", label: "Output", min: -24, max: 24, default: 0, step: 0.1, unit: "dB" },
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

function formatFreq(value) {
  if (value >= 1000) return `${(value / 1000).toFixed(value >= 10000 ? 1 : 2)} kHz`;
  return `${Math.round(value)} Hz`;
}

function formatValue(def, value) {
  if (def.unit === "dB") return `${value >= 0 ? "+" : ""}${value.toFixed(1)} dB`;
  if (def.unit === "ms") return value >= 100 ? `${Math.round(value)} ms` : `${value.toFixed(1)} ms`;
  if (def.unit === "s") return `${value.toFixed(2)} s`;
  if (def.unit === "Hz") return formatFreq(value);
  if (def.unit === "%") return `${Math.round(value * 100)}%`;
  return value.toFixed(2);
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

function setState(def, value) {
  state.set(def.key, clamp(Number(value), def.min, def.max));
  queueGraphUpdate();
}

function createControl(def) {
  const wrap = document.createElement("label");
  wrap.className = "control";

  const title = document.createElement("span");
  title.className = "control-label";
  title.textContent = def.label;

  const input = document.createElement("input");
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

  const readout = document.createElement("span");
  readout.className = "readout";
  wrap.append(title, rail, readout);

  function paint(value, options = {}) {
    const next = Number(value);
    const inputValue = valueToInput(def, next);
    input.value = String(inputValue);
    const norm = def.scale === "log" ? inputValue : (next - def.min) / (def.max - def.min);
    input.style.setProperty("--value", `${clamp01(norm) * 100}%`);
    readout.textContent = formatValue(def, next);
    if (options.updateState !== false) setState(def, next);
  }

  input.addEventListener("input", () => {
    const next = inputToValue(def, Number(input.value));
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

function svgEl(name) {
  return document.createElementNS("http://www.w3.org/2000/svg", name);
}

function path(points) {
  return points.map((p, i) => `${i === 0 ? "M" : "L"}${p.x.toFixed(1)} ${p.y.toFixed(1)}`).join(" ");
}

function paintGraph() {
  const roomGroup = document.querySelector("#room-lines");
  if (!roomGroup) return;
  roomGroup.replaceChildren();

  const room = state.get("room");
  const decay = state.get("decay");
  const diffusion = state.get("diffusion");
  const damping = state.get("damping");
  const earlyLate = state.get("earlyLate");
  const lowCut = state.get("lowCut");
  const highCut = state.get("highCut");

  const cx = GRAPH.width * 0.5;
  const cy = GRAPH.height * 0.48;
  const width = 190 + room * 440;
  const height = 70 + room * 135;
  const left = cx - width / 2;
  const right = cx + width / 2;
  const top = cy - height / 2;
  const bottom = cy + height / 2;

  const wall = svgEl("path");
  wall.classList.add("room-wall");
  wall.setAttribute("d", `M${left} ${top} L${right} ${top} L${right} ${bottom} L${left} ${bottom} Z`);
  roomGroup.append(wall);

  for (let i = 0; i < 10; i++) {
    const t = i / 9;
    const spread = 0.18 + t * (0.58 + diffusion * 0.32);
    const y = top + height * t;
    const alpha = clamp((1 - t * 0.68) * (0.48 + decay / 18), 0.12, 0.9);
    const line = svgEl("line");
    line.classList.add("reflection-line");
    line.setAttribute("x1", cx - width * spread * 0.5);
    line.setAttribute("x2", cx + width * spread * 0.5);
    line.setAttribute("y1", y);
    line.setAttribute("y2", y);
    line.setAttribute("stroke-width", 0.9 + diffusion * 2.2);
    line.setAttribute("stroke-opacity", alpha * (1 - damping * 0.35));
    roomGroup.append(line);
  }

  const tail = [];
  for (let i = 0; i <= 100; i++) {
    const t = i / 100;
    const x = GRAPH.left + t * (GRAPH.width - GRAPH.left - GRAPH.right);
    const curve = Math.exp(-(t * 5.5) / Math.max(0.2, decay));
    const y = GRAPH.height - GRAPH.bottom - curve * (GRAPH.height - GRAPH.top - GRAPH.bottom - 18);
    tail.push({ x, y });
  }
  document.querySelector("#tail-line")?.setAttribute("d", path(tail));

  const markerX = GRAPH.left + earlyLate * (GRAPH.width - GRAPH.left - GRAPH.right);
  const marker = document.querySelector("#early-marker");
  marker?.setAttribute("x1", markerX);
  marker?.setAttribute("x2", markerX);
  marker?.setAttribute("y1", GRAPH.top);
  marker?.setAttribute("y2", GRAPH.height - GRAPH.bottom);

  const lowNorm = logNorm(lowCut, 20, 20000);
  const highNorm = logNorm(highCut, 20, 20000);
  const fx1 = GRAPH.left + lowNorm * (GRAPH.width - GRAPH.left - GRAPH.right);
  const fx2 = GRAPH.left + highNorm * (GRAPH.width - GRAPH.left - GRAPH.right);
  document.querySelector("#filter-band")?.setAttribute(
    "d",
    `M${fx1.toFixed(1)} ${GRAPH.top} L${fx2.toFixed(1)} ${GRAPH.top} L${fx2.toFixed(1)} ${GRAPH.height - GRAPH.bottom} L${fx1.toFixed(1)} ${GRAPH.height - GRAPH.bottom} Z`,
  );
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
