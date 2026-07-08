import ClapAudioNode from "./clap-audionode/clap-audionode.mjs?v=z-audio-host-5";

const SLOT_COUNT = 4;
const SOURCE_TONE = "tone";
const SOURCE_FILE = "file";
const EMPTY_SLOT_NAME = "Empty pass-through slot";
const EMPTY_SLOT_META = "Audio/MIDI passes through unchanged. Drop WebCLAP here.";
const CLAP_EVENT_NOTE_ON = 0;
const CLAP_EVENT_NOTE_OFF = 1;
const CLAP_NOTE_EVENT_SIZE = 40;
const REMOTE_CALL_TIMEOUT_MS = 2500;
const PLUGIN_NODE_OPTIONS = {
  numberOfInputs: 1,
  numberOfOutputs: 1,
  outputChannelCount: [2],
};
const SHELF_ENDPOINT = "./__webclap_bundles.json";
const DEFAULT_SHELF_BUNDLES = [
  ["Synth", "../../target/webclap/z-audio-simple-synth.wclap.tar.gz"],
  ["Formula Piano", "../../target/webclap/z-audio-formula-piano.wclap.tar.gz"],
  ["VCSL Piano", "../../target/webclap/z-audio-vcsl-piano.wclap.tar.gz"],
  ["Sampler", "../../target/webclap/z-audio-sampler.wclap.tar.gz"],
  ["Granular", "../../target/webclap/z-audio-granular.wclap.tar.gz"],
  ["Wave Synth", "../../target/webclap/z-audio-wavetable.wclap.tar.gz"],
  ["Drums", "../../target/webclap/z-audio-formula-drums.wclap.tar.gz"],
  ["EQ", "../../target/webclap/z-audio-simple-eq.wclap.tar.gz"],
  ["Ring Mod", "../../target/webclap/z-audio-ring-mod.wclap.tar.gz"],
  ["Distortion", "../../target/webclap/z-audio-distortion.wclap.tar.gz"],
  ["Saturator", "../../target/webclap/z-audio-saturator.wclap.tar.gz"],
  ["Bitcrusher", "../../target/webclap/z-audio-bitcrusher.wclap.tar.gz"],
  ["Delay", "../../target/webclap/z-audio-delay.wclap.tar.gz"],
  ["Chorus", "../../target/webclap/z-audio-chorus.wclap.tar.gz"],
  ["Flanger", "../../target/webclap/z-audio-flanger.wclap.tar.gz"],
  ["Phaser", "../../target/webclap/z-audio-phaser.wclap.tar.gz"],
  ["Tremolo", "../../target/webclap/z-audio-tremolo.wclap.tar.gz"],
  ["Gate", "../../target/webclap/z-audio-gate.wclap.tar.gz"],
  ["Diffuser", "../../target/webclap/z-audio-diffuser.wclap.tar.gz"],
  ["Reverb", "../../target/webclap/z-audio-parametric-reverb.wclap.tar.gz"],
  ["Limiter", "../../target/webclap/z-audio-limiter.wclap.tar.gz"],
  ["Compressor", "../../target/webclap/z-audio-compressor.wclap.tar.gz"],
].map(([label, url]) => ({ label, url }));

const $ = document.querySelector.bind(document);
const slotsRoot = $("#slots");
const shelfRoot = $("#shelf");
const paramsPanel = $("#params-panel");
const statusLine = $("#status-line");
const sampleRateReadout = $("#sample-rate");
const isolationState = $("#isolation-state");
const cpuReadout = $("#cpu-readout");
const meterL = $("#meter-l");
const meterR = $("#meter-r");
const audioElement = $("#audio-element");
const audioInput = $("#audio-file-input");
const audioDrop = $("#audio-drop");
const audioFileName = $("#audio-file-name");
const seekSlider = $("#seek-slider");
const timeReadout = $("#time-readout");
const loopToggle = $("#loop-toggle");
const toneButton = $("#tone-source-button");
const fileButton = $("#file-source-button");
const signalKind = $("#signal-kind");
const signalKindReadout = $("#signal-kind-readout");
const toneFrequency = $("#tone-frequency");
const toneFrequencyReadout = $("#tone-frequency-readout");
const volumeSlider = $("#volume-slider");
const volumeReadout = $("#volume-readout");
const startButton = $("#start-button");
const stopButton = $("#stop-button");
const midiInputSelect = $("#midi-input-select");
const midiRescanButton = $("#midi-rescan-button");
const midiAllNotesOffButton = $("#midi-all-notes-off-button");
const midiStatus = $("#midi-status");
const floatingPanels = $("#floating-panels");

const slots = Array.from({ length: SLOT_COUNT }, (_, index) => ({
  index,
  module: null,
  node: null,
  objectUrl: null,
  sourceLabel: "",
  descriptor: null,
  plugins: [],
  pluginId: null,
  params: [],
  latencySamples: 0,
  uiSize: null,
  bypass: false,
  loading: false,
  error: "",
}));

let audioContext = null;
let mediaSource = null;
let sourceGain = null;
let outputGain = null;
let splitter = null;
let analyserL = null;
let analyserR = null;
let signalSource = null;
let signalGain = null;
let sourceMode = SOURCE_TONE;
let selectedSlot = 0;
let pageProxy = null;
let animationFrame = 0;
let cpuTimer = 0;
let currentAudioObjectUrl = null;
let dragDepth = 0;
let midiAccess = null;
let activeMidiInput = null;
const heldMidiNotes = new Set();
const floatingPanelClosers = new Map();
// Plugin UIs live in their own browser windows, one per slot.
const pluginWindows = new Map(); // slot index → { win, iframe, frameId, node, watcher }
const frameResolvers = new Map(); // frame id → plugin node (page-proxy lookups)
const DEFAULT_UI_SIZE = { width: 900, height: 620 };

boot();

async function boot() {
  isolationState.textContent = String(globalThis.crossOriginIsolated);
  renderShelf(DEFAULT_SHELF_BUNDLES);
  renderSlots();
  renderDetails();
  wireUi();
  setSourceMode(SOURCE_TONE);
  setStatus("Idle");
  refreshShelf();

  try {
    pageProxy = await globalThis.pageProxyReady;
    // Plugin UI iframes live in popup windows, so resolve proxied resource
    // requests through our own frame registry instead of the default
    // host-document getElementById lookup.
    pageProxy.getResource = (path) => {
      const frameId = path.substr(1).replace(/[/?#].*/, "");
      const node = frameResolvers.get(frameId);
      if (!node) return null;
      const rest = path.substr(frameId.length + 1);
      if (rest.startsWith("/file/")) return node.getFile(rest.slice(5));
      if (rest.startsWith("/get_resource/")) return node.getResource(rest.slice(13));
      return null;
    };
  } catch (error) {
    setStatus(`UI proxy unavailable: ${messageFromError(error)}`, true);
  }

  window.addEventListener("pagehide", closeAllPluginWindows);
}

function wireUi() {
  startButton.addEventListener("click", startTransport);
  stopButton.addEventListener("click", stopTransport);

  toneButton.addEventListener("click", () => setSourceMode(SOURCE_TONE));
  fileButton.addEventListener("click", () => setSourceMode(SOURCE_FILE));

  signalKind.addEventListener("change", () => {
    updateSignalUi();
    if (sourceMode === SOURCE_TONE && audioContext) {
      stopTone();
      startTone();
    }
  });

  toneFrequency.addEventListener("input", () => {
    const frequency = Number(toneFrequency.value);
    toneFrequencyReadout.textContent = `${Math.round(frequency)} Hz`;
    if (signalSource?.frequency) signalSource.frequency.setTargetAtTime(frequency, audioContext.currentTime, 0.01);
  });

  volumeSlider.addEventListener("input", () => {
    const db = Number(volumeSlider.value);
    volumeReadout.textContent = `${db.toFixed(1)} dB`;
    if (outputGain && audioContext) {
      outputGain.gain.setTargetAtTime(dbToGain(db), audioContext.currentTime, 0.01);
    }
  });

  audioDrop.addEventListener("click", () => audioInput.click());
  audioDrop.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      audioInput.click();
    }
  });
  audioInput.addEventListener("change", () => {
    const file = audioInput.files?.[0];
    if (file) loadAudioFile(file);
  });

  loopToggle.addEventListener("change", () => {
    audioElement.loop = loopToggle.checked;
  });
  audioElement.loop = loopToggle.checked;
  audioElement.addEventListener("loadedmetadata", updateTimeUi);
  audioElement.addEventListener("timeupdate", updateTimeUi);
  audioElement.addEventListener("ended", updateTimeUi);

  let seeking = false;
  seekSlider.addEventListener("pointerdown", () => {
    seeking = true;
  });
  seekSlider.addEventListener("pointerup", () => {
    seeking = false;
  });
  seekSlider.addEventListener("input", () => {
    const duration = finiteDuration();
    if (duration > 0) audioElement.currentTime = Number(seekSlider.value) * duration;
  });
  audioElement.addEventListener("timeupdate", () => {
    if (!seeking) updateTimeUi();
  });

  document.body.addEventListener("dragenter", (event) => {
    dragDepth += 1;
    if (hasUsefulDrop(event)) document.body.classList.add("drag-over");
  });
  document.body.addEventListener("dragover", (event) => {
    if (!hasUsefulDrop(event)) return;
    event.preventDefault();
  });
  document.body.addEventListener("dragleave", () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) document.body.classList.remove("drag-over");
  });
  document.body.addEventListener("drop", (event) => {
    clearDropState();
    handlePageDrop(event);
  });
  window.addEventListener("dragend", clearDropState);
  window.addEventListener("blur", clearDropState);

  audioDrop.addEventListener("dragover", (event) => {
    if (!hasAudioFile(event)) return;
    event.preventDefault();
    audioDrop.classList.add("drag-over");
  });
  audioDrop.addEventListener("dragleave", () => audioDrop.classList.remove("drag-over"));
  audioDrop.addEventListener("drop", (event) => {
    const file = firstFile(event);
    if (!file || !isAudioFile(file)) return;
    event.preventDefault();
    event.stopPropagation();
    clearDropState();
    loadAudioFile(file);
  });

  midiRescanButton.addEventListener("click", refreshMidiInputs);
  midiInputSelect.addEventListener("change", selectMidiInput);
  midiAllNotesOffButton.addEventListener("click", sendAllNotesOff);

  updateSignalUi();
}

async function refreshShelf() {
  try {
    const response = await fetch(SHELF_ENDPOINT, { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const bundles = await response.json();
    if (Array.isArray(bundles) && bundles.length) renderShelf(bundles);
  } catch (error) {
    console.warn("Shelf auto-scan unavailable; using built-in bundle list.", error);
  }
}

function renderShelf(bundles) {
  shelfRoot.replaceChildren();
  const title = document.createElement("p");
  title.className = "shelf-title";
  title.textContent = "Shelf";
  shelfRoot.append(title);

  const seen = new Set();
  for (const bundle of bundles) {
    const url = bundle?.url;
    if (!url || seen.has(url)) continue;
    seen.add(url);
    const button = document.createElement("button");
    button.className = "shelf-chip";
    button.type = "button";
    button.dataset.url = url;
    button.textContent = bundle.label || labelFromBundleUrl(url);
    wireShelfButton(button);
    shelfRoot.append(button);
  }

  if (seen.size === 0) {
    const empty = document.createElement("span");
    empty.className = "shelf-empty";
    empty.textContent = "Run cargo xtask bundle-webclap --release";
    shelfRoot.append(empty);
  }
}

function wireShelfButton(button) {
  button.addEventListener("click", () =>
    loadPluginUrl(firstAvailableSlot(), cacheBustedBundleUrl(button.dataset.url), button.textContent.trim()),
  );
  button.draggable = true;
  button.addEventListener("dragstart", (event) => {
    event.dataTransfer.setData("text/x-z-audio-wclap-url", cacheBustedBundleUrl(button.dataset.url));
    event.dataTransfer.setData("text/plain", button.textContent.trim());
  });
}

function cacheBustedBundleUrl(url) {
  if (!url) return url;
  const next = new URL(url, document.baseURI);
  if (!next.pathname.includes("/target/webclap/")) return url;
  if (!next.searchParams.has("v")) next.searchParams.set("v", String(Date.now()));
  next.searchParams.set("load", String(Date.now()));
  return next.href;
}

async function ensureAudioGraph() {
  if (audioContext) return;

  audioContext = new AudioContext();
  sourceGain = audioContext.createGain();
  outputGain = audioContext.createGain();
  splitter = audioContext.createChannelSplitter(2);
  analyserL = audioContext.createAnalyser();
  analyserR = audioContext.createAnalyser();
  mediaSource = audioContext.createMediaElementSource(audioElement);

  analyserL.fftSize = 512;
  analyserR.fftSize = 512;
  outputGain.gain.value = dbToGain(Number(volumeSlider.value));
  outputGain.connect(audioContext.destination);
  outputGain.connect(splitter);
  splitter.connect(analyserL, 0);
  splitter.connect(analyserR, 1);

  sampleRateReadout.textContent = `${Math.round(audioContext.sampleRate)} Hz`;
  audioContext.addEventListener("statechange", () => {
    setStatus(audioContext.state === "running" ? "Running" : audioContext.state);
  });
  connectSelectedSource();
  reconnectGraph();
  startMeters();
  startCpuTimer();
}

async function startTransport() {
  await ensureAudioGraph();
  await audioContext.resume();
  if (sourceMode === SOURCE_TONE) {
    startTone();
  } else {
    if (!audioElement.src) {
      setStatus("Load an audio file first", true);
      return;
    }
    try {
      const duration = finiteDuration();
      if (duration > 0 && audioElement.currentTime >= duration) {
        audioElement.currentTime = 0;
      }
      await audioElement.play();
    } catch (error) {
      setStatus(`Audio file could not play: ${messageFromError(error)}`, true);
    }
  }
}

function stopTransport() {
  if (sourceMode === SOURCE_TONE) stopTone();
  sendAllNotesOff();
  audioElement.pause();
  if (audioContext) audioContext.suspend();
}

function setSourceMode(nextMode) {
  sourceMode = nextMode;
  toneButton.classList.toggle("active", nextMode === SOURCE_TONE);
  fileButton.classList.toggle("active", nextMode === SOURCE_FILE);
  if (nextMode === SOURCE_TONE) audioElement.pause();
  if (audioContext) connectSelectedSource();
}

function connectSelectedSource() {
  stopTone();
  tryDisconnect(mediaSource);
  if (sourceMode === SOURCE_FILE) {
    mediaSource.connect(sourceGain);
    return;
  }
  startTone();
}

function startTone() {
  if (!audioContext || signalSource) return;
  const kind = signalKind.value;
  signalGain = audioContext.createGain();
  signalGain.gain.value = dbToGain(kind.endsWith("noise") || isNoiseKind(kind) ? -16 : -10);

  if (kind === "sine" || kind === "triangle") {
    const oscillator = audioContext.createOscillator();
    oscillator.type = kind;
    oscillator.frequency.value = Number(toneFrequency.value);
    signalSource = oscillator;
  } else {
    const bufferSource = audioContext.createBufferSource();
    bufferSource.buffer = createNoiseBuffer(kind, audioContext.sampleRate);
    bufferSource.loop = true;
    signalSource = bufferSource;
  }

  signalSource.connect(signalGain);
  signalGain.connect(sourceGain);
  signalSource.start();
}

function stopTone() {
  if (!signalSource) return;
  try {
    signalSource.stop();
  } catch {
    // Already stopped.
  }
  tryDisconnect(signalSource);
  tryDisconnect(signalGain);
  signalSource = null;
  signalGain = null;
}

function reconnectGraph() {
  if (!sourceGain || !outputGain) return;
  tryDisconnect(sourceGain);
  for (const slot of slots) {
    if (slot.node) {
      tryDisconnect(slot.node);
      tryDisconnectEvents(slot.node);
    }
  }

  let tail = sourceGain;
  for (const slot of slots) {
    if (!slot.node || slot.bypass || !usesAudioLane(slot)) continue;
    tail.connect(slot.node);
    tail = slot.node;
  }
  tail.connect(outputGain);
  reconnectEventGraph();
}

function reconnectEventGraph() {
  let previous = null;
  for (const slot of slots) {
    if (!slot.node || slot.bypass || !usesEventLane(slot)) continue;
    if (previous?.connectEvents) {
      previous.connectEvents(slot.node).catch((error) => {
        setStatus(`MIDI event routing failed: ${messageFromError(error)}`, true);
      });
    }
    previous = slot.node;
  }
}

async function loadPluginUrl(slotIndex, url, label = url) {
  if (!url) return;
  await ensureAudioGraph();
  await clearSlot(slotIndex);
  const slot = slots[slotIndex];
  slot.loading = true;
  slot.sourceLabel = label;
  renderSlots();
  setStatus(`Loading ${label}`);

  try {
    await instantiatePlugin(slot, url, label);
    setStatus(`Loaded ${slot.descriptor?.name || label}`);
  } catch (error) {
    slot.error = messageFromError(error);
    slot.node = null;
    setStatus(`Plugin load failed: ${slot.error}`, true);
  } finally {
    slot.loading = false;
    selectedSlot = slotIndex;
    reconnectGraph();
    renderSlots();
    renderDetails();
  }
}

async function loadPluginFile(slotIndex, file) {
  const url = URL.createObjectURL(file);
  await loadPluginObjectUrl(slotIndex, url, file.name);
}

async function loadPluginObjectUrl(slotIndex, objectUrl, label) {
  await ensureAudioGraph();
  await clearSlot(slotIndex);
  const slot = slots[slotIndex];
  slot.objectUrl = objectUrl;
  slot.loading = true;
  slot.sourceLabel = label;
  renderSlots();
  setStatus(`Loading ${label}`);

  try {
    await instantiatePlugin(slot, objectUrl, label);
    setStatus(`Loaded ${slot.descriptor?.name || label}`);
  } catch (error) {
    slot.error = messageFromError(error);
    slot.node = null;
    setStatus(`Plugin load failed: ${slot.error}`, true);
  } finally {
    slot.loading = false;
    selectedSlot = slotIndex;
    reconnectGraph();
    renderSlots();
    renderDetails();
  }
}

async function instantiatePlugin(slot, url, label) {
  slot.module = new ClapAudioNode(url);
  slot.plugins = await slot.module.plugins();
  const plugin = slot.plugins[0];
  slot.pluginId = plugin?.id || null;
  slot.node = await slot.module.createNode(audioContext, slot.pluginId, PLUGIN_NODE_OPTIONS);
  slot.descriptor = slot.node.descriptor || plugin || { name: label };
  slot.params = await readParams(slot.node);
  slot.latencySamples = await readLatency(slot.node);
  slot.uiSize = await readPluginUiSize(slot.node);
  slot.error = "";
}

/** Reads the plugin-declared UI size from the bundled plugin.json, if any. */
async function readPluginUiSize(node) {
  if (!node?.getFile) return null;
  try {
    const file = await node.getFile("/plugin.json");
    if (!file) return null;
    const text = file instanceof Blob ? await file.text() : new TextDecoder().decode(file);
    const ui = JSON.parse(text)?.ui;
    const size = ui?.expanded_size || ui?.compact_size;
    const width = Math.round(finiteNumber(size?.width, 0));
    const height = Math.round(finiteNumber(size?.height, 0));
    if (width > 0 && height > 0) return { width, height };
  } catch {
    // Bundles without a readable manifest fall back to measuring the UI.
  }
  return null;
}

async function readParams(node) {
  if (!node?.getParams) return [];
  try {
    return await withTimeout(node.getParams(), REMOTE_CALL_TIMEOUT_MS, "parameter scan timed out");
  } catch (error) {
    setStatus(`Could not read params: ${messageFromError(error)}`, true);
    return [];
  }
}

async function readLatency(node) {
  if (!node?.getLatency) return 0;
  try {
    const value = await withTimeout(node.getLatency(), REMOTE_CALL_TIMEOUT_MS, "latency read timed out");
    return Math.max(0, Math.round(finiteNumber(value, 0)));
  } catch (error) {
    setStatus(`Could not read latency: ${messageFromError(error)}`, true);
    return 0;
  }
}

async function clearSlot(index) {
  const slot = slots[index];
  closeSlotFloatingPanels(index);
  closePluginWindow(index);
  if (slot.node) tryDisconnect(slot.node);
  if (slot.node) tryDisconnectEvents(slot.node);
  if (slot.objectUrl) URL.revokeObjectURL(slot.objectUrl);
  Object.assign(slot, {
    module: null,
    node: null,
    objectUrl: null,
    sourceLabel: "",
    descriptor: null,
    plugins: [],
    pluginId: null,
    params: [],
    latencySamples: 0,
    uiSize: null,
    bypass: false,
    loading: false,
    error: "",
  });
  reconnectGraph();
}

function loadAudioFile(file) {
  if (!isAudioFile(file)) {
    setStatus("Unsupported audio file", true);
    return;
  }
  if (currentAudioObjectUrl) URL.revokeObjectURL(currentAudioObjectUrl);
  currentAudioObjectUrl = URL.createObjectURL(file);
  audioElement.src = currentAudioObjectUrl;
  audioElement.loop = loopToggle.checked;
  audioFileName.textContent = file.name;
  setSourceMode(SOURCE_FILE);
  updateTimeUi();
  setStatus(`Loaded audio: ${file.name}`);
}

function renderSlots() {
  slotsRoot.replaceChildren();
  for (const slot of slots) {
    const isEmpty = !slot.node && !slot.loading && !slot.error;
    const root = document.createElement("article");
    root.className = "slot";
    root.classList.toggle("selected", slot.index === selectedSlot);
    root.classList.toggle("empty", isEmpty);
    root.classList.toggle("loading", slot.loading);
    root.classList.toggle("bypassed", slot.bypass);
    root.classList.toggle("error-state", Boolean(slot.error));
    root.dataset.slot = String(slot.index);
    root.tabIndex = 0;

    const index = document.createElement("span");
    index.className = "slot-index";
    index.textContent = String(slot.index + 1);

    const body = document.createElement("div");
    const name = document.createElement("p");
    name.className = "slot-name";
    name.textContent = slot.loading
      ? "Loading..."
      : slot.error
        ? "Load failed"
        : slot.descriptor?.name || EMPTY_SLOT_NAME;
    if (slot.error) name.classList.add("error");
    const meta = document.createElement("p");
    meta.className = "slot-meta";
    meta.textContent = slot.error || (isEmpty ? EMPTY_SLOT_META : slotSummary(slot));
    const tags = document.createElement("div");
    tags.className = "slot-tags";
    if (isEmpty) {
      tags.append(slotTag("pass-through"));
    } else {
      tags.append(slotTag(pluginFlow(slot)));
      if (slot.bypass) tags.append(slotTag("bypassed", "warn"));
    }
    body.append(name, meta, tags);

    const actions = document.createElement("div");
    actions.className = "slot-actions";
    actions.append(
      slotButton("ui", "Open the plugin UI in its own window", !slot.node?.openInterface, () => togglePluginWindow(slot)),
      slotButton("auto", "Open generated parameter controls", !slot.node, () => toggleAutoPanel(slot)),
      slotButton("save", "Copy plugin state to clipboard", !slot.node?.saveState, () => saveSlotState(slot.index)),
      slotButton("load", "Load plugin state from clipboard", !slot.node?.loadState, () => loadSlotState(slot.index)),
      slotButton(slot.bypass ? "byp" : "on", slot.bypass ? "Enable slot" : "Bypass slot", !slot.node, () => toggleSlotBypass(slot)),
      slotButton(`${slot.latencySamples || 0}smp`, "Refresh reported plugin latency", !slot.node, () => refreshSlotLatency(slot.index)),
      slotButton("✕", "Remove plugin", !slot.node && !slot.error, () => removeSlot(slot.index), "danger"),
    );

    root.append(index, body, actions);
    root.addEventListener("click", () => selectSlot(slot.index));
    root.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        selectSlot(slot.index);
      }
    });
    root.addEventListener("dragover", (event) => {
      if (!hasPluginDrop(event)) return;
      event.preventDefault();
      root.classList.add("drag-over");
    });
    root.addEventListener("dragleave", () => root.classList.remove("drag-over"));
    root.addEventListener("drop", async (event) => {
      if (!hasPluginDrop(event)) return;
      event.preventDefault();
      event.stopPropagation();
      clearDropState();
      await handlePluginDrop(event, slot.index);
    });

    slotsRoot.append(root);
  }
}

function slotTag(text, tone = "") {
  const tag = document.createElement("span");
  tag.className = ["slot-tag", tone].filter(Boolean).join(" ");
  tag.textContent = text;
  return tag;
}

function slotButton(text, title, disabled, action, tone = "") {
  const button = document.createElement("button");
  button.type = "button";
  button.className = ["slot-button", tone].filter(Boolean).join(" ");
  button.textContent = text;
  button.title = title;
  button.disabled = disabled;
  button.addEventListener("click", async (event) => {
    event.stopPropagation();
    try {
      await action();
    } catch (error) {
      setStatus(messageFromError(error), true);
    }
  });
  return button;
}

function toggleSlotBypass(slot) {
  slot.bypass = !slot.bypass;
  reconnectGraph();
  renderSlots();
  setStatus(`${slotDisplayName(slot)} ${slot.bypass ? "bypassed" : "enabled"}`);
}

async function refreshSlotLatency(index) {
  const slot = slots[index];
  if (!slot.node) return;
  slot.latencySamples = await readLatency(slot.node);
  renderSlots();
  setStatus(`${slotDisplayName(slot)} latency: ${slot.latencySamples || 0} samples`);
}

async function removeSlot(index) {
  await clearSlot(index);
  renderSlots();
  renderDetails();
  setStatus(`Slot ${index + 1} cleared`);
}

function selectSlot(index) {
  selectedSlot = index;
  renderSlots();
  renderDetails();
}

function renderDetails() {
  const slot = slots[selectedSlot];
  paramsPanel.replaceChildren();

  if (!slot?.node) {
    paramsPanel.append(emptyState("Select a loaded slot to edit parameters."));
    return;
  }

  renderParams(slot);
}

function renderParams(slot) {
  if (!slot.params.length) {
    paramsPanel.append(emptyState("No parameters exposed."));
    return;
  }

  paramsPanel.append(createParamList(slot));
}

function createParamList(slot) {
  const list = document.createElement("div");
  list.className = "param-list";
  for (const param of slot.params) {
    const row = document.createElement("label");
    row.className = "param-row";

    const name = document.createElement("span");
    name.textContent = param.name || String(param.id);

    const input = document.createElement("input");
    input.type = "range";
    input.min = finiteNumber(param.min, 0);
    input.max = finiteNumber(param.max, 1);
    input.step = param.flags?.stepped ? "1" : "0.000001";

    const output = document.createElement("output");
    const writeValue = async () => {
      try {
        const value = await withTimeout(
          slot.node.setParam(param.id, Number(input.value)),
          REMOTE_CALL_TIMEOUT_MS,
          "parameter write timed out",
        );
        output.textContent = value?.text || Number(input.value).toFixed(3);
      } catch (error) {
        output.textContent = "error";
        setStatus(`Param write failed: ${messageFromError(error)}`, true);
      }
    };
    const readValue = async () => {
      try {
        const value = await withTimeout(
          slot.node.getParam(param.id),
          REMOTE_CALL_TIMEOUT_MS,
          "parameter read timed out",
        );
        const numeric = finiteNumber(value?.value, finiteNumber(param.default, Number(input.min)));
        input.value = String(numeric);
        output.textContent = value?.text || numeric.toFixed(3);
      } catch (error) {
        output.textContent = "error";
      }
    };

    input.addEventListener("input", writeValue);
    input.addEventListener("dblclick", () => {
      input.value = String(finiteNumber(param.default, Number(input.min)));
      writeValue();
    });
    readValue();
    row.append(name, input, output);
    list.append(row);
  }
  return list;
}

// --- Plugin UI windows ------------------------------------------------------
//
// Each plugin UI opens in its own browser window, sized from the bundle's
// plugin.json `ui` block (WebCLAP-defined). The iframe returned by
// `openInterface` is adopted into the popup document; resource fetches keep
// flowing because the service-worker proxy resolves through `frameResolvers`
// (registered against this host page), and UI→plugin postMessages are relayed
// from the popup window back onto this window where clap-audionode listens.

function togglePluginWindow(slot) {
  const open = pluginWindows.get(slot.index);
  if (open && !open.win.closed) {
    closePluginWindow(slot.index);
    return;
  }
  openPluginWindow(slot);
}

function openPluginWindow(slot) {
  if (!pageProxy) {
    setStatus("Plugin UI proxy is not ready.", true);
    return;
  }
  if (!slot.node?.openInterface) {
    setStatus(`${slotDisplayName(slot)} does not expose a WebCLAP UI.`, true);
    return;
  }
  closePluginWindow(slot.index);

  const size = slot.uiSize || DEFAULT_UI_SIZE;
  const win = window.open(
    "",
    `z-audio-plugin-ui-${slot.index}`,
    `popup=yes,width=${size.width},height=${size.height}`,
  );
  if (!win) {
    setStatus("Plugin window was blocked by the browser popup blocker.", true);
    return;
  }

  const frameId = `plugin-ui-${crypto.randomUUID()}`;
  const iframe = slot.node.openInterface({
    filePrefix: `${pageProxy.prefix}${frameId}/file`,
    resourcePrefix: `${pageProxy.prefix}${frameId}/get_resource`,
  });
  iframe.id = frameId;
  frameResolvers.set(frameId, slot.node);

  const doc = win.document;
  doc.title = slotDisplayName(slot);
  const style = doc.createElement("style");
  style.textContent =
    "html,body{margin:0;padding:0;width:100%;height:100%;overflow:hidden;background:#0d1117}" +
    "iframe{display:block;border:0;width:100%;height:100%}";
  doc.head.append(style);
  doc.body.append(iframe); // cross-document append adopts the iframe

  // clap-audionode listens for UI messages on the host window and checks
  // event.source, so relay ArrayBuffer messages arriving in the popup.
  // The payload must be re-created in this window's realm: a popup-realm
  // ArrayBuffer fails the host's `instanceof ArrayBuffer` checks.
  const relay = (event) => {
    if (event.source !== iframe.contentWindow) return;
    if (Object.prototype.toString.call(event.data) !== "[object ArrayBuffer]") return;
    const data = new Uint8Array(new Uint8Array(event.data)).buffer;
    window.dispatchEvent(new MessageEvent("message", { data, source: event.source }));
  };
  win.addEventListener("message", relay);

  // No manifest size: size the window to the UI document once it loads.
  if (!slot.uiSize) {
    iframe.addEventListener("load", () => {
      try {
        const root = iframe.contentDocument?.documentElement;
        if (!root) return;
        const width = Math.min(Math.max(root.scrollWidth, 320), screen.availWidth);
        const height = Math.min(Math.max(root.scrollHeight, 240), screen.availHeight);
        win.resizeBy(width - win.innerWidth, height - win.innerHeight);
      } catch {
        // Keep the default size if the UI document is not measurable.
      }
    });
  }

  // There is no reliable cross-window close event, so poll `closed` to tear
  // down the interface when the user closes the popup directly.
  const watcher = window.setInterval(() => {
    if (win.closed) closePluginWindow(slot.index);
  }, 500);

  pluginWindows.set(slot.index, { win, iframe, frameId, node: slot.node, relay, watcher });
  setStatus(`${slotDisplayName(slot)} UI opened (${size.width}×${size.height})`);
}

function closePluginWindow(index) {
  const open = pluginWindows.get(index);
  if (!open) return;
  pluginWindows.delete(index);
  window.clearInterval(open.watcher);
  frameResolvers.delete(open.frameId);
  if (open.node?.closeInterface) open.node.closeInterface();
  if (!open.win.closed) {
    open.win.removeEventListener("message", open.relay);
    open.win.close();
  }
}

function closeAllPluginWindows() {
  for (const index of [...pluginWindows.keys()]) closePluginWindow(index);
}

function toggleAutoPanel(slot) {
  const key = `auto-${slot.index}`;
  if (floatingPanelClosers.has(key)) {
    closeFloatingPanel(key);
    return;
  }

  const body = document.createElement("div");
  body.className = "floating-panel-body auto-body";
  if (slot.params.length) {
    body.append(createParamList(slot));
  } else {
    body.append(emptyState("No parameters exposed."));
  }
  openFloatingPanel(key, `${slotDisplayName(slot)} auto`, body, "auto-panel");
}

function openFloatingPanel(key, title, body, extraClass = "", onClose = null) {
  closeFloatingPanel(key);
  const panel = document.createElement("section");
  panel.className = ["floating-panel", extraClass].filter(Boolean).join(" ");
  panel.dataset.panelKey = key;

  const header = document.createElement("div");
  header.className = "floating-panel-header";
  const heading = document.createElement("h2");
  heading.textContent = title;
  const close = document.createElement("button");
  close.type = "button";
  close.textContent = "✕";
  close.title = "Close";
  header.append(heading, close);
  panel.append(header, body);

  const cleanup = () => {
    if (floatingPanelClosers.get(key)?.panel !== panel) return;
    onClose?.();
    panel.remove();
    floatingPanelClosers.delete(key);
  };
  close.addEventListener("click", cleanup);
  floatingPanelClosers.set(key, { panel, close: cleanup });
  floatingPanels.append(panel);
  return panel;
}

function closeFloatingPanel(key) {
  floatingPanelClosers.get(key)?.close();
}

function closeSlotFloatingPanels(index) {
  closeFloatingPanel(`auto-${index}`);
}

async function saveSlotState(index) {
  const slot = slots[index];
  if (!slot.node?.saveState) return;
  const state = await withTimeout(slot.node.saveState(), REMOTE_CALL_TIMEOUT_MS, "state save timed out");
  if (!state) {
    setStatus(`${slotDisplayName(slot)} did not return state`, true);
    return;
  }
  const text = bytesToBase64(state);
  await writeClipboardText(text);
  setStatus(`${slotDisplayName(slot)} state copied to clipboard`);
}

async function loadSlotState(index) {
  const slot = slots[index];
  if (!slot.node?.loadState) return;
  const text = await readClipboardText("Paste WebCLAP state base64");
  if (!text) {
    setStatus("No state text provided", true);
    return;
  }

  let state;
  try {
    state = base64ToArrayBuffer(text);
  } catch (error) {
    setStatus(`State text is not valid base64: ${messageFromError(error)}`, true);
    return;
  }

  const ok = await withTimeout(slot.node.loadState(state), REMOTE_CALL_TIMEOUT_MS, "state load timed out");
  if (ok === false) {
    setStatus(`${slotDisplayName(slot)} rejected state`, true);
    return;
  }
  slot.params = await readParams(slot.node);
  renderDetails();
  if (floatingPanelClosers.has(`auto-${slot.index}`)) {
    closeFloatingPanel(`auto-${slot.index}`);
    toggleAutoPanel(slot);
  }
  setStatus(`${slotDisplayName(slot)} state loaded`);
}

function clearDropState() {
  dragDepth = 0;
  document.body.classList.remove("drag-over");
  audioDrop.classList.remove("drag-over");
  document.querySelectorAll(".slot.drag-over").forEach((node) => node.classList.remove("drag-over"));
}

async function handlePageDrop(event) {
  const file = firstFile(event);
  const shelfUrl = event.dataTransfer.getData("text/x-z-audio-wclap-url");
  if (shelfUrl) {
    event.preventDefault();
    await loadPluginUrl(firstAvailableSlot(), shelfUrl, event.dataTransfer.getData("text/plain") || shelfUrl);
    return;
  }
  if (!file) return;
  event.preventDefault();
  if (isPluginFile(file)) {
    await loadPluginFile(firstAvailableSlot(), file);
  } else if (isAudioFile(file)) {
    loadAudioFile(file);
  } else {
    setStatus(`Unsupported file: ${file.name}`, true);
  }
}

async function handlePluginDrop(event, slotIndex) {
  const shelfUrl = event.dataTransfer.getData("text/x-z-audio-wclap-url");
  if (shelfUrl) {
    await loadPluginUrl(slotIndex, shelfUrl, event.dataTransfer.getData("text/plain") || shelfUrl);
    return;
  }
  const file = firstFile(event);
  if (!file) return;
  if (!isPluginFile(file)) {
    setStatus(`Drop a .wclap or .wclap.tar.gz file on slot ${slotIndex + 1}`, true);
    return;
  }
  await loadPluginFile(slotIndex, file);
}

function firstAvailableSlot() {
  const empty = slots.find((slot) => !slot.node && !slot.loading);
  return empty?.index ?? selectedSlot;
}

function hasUsefulDrop(event) {
  return hasPluginDrop(event) || hasAudioFile(event);
}

function hasPluginDrop(event) {
  if (event.dataTransfer.types.includes("text/x-z-audio-wclap-url")) return true;
  return Array.from(event.dataTransfer.items || []).some((item) => {
    const file = item.kind === "file" ? item.getAsFile() : null;
    return file ? isPluginFile(file) : true;
  });
}

function hasAudioFile(event) {
  return Array.from(event.dataTransfer.items || []).some((item) => {
    if (item.kind !== "file") return false;
    const file = item.getAsFile();
    return file ? isAudioFile(file) : item.type.startsWith("audio/");
  });
}

function firstFile(event) {
  return event.dataTransfer.items?.[0]?.getAsFile?.() || event.dataTransfer.files?.[0] || null;
}

function isPluginFile(file) {
  const name = file.name.toLowerCase();
  return name.endsWith(".wclap") || name.endsWith(".wclap.tar.gz") || name.endsWith(".wasm");
}

function isAudioFile(file) {
  return file.type.startsWith("audio/") || /\.(wav|mp3|m4a|aac|ogg|opus|flac|webm)$/i.test(file.name);
}

async function refreshMidiInputs() {
  if (!("requestMIDIAccess" in navigator)) {
    midiStatus.textContent = "Web MIDI unavailable";
    return;
  }

  try {
    midiAccess = await navigator.requestMIDIAccess({ sysex: false });
    midiAccess.onstatechange = populateMidiInputs;
    populateMidiInputs();
    midiStatus.textContent = midiAccess.inputs.size ? "MIDI ready" : "No MIDI inputs";
  } catch (error) {
    midiStatus.textContent = `MIDI denied: ${messageFromError(error)}`;
  }
}

function populateMidiInputs() {
  const selected = midiInputSelect.value;
  midiInputSelect.replaceChildren();
  midiInputSelect.append(new Option("No device", ""));

  if (!midiAccess) return;
  for (const input of midiAccess.inputs.values()) {
    midiInputSelect.append(new Option(input.name || input.id, input.id));
  }
  if ([...midiInputSelect.options].some((option) => option.value === selected)) {
    midiInputSelect.value = selected;
  }
  selectMidiInput();
}

function selectMidiInput() {
  if (activeMidiInput) activeMidiInput.onmidimessage = null;
  activeMidiInput = null;

  if (!midiAccess) {
    midiStatus.textContent = "MIDI idle";
    return;
  }
  if (!midiInputSelect.value) {
    midiStatus.textContent = midiAccess.inputs.size ? "MIDI idle" : "No MIDI inputs";
    return;
  }

  activeMidiInput = midiAccess.inputs.get(midiInputSelect.value) || null;
  if (!activeMidiInput) {
    midiStatus.textContent = "MIDI device missing";
    return;
  }

  activeMidiInput.onmidimessage = handleMidiMessage;
  midiStatus.textContent = `MIDI: ${activeMidiInput.name || activeMidiInput.id}`;
}

function handleMidiMessage(message) {
  const [status, data1, data2] = message.data;
  const command = status & 0xf0;
  const channel = status & 0x0f;
  const key = data1 & 0x7f;
  const velocity = (data2 & 0x7f) / 127;

  if (command === 0x90 && velocity > 0) {
    heldMidiNotes.add(`${channel}:${key}`);
    sendMidiNote(CLAP_EVENT_NOTE_ON, key, velocity, channel);
  } else if (command === 0x80 || command === 0x90) {
    heldMidiNotes.delete(`${channel}:${key}`);
    sendMidiNote(CLAP_EVENT_NOTE_OFF, key, 0, channel);
  }
}

function sendAllNotesOff() {
  for (const note of heldMidiNotes) {
    const [channel, key] = note.split(":").map(Number);
    sendMidiNote(CLAP_EVENT_NOTE_OFF, key, 0, channel);
  }
  heldMidiNotes.clear();
}

function sendMidiNote(type, key, velocity, channel) {
  const target = firstMidiTarget();
  if (!target?.pushEvents) {
    midiStatus.textContent = "No MIDI target in chain";
    return;
  }

  const event = encodeClapNoteEvent(type, key, velocity, channel);
  target.pushEvents([event]).catch((error) => {
    midiStatus.textContent = `MIDI send failed: ${messageFromError(error)}`;
  });
}

function firstMidiTarget() {
  return slots.find((slot) => slot.node && !slot.bypass && acceptsMidiInput(slot))?.node || null;
}

function encodeClapNoteEvent(type, key, velocity, channel) {
  const buffer = new ArrayBuffer(CLAP_NOTE_EVENT_SIZE);
  const view = new DataView(buffer);
  view.setUint32(0, CLAP_NOTE_EVENT_SIZE, true);
  view.setUint32(4, 0, true);
  view.setUint16(8, 0, true);
  view.setUint16(10, type, true);
  view.setUint32(12, 0, true);
  view.setInt32(16, -1, true);
  view.setInt16(20, 0, true);
  view.setInt16(22, channel, true);
  view.setInt16(24, key, true);
  view.setFloat64(32, velocity, true);
  return buffer;
}

function updateSignalUi() {
  const option = signalKind.selectedOptions[0];
  signalKindReadout.textContent = option?.textContent || signalKind.value;
  const isNoise = isNoiseKind(signalKind.value);
  toneFrequency.disabled = isNoise;
  toneFrequencyReadout.textContent = isNoise ? "-" : `${Math.round(Number(toneFrequency.value))} Hz`;
}

function isNoiseKind(kind) {
  return kind === "white" || kind === "pink" || kind === "brown";
}

function createNoiseBuffer(kind, sampleRate) {
  const length = Math.max(1, Math.floor(sampleRate * 2));
  const buffer = audioContext.createBuffer(1, length, sampleRate);
  const data = buffer.getChannelData(0);

  if (kind === "white") {
    for (let i = 0; i < length; i++) data[i] = Math.random() * 2 - 1;
    return buffer;
  }

  if (kind === "brown") {
    let last = 0;
    for (let i = 0; i < length; i++) {
      last = (last + 0.02 * (Math.random() * 2 - 1)) / 1.02;
      data[i] = last * 3.5;
    }
    return buffer;
  }

  let b0 = 0;
  let b1 = 0;
  let b2 = 0;
  let b3 = 0;
  let b4 = 0;
  let b5 = 0;
  let b6 = 0;
  for (let i = 0; i < length; i++) {
    const white = Math.random() * 2 - 1;
    b0 = 0.99886 * b0 + white * 0.0555179;
    b1 = 0.99332 * b1 + white * 0.0750759;
    b2 = 0.96900 * b2 + white * 0.1538520;
    b3 = 0.86650 * b3 + white * 0.3104856;
    b4 = 0.55000 * b4 + white * 0.5329522;
    b5 = -0.7616 * b5 - white * 0.0168980;
    data[i] = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362) * 0.11;
    b6 = white * 0.115926;
  }
  return buffer;
}

function slotSummary(slot) {
  if (!slot.node) return slot.sourceLabel || "Empty slot passes through";
  const parts = [pluginFlow(slot)];
  if (slot.descriptor?.vendor) parts.push(slot.descriptor.vendor);
  if (slot.sourceLabel) parts.push(slot.sourceLabel);
  return parts.filter(Boolean).join(" / ");
}

function pluginFlow(slot) {
  const features = pluginFeatures(slot);
  if (features.includes("note-effect")) return "MIDI -> MIDI";
  if (features.includes("instrument")) return "MIDI -> Audio";
  if (features.includes("note-detector")) return "Audio -> MIDI";
  if (features.includes("audio-effect")) return "Audio -> Audio";
  return "Audio + MIDI";
}

function usesAudioLane(slot) {
  const features = pluginFeatures(slot);
  if (!features.length) return true;
  return features.includes("audio-effect") || features.includes("instrument") || features.includes("note-detector");
}

function usesEventLane(slot) {
  const features = pluginFeatures(slot);
  return (
    features.includes("instrument") ||
    features.includes("note-effect") ||
    features.includes("note-detector")
  );
}

function acceptsMidiInput(slot) {
  const features = pluginFeatures(slot);
  return features.includes("instrument") || features.includes("note-effect");
}

function pluginFeatures(slot) {
  return slot.descriptor?.features || slot.plugins?.[0]?.features || [];
}

function updateTimeUi() {
  const duration = finiteDuration();
  const current = Number.isFinite(audioElement.currentTime) ? audioElement.currentTime : 0;
  seekSlider.value = duration > 0 ? String(current / duration) : "0";
  timeReadout.textContent = `${formatTime(current)} / ${formatTime(duration)}`;
}

function finiteDuration() {
  return Number.isFinite(audioElement.duration) ? audioElement.duration : 0;
}

function startMeters() {
  if (animationFrame) return;
  const lData = new Float32Array(analyserL.fftSize);
  const rData = new Float32Array(analyserR.fftSize);
  const tick = () => {
    analyserL.getFloatTimeDomainData(lData);
    analyserR.getFloatTimeDomainData(rData);
    meterL.value = rms(lData);
    meterR.value = rms(rData);
    animationFrame = requestAnimationFrame(tick);
  };
  tick();
}

function startCpuTimer() {
  if (cpuTimer) return;
  cpuTimer = window.setInterval(async () => {
    const active = slots.filter((slot) => slot.node && !slot.bypass);
    if (!active.length) {
      cpuReadout.textContent = "-";
      return;
    }
    const values = [];
    for (const slot of active) {
      try {
        const perf = await slot.node.performance();
        values.push((perf.js / perf.block) * 100);
      } catch {
        // Ignore per-plugin performance read failures.
      }
    }
    cpuReadout.textContent = values.length
      ? `${values.reduce((a, b) => a + b, 0).toFixed(1)}%`
      : "-";
  }, 1000);
}

function rms(samples) {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.min(1, Math.sqrt(sum / samples.length) * 2.4);
}

function bytesToBase64(state) {
  const bytes = bytesFromState(state);
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

function base64ToArrayBuffer(text) {
  const clean = text.trim().replace(/^data:[^,]+,/, "").replace(/\s+/g, "");
  const binary = atob(clean);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

function bytesFromState(state) {
  if (state instanceof ArrayBuffer) return new Uint8Array(state);
  if (ArrayBuffer.isView(state)) return new Uint8Array(state.buffer, state.byteOffset, state.byteLength);
  return new Uint8Array(state);
}

async function writeClipboardText(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.append(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  textarea.remove();
  if (!copied) throw new Error("clipboard write unavailable");
}

async function readClipboardText(promptText) {
  if (navigator.clipboard?.readText) return navigator.clipboard.readText();
  return window.prompt(promptText, "") || "";
}

function slotDisplayName(slot) {
  return slot.descriptor?.name || slot.sourceLabel || `Slot ${slot.index + 1}`;
}

function labelFromBundleUrl(url) {
  const fileName = decodeURIComponent((url.split("/").pop() || url).split("?", 1)[0]);
  return fileName
    .replace(/\.wclap\.tar\.gz$/i, "")
    .replace(/^z-audio-/i, "")
    .split("-")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function dbToGain(db) {
  return 10 ** (db / 20);
}

function formatTime(seconds) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0:00";
  const minutes = Math.floor(seconds / 60);
  const rest = Math.floor(seconds % 60).toString().padStart(2, "0");
  return `${minutes}:${rest}`;
}

function finiteNumber(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function withTimeout(promise, timeoutMs, message) {
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      setTimeout(() => reject(new Error(message)), timeoutMs);
    }),
  ]);
}

function tryDisconnect(node) {
  if (!node) return;
  try {
    node.disconnect();
  } catch {
    // AudioNode.disconnect() throws when there is no current connection.
  }
}

function tryDisconnectEvents(node) {
  if (!node?.disconnectEvents) return;
  node.disconnectEvents(null).catch(() => {});
}

function emptyState(text) {
  const p = document.createElement("p");
  p.className = "empty-state";
  p.textContent = text;
  return p;
}

function setStatus(text, isError = false) {
  statusLine.textContent = text;
  statusLine.classList.toggle("error", isError);
}

function messageFromError(error) {
  return error?.message || String(error);
}
