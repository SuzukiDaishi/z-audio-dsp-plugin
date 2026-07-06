// Unit tests for the slice-point estimator. Run with:
//   node --test crates/z-audio-webclap-sampler/ui/onsets.test.mjs
import test from "node:test";
import assert from "node:assert/strict";

import { computeOnsetCurve, detectSliceMarkers } from "./onsets.js";

const SR = 44100;

function silence(seconds) {
  return new Float32Array(Math.round(SR * seconds));
}

/** Exponentially decaying tone burst starting at `at` (frames). */
function addBurst(buf, at, freq, amp = 0.8, decay = 30) {
  for (let i = at; i < buf.length; i++) {
    const t = (i - at) / SR;
    const env = Math.exp(-t * decay);
    if (env < 1e-4) break;
    buf[i] += amp * env * Math.sin(2 * Math.PI * freq * t);
  }
}

function addNoiseFloor(buf, amp) {
  let s = 1234567;
  for (let i = 0; i < buf.length; i++) {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    buf[i] += amp * (s / 0x3fffffff - 1);
  }
}

function detect(buf, sensitivity = 0.5) {
  return detectSliceMarkers(buf, SR, 0, buf.length, sensitivity);
}

/** True if some marker lands within `tolMs` of `at`. */
function hasMarkerNear(markers, at, tolMs = 12) {
  const tol = (SR * tolMs) / 1000;
  return markers.some((m) => Math.abs(m - at) <= tol);
}

test("drum-like bursts are all found at the right positions", () => {
  const buf = silence(2.2);
  const positions = [];
  for (let i = 0; i < 8; i++) {
    const at = Math.round(SR * (0.1 + i * 0.25));
    positions.push(at);
    addBurst(buf, at, 180 * (1 + (i % 4)), 0.8, 25);
  }
  addNoiseFloor(buf, 0.003);
  const markers = detect(buf);
  // First marker is the trim start; every burst must have a marker close by.
  for (const at of positions) {
    assert.ok(hasMarkerNear(markers, at), `missing onset near ${at}`);
  }
  // No more than trim start + 8 bursts (no doubles / ghosts).
  assert.ok(markers.length <= 9, `too many markers: ${markers.length}`);
});

test("velocity range: quiet hits appear as sensitivity rises", () => {
  const buf = silence(2.0);
  const loud = [0.1, 0.6, 1.1, 1.6].map((s) => Math.round(SR * s));
  const quiet = [0.35, 0.85, 1.35].map((s) => Math.round(SR * s));
  for (const at of loud) addBurst(buf, at, 200, 0.9, 25);
  for (const at of quiet) addBurst(buf, at, 300, 0.05, 25);
  addNoiseFloor(buf, 0.002);

  const low = detect(buf, 0.15);
  for (const at of loud) assert.ok(hasMarkerNear(low, at), `low sens missed loud hit at ${at}`);

  const high = detect(buf, 0.95);
  for (const at of [...loud, ...quiet]) {
    assert.ok(hasMarkerNear(high, at), `high sens missed hit at ${at}`);
  }
  assert.ok(high.length >= low.length, "sensitivity should be monotonic");
});

test("pitch change with no level dip is detected (spectral onset)", () => {
  // Two legato tones, constant amplitude — an energy detector sees nothing.
  const buf = silence(1.0);
  const change = Math.round(SR * 0.5);
  for (let i = 0; i < buf.length; i++) {
    const t = i / SR;
    const freq = i < change ? 220 : 330;
    buf[i] = 0.5 * Math.sin(2 * Math.PI * freq * t);
  }
  const markers = detect(buf, 0.6);
  assert.ok(hasMarkerNear(markers, change, 25), `no marker near pitch change: ${markers}`);
});

test("steady tone produces no false slices", () => {
  const buf = silence(1.5);
  for (let i = 0; i < buf.length; i++) {
    buf[i] = 0.5 * Math.sin((2 * Math.PI * 220 * i) / SR);
  }
  const markers = detect(buf, 0.5);
  // Only the trim-start marker (plus at most the initial attack at 0).
  assert.ok(markers.length <= 2, `steady tone sliced: ${markers}`);
});

test("refinement lands on the quiet side of the attack, not mid-transient", () => {
  const buf = silence(1.0);
  const at = Math.round(SR * 0.4);
  addBurst(buf, at, 150, 0.9, 20);
  addNoiseFloor(buf, 0.001);
  const markers = detect(buf);
  const near = markers.find((m) => Math.abs(m - at) < SR * 0.012);
  assert.ok(near !== undefined, "onset not found");
  // The cut must not be after the attack has already grown loud.
  assert.ok(Math.abs(buf[near]) < 0.2, `cut lands on |x|=${Math.abs(buf[near])}`);
  assert.ok(near <= at + SR * 0.003, "cut should not chop into the transient");
});

test("markers stay inside the trim range and start at trim start", () => {
  const buf = silence(2.0);
  for (const s of [0.2, 0.7, 1.2, 1.7]) addBurst(buf, Math.round(SR * s), 250, 0.8, 25);
  const start = Math.round(SR * 0.5);
  const end = Math.round(SR * 1.5);
  const markers = detectSliceMarkers(buf, SR, start, end, 0.5);
  assert.equal(markers[0], start);
  for (const m of markers) {
    assert.ok(m >= start && m < end, `marker ${m} outside [${start}, ${end})`);
  }
  assert.ok(hasMarkerNear(markers, Math.round(SR * 0.7)));
  assert.ok(hasMarkerNear(markers, Math.round(SR * 1.2)));
});

test("empty and tiny buffers do not crash", () => {
  assert.deepEqual(detect(new Float32Array(0)), [0]);
  assert.deepEqual(detect(new Float32Array(64)), [0]);
});

test("onset curve is reusable across sensitivity picks (cache path)", () => {
  const buf = silence(1.0);
  addBurst(buf, Math.round(SR * 0.3), 200, 0.8, 25);
  const curve = computeOnsetCurve(buf, SR);
  const a = detectSliceMarkers(buf, SR, 0, buf.length, 0.3, curve);
  const b = detectSliceMarkers(buf, SR, 0, buf.length, 0.3);
  assert.deepEqual(a, b);
});
