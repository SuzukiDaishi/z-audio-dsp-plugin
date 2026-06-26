const PARAMS = [
  { id: 180, label: "Master Gain", min: -24, max: 12, default: 0, unit: " dB" },
  { id: 181, label: "Tone", min: 0, max: 1, default: 1, unit: "" },
  { id: 182, label: "Velocity Curve", min: 0, max: 1, default: 0.5, unit: "" },
  { id: 183, label: "Release Level", min: -24, max: 12, default: 0, unit: " dB" },
  { id: 184, label: "Release Time", min: 0.05, max: 5, default: 0.35, unit: " s" },
  { id: 185, label: "Stereo Width", min: 0, max: 1, default: 1, unit: "" },
];

const controlsForm = document.querySelector("#controls");
const status = document.querySelector("#status");
const controls = new Map();

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function formatValue(def, value) {
  return `${value.toFixed(2)}${def.unit}`;
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

function sendSet(id, value) {
  window.parent.postMessage(encodeSet(id, value), "*");
}

function createControl(def) {
  const wrap = document.createElement("label");
  wrap.className = "control";

  const title = document.createElement("span");
  title.className = "control-label";
  title.textContent = def.label;

  const readout = document.createElement("span");
  readout.className = "readout";
  readout.textContent = formatValue(def, def.default);

  const input = document.createElement("input");
  input.type = "range";
  input.min = def.min;
  input.max = def.max;
  input.step = (def.max - def.min) / 200;
  input.value = def.default;

  input.addEventListener("input", () => {
    const value = clamp(Number(input.value), def.min, def.max);
    readout.textContent = formatValue(def, value);
    sendSet(def.id, value);
  });

  wrap.append(title, input, readout);
  controlsForm.append(wrap);
  controls.set(def.id, { input, readout, def });
}

function applySnapshot(snapshot) {
  for (const [id, value] of snapshot) {
    const control = controls.get(id);
    if (!control) continue;
    const clamped = clamp(value, control.def.min, control.def.max);
    control.input.value = clamped;
    control.readout.textContent = formatValue(control.def, clamped);
  }
  status.textContent = "CONNECTED";
}

window.addEventListener("message", (event) => {
  if (!(event.data instanceof ArrayBuffer)) return;
  const snapshot = decodeParamsSnapshot(event.data);
  if (snapshot) applySnapshot(snapshot);
});

PARAMS.forEach(createControl);
window.parent.postMessage(encodeReady(), "*");
