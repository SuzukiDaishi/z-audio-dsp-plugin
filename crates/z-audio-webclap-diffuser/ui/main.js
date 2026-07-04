const GRAPH = {
  width: 760,
  height: 246,
  left: 34,
  right: 34,
  top: 24,
  bottom: 28,
};

const GROUPS = [
  {
    title: "Diffuse",
    params: [
      { id: 220, key: "mix", label: "Mix", min: 0, max: 1, default: 1, step: 0.01, unit: "%" },
      { id: 221, key: "diffusion", label: "Diffusion", min: 0, max: 1, default: 0.04, step: 0.01, unit: "%" },
      { id: 225, key: "allpassCount", label: "AP Count", min: 1, max: 100, default: 100, step: 1, unit: "count" },
      { id: 222, key: "size", label: "Size", min: 0, max: 1, default: 0.5, step: 0.01, unit: "%" },
    ],
  },
  {
    title: "Output",
    params: [
      { id: 223, key: "width", label: "Width", min: 0, max: 1, default: 1, step: 0.01, unit: "%" },
      { id: 224, key: "output", label: "Output", min: -24, max: 24, default: 0, step: 0.1, unit: "dB" },
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

function formatValue(def, value) {
  if (def.unit === "dB") return `${value >= 0 ? "+" : ""}${value.toFixed(1)} dB`;
  if (def.unit === "%") return `${Math.round(value * 100)}%`;
  if (def.unit === "count") return `${Math.round(value)}`;
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
  input.min = def.min;
  input.max = def.max;
  input.step = def.step;

  const rail = document.createElement("span");
  rail.className = "slider-rail";
  rail.append(input);

  const readout = document.createElement("span");
  readout.className = "readout";
  wrap.append(title, rail, readout);

  function paint(value, options = {}) {
    const next = Number(value);
    input.value = String(next);
    input.style.setProperty("--value", `${clamp01((next - def.min) / (def.max - def.min)) * 100}%`);
    readout.textContent = formatValue(def, next);
    if (options.updateState !== false) setState(def, next);
  }

  input.addEventListener("input", () => {
    const next = Number(input.value);
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
  const diffusion = state.get("diffusion");
  const allpassCount = Math.round(state.get("allpassCount"));
  const effectiveOrder = diffusion * allpassCount;
  const activeStages = Math.round(effectiveOrder);
  const wetPresence = Math.min(1, effectiveOrder);
  const size = state.get("size");
  const width = state.get("width");
  const mix = state.get("mix");
  const stageGroup = document.querySelector("#stage-lines");
  const markerGroup = document.querySelector("#delay-markers");
  if (!stageGroup || !markerGroup) return;
  stageGroup.replaceChildren();
  markerGroup.replaceChildren();

  const centerY = GRAPH.height * 0.5;
  const span = GRAPH.width - GRAPH.left - GRAPH.right;
  const baseDelays = [4.7, 3.6, 12.7, 9.3];
  const scale = 0.5 + size;
  const visibleStages = Math.min(activeStages, 24);
  const svg = document.querySelector(".graph-panel svg");
  const svgScale = svg ? svg.getBoundingClientRect().width / GRAPH.width : 1;
  const labelStepPx = visibleStages > 0 ? (span / visibleStages) * svgScale : 0;
  const showBaseDelayLabels = labelStepPx >= 72;

  for (let i = 0; i < visibleStages; i++) {
    const x0 = GRAPH.left + span * (i / visibleStages);
    const x1 = GRAPH.left + span * ((i + 0.72) / visibleStages);
    const y = centerY + (i % 2 === 0 ? -1 : 1) * (24 + width * 30);
    const line = svgEl("path");
    line.classList.add("stage-link");
    line.setAttribute("d", `M${x0} ${centerY} C${x0 + 36} ${y} ${x1 - 36} ${y} ${x1} ${centerY}`);
    line.setAttribute("stroke-opacity", 0.28 + wetPresence * 0.58);
    stageGroup.append(line);

    const node = svgEl("circle");
    node.classList.add("stage-node");
    node.setAttribute("cx", x1);
    node.setAttribute("cy", centerY);
    node.setAttribute("r", Math.max(4, 8 + wetPresence * 7 - visibleStages * 0.24));
    markerGroup.append(node);

    if ((showBaseDelayLabels && i < 4) || i === 0 || i === visibleStages - 1) {
      const label = svgEl("text");
      label.classList.add("stage-label");
      label.setAttribute("x", x1);
      label.setAttribute("y", centerY + 38);
      label.textContent = i === visibleStages - 1 && activeStages > visibleStages
        ? `${activeStages}x`
        : `${(baseDelays[i % baseDelays.length] * scale).toFixed(1)} ms`;
      markerGroup.append(label);
    }
  }

  const energy = [];
  for (let i = 0; i <= 140; i++) {
    const t = i / 140;
    const x = GRAPH.left + span * t;
    const wave = Math.sin(t * Math.PI * (10 + size * 9 + effectiveOrder * 0.22));
    const envelope = (1 - t) ** 1.2;
    const y = centerY + wave * envelope * (18 + wetPresence * 42) * mix * wetPresence;
    energy.push({ x, y });
  }
  document.querySelector("#energy-line")?.setAttribute("d", path(energy));
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
