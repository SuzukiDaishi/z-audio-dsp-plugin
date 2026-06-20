const BASE_WIDTH = 960;
const BASE_HEIGHT = 540;

const PARAMS = [
  {
    group: "osc",
    id: 2,
    label: "Shape",
    type: "select",
    options: ["Sine", "Triangle", "Saw", "Pulse", "Noise"],
    default: 0,
  },
  {
    group: "osc",
    id: 10,
    label: "Level",
    type: "slider",
    min: 0,
    max: 2,
    step: 0.001,
    default: 1,
  },
  {
    group: "osc",
    id: 11,
    label: "Pulse Width",
    type: "slider",
    min: 0.05,
    max: 0.95,
    step: 0.001,
    default: 0.5,
  },
  {
    group: "env",
    id: 20,
    label: "Attack",
    type: "slider",
    min: 0,
    max: 10,
    step: 0.001,
    default: 0.01,
    unit: "s",
  },
  {
    group: "env",
    id: 21,
    label: "Decay",
    type: "slider",
    min: 0,
    max: 10,
    step: 0.001,
    default: 0.1,
    unit: "s",
  },
  {
    group: "env",
    id: 22,
    label: "Sustain",
    type: "slider",
    min: 0,
    max: 1,
    step: 0.001,
    default: 0.7,
  },
  {
    group: "env",
    id: 23,
    label: "Release",
    type: "slider",
    min: 0,
    max: 10,
    step: 0.001,
    default: 0.2,
    unit: "s",
  },
  {
    group: "env",
    id: 24,
    label: "Curve",
    type: "select",
    options: ["Linear", "Expo"],
    default: 1,
  },
  {
    group: "lfo",
    id: 31,
    label: "Waveform",
    type: "select",
    options: ["Sine", "Triangle", "Saw Up", "Saw Down", "Square", "Random"],
    default: 0,
  },
  {
    group: "lfo",
    id: 32,
    label: "Rate",
    type: "slider",
    min: 0.01,
    max: 20,
    step: 0.01,
    default: 5,
    unit: "Hz",
  },
  {
    group: "lfo",
    id: 33,
    label: "Depth",
    type: "slider",
    min: 0,
    max: 12,
    step: 0.001,
    default: 0,
  },
  {
    group: "lfo",
    id: 34,
    label: "Route",
    type: "select",
    options: [
      { value: 0, label: "None" },
      { value: 1, label: "Gain" },
      { value: 2, label: "Pitch" },
    ],
    default: 0,
  },
  {
    group: "out",
    id: 0,
    label: "Master",
    type: "slider",
    min: 0,
    max: 2,
    step: 0.001,
    default: 1,
  },
];

const GROUPS = [
  { id: "osc", title: "Oscillator" },
  { id: "env", title: "Amp Env" },
  { id: "lfo", title: "LFO" },
  { id: "out", title: "Output" },
];

const controls = new Map();
const values = new Map(PARAMS.map((param) => [param.id, param.default ?? 0]));
const status = document.querySelector("#status");

let visualQueued = false;

function resizePlugin() {
  const scale = Math.min(
    window.innerWidth / BASE_WIDTH,
    window.innerHeight / BASE_HEIGHT,
  );
  document.documentElement.style.setProperty("--scale", String(Math.max(0.2, scale)));
}

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

function optionValue(option, index) {
  return typeof option === "object" ? option.value : index;
}

function optionLabel(option) {
  return typeof option === "object" ? option.label : option;
}

function normalizeSelectValue(def, value) {
  const rounded = Math.round(value);
  return def.options.some((option, index) => optionValue(option, index) === rounded)
    ? rounded
    : optionValue(def.options[0], 0);
}

function selectedLabel(def, value) {
  const normalized = normalizeSelectValue(def, value);
  const found = def.options.find((option, index) => optionValue(option, index) === normalized);
  return found ? optionLabel(found) : "";
}

function format(def, value) {
  if (def.type === "select") return selectedLabel(def, value);
  if (def.unit === "s" && value < 1) return `${Math.round(value * 1000)} ms`;
  const decimals = def.step >= 1 ? 0 : def.step >= 0.01 ? 2 : 3;
  return `${Number(value).toFixed(decimals)}${def.unit ? ` ${def.unit}` : ""}`;
}

function clamp01(value) {
  return Math.max(0, Math.min(1, value));
}

function queueVisualUpdate() {
  if (visualQueued) return;
  visualQueued = true;
  requestAnimationFrame(() => {
    visualQueued = false;
    updateVisuals();
  });
}

function createControl(def) {
  const wrap = document.createElement("div");
  wrap.className = `control control-${def.type}`;

  const label = document.createElement("label");
  label.textContent = def.label;

  const value = document.createElement("span");
  value.className = "readout";

  let input;
  if (def.type === "select") {
    input = document.createElement("select");
    input.id = `param-${def.id}`;
    input.autocomplete = "off";
    for (let i = 0; i < def.options.length; i++) {
      const option = document.createElement("option");
      option.value = optionValue(def.options[i], i);
      option.textContent = optionLabel(def.options[i]);
      input.append(option);
    }
    input.value = normalizeSelectValue(def, def.default ?? 0);
    wrap.append(label, input, value);
  } else {
    input = document.createElement("input");
    input.id = `param-${def.id}`;
    input.type = "range";
    input.autocomplete = "off";
    input.min = def.min;
    input.max = def.max;
    input.step = def.step;
    input.value = def.default ?? def.min;

    const rail = document.createElement("div");
    rail.className = "slider-rail";
    rail.append(input);
    wrap.append(label, rail, value);
  }

  label.htmlFor = input.id;

  const paintRange = (next) => {
    if (def.type !== "slider") return;
    const norm = clamp01((next - def.min) / (def.max - def.min));
    input.style.setProperty("--value", `${norm * 100}%`);
  };

  const read = () => Number(input.value);
  const paint = (next) => {
    const visible = def.type === "select" ? normalizeSelectValue(def, next) : Number(next);
    input.value = String(visible);
    values.set(def.id, visible);
    value.textContent = format(def, visible);
    paintRange(visible);
    queueVisualUpdate();
  };

  input.addEventListener("input", () => {
    const next = read();
    paint(next);
    window.parent.postMessage(encodeSet(def.id, next), "*");
  });

  paint(def.default ?? 0);
  controls.set(def.id, { def, paint });
  return wrap;
}

function createScope(id) {
  const scope = document.createElement("div");
  scope.className = `scope ${id}-scope`;
  scope.innerHTML = `
    <svg viewBox="0 0 280 118" preserveAspectRatio="none" aria-hidden="true">
      <path id="${id}-path"></path>
    </svg>
    <div class="scope-readout" id="${id}-readout"></div>
  `;
  return scope;
}

function buildControls() {
  const form = document.querySelector("#controls");
  for (const group of GROUPS) {
    const bay = document.createElement("section");
    bay.className = `bay bay-${group.id}`;

    const header = document.createElement("header");
    header.className = "bay-header";
    header.innerHTML = `<h2>${group.title}</h2>`;
    if (group.id === "lfo") {
      const badge = document.createElement("span");
      badge.id = "lfo-state";
      badge.className = "module-badge";
      header.append(badge);
    }

    const body = document.createElement("div");
    body.className = "bay-body";

    if (group.id !== "out") {
      body.append(createScope(group.id));
    }

    const grid = document.createElement("div");
    grid.className = "control-grid";
    for (const def of PARAMS.filter((param) => param.group === group.id)) {
      grid.append(createControl(def));
    }

    body.append(grid);
    bay.append(header, body);
    form.append(bay);
  }
  updateVisuals();
}

function oscillatorSample(kind, t, pulse) {
  switch (Math.round(kind)) {
    case 1:
      return 1 - Math.abs(t * 4 - 2);
    case 2:
      return t * 2 - 1;
    case 3:
      return t < pulse ? 0.9 : -0.9;
    case 4:
      return (((Math.sin(t * 83.1) * 43758.5453) % 1) * 2) - 1;
    default:
      return Math.sin(t * Math.PI * 2);
  }
}

function lfoSample(kind, t) {
  switch (Math.round(kind)) {
    case 1:
      return 1 - Math.abs(t * 4 - 2);
    case 2:
      return t * 2 - 1;
    case 3:
      return 1 - t * 2;
    case 4:
      return t < 0.5 ? 0.85 : -0.85;
    case 5:
      return (((Math.sin(Math.floor(t * 8) * 91.7) * 143.22) % 1) * 2) - 1;
    default:
      return Math.sin(t * Math.PI * 2);
  }
}

function wavePath(sampleFn, cycles = 1) {
  const points = [];
  const width = 280;
  const height = 118;
  for (let i = 0; i <= 112; i++) {
    const x = (i / 112) * width;
    const sample = sampleFn((i / 112) * cycles);
    const y = height * 0.5 - sample * height * 0.36;
    points.push(`${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(1)}`);
  }
  return points.join(" ");
}

function setScopePath(id, path) {
  const stroke = document.querySelector(`#${id}-path`);
  if (!stroke) return;
  stroke.setAttribute("d", path);
}

function timeWeight(seconds) {
  return Math.log10(seconds + 1) / Math.log10(11);
}

function param(id) {
  return values.get(id) ?? PARAMS.find((def) => def.id === id)?.default ?? 0;
}

function updateOscVisual() {
  const kind = param(2);
  const pulse = param(11);
  const path = wavePath((x) => oscillatorSample(kind, x % 1, pulse), kind === 4 ? 3 : 1);
  setScopePath("osc", path);

  const readout = document.querySelector("#osc-readout");
  if (readout) {
    readout.innerHTML = `<span>${selectedLabel(PARAMS[0], kind)}</span><span>PW ${Math.round(pulse * 100)}%</span>`;
  }
}

function updateEnvVisual() {
  const attack = param(20);
  const decay = param(21);
  const sustain = param(22);
  const release = param(23);
  const attackX = 12 + timeWeight(attack) * 58;
  const decayX = attackX + 20 + timeWeight(decay) * 58;
  const releaseX = 268 - timeWeight(release) * 62;
  const sustainY = 104 - sustain * 78;
  const path = [
    "M10 106",
    `L${attackX.toFixed(1)} 16`,
    `L${decayX.toFixed(1)} ${sustainY.toFixed(1)}`,
    `L${releaseX.toFixed(1)} ${sustainY.toFixed(1)}`,
    "L270 106",
  ].join(" ");
  setScopePath("env", path);

  const readout = document.querySelector("#env-readout");
  if (readout) {
    readout.innerHTML = `<span>A ${format(PARAMS[3], attack)}</span><span>S ${sustain.toFixed(2)}</span><span>R ${format(PARAMS[6], release)}</span>`;
  }
}

function updateLfoVisual() {
  const waveform = param(31);
  const rate = param(32);
  const amount = param(33);
  const route = param(34);
  const bypassed = route === 0 || amount <= 0.0001;
  const depth = bypassed ? 0.12 : Math.max(0.16, Math.min(1, amount / 12));
  const cycles = Math.max(1, Math.min(5, rate / 4));
  const path = wavePath((x) => lfoSample(waveform, x % 1) * depth, cycles);
  setScopePath("lfo", path);

  const lfoBay = document.querySelector(".bay-lfo");
  lfoBay?.classList.toggle("is-bypassed", bypassed);

  const badge = document.querySelector("#lfo-state");
  if (badge) {
    badge.textContent = bypassed ? "Bypassed" : selectedLabel(PARAMS[11], route);
  }

  const readout = document.querySelector("#lfo-readout");
  if (readout) {
    readout.innerHTML = `<span>${selectedLabel(PARAMS[8], waveform)}</span><span>${rate.toFixed(2)} Hz</span><span>Depth ${amount.toFixed(2)}</span>`;
  }
}

function updateOutputVisual() {
  const master = param(0);
  const shell = document.querySelector(".synth-shell");
  shell?.style.setProperty("--master", String(clamp01(master / 2)));
}

function updateVisuals() {
  updateOscVisual();
  updateEnvVisual();
  updateLfoVisual();
  updateOutputVisual();
}

function applySnapshot(snapshot) {
  for (const [id, next] of snapshot) {
    const control = controls.get(id);
    if (!control) continue;
    control.paint(next);
  }
  status.textContent = "Connected";
}

window.addEventListener("resize", resizePlugin);
window.addEventListener("message", (event) => {
  if (!(event.data instanceof ArrayBuffer)) return;
  const snapshot = decodeParamsSnapshot(event.data);
  if (snapshot) applySnapshot(snapshot);
});

resizePlugin();
buildControls();
window.parent.postMessage(encodeReady(), "*");
