// Onset (slice-point) estimation for the sampler's Slice mode.
//
// Pipeline (the standard spectral-flux recipe, tuned for sampler slicing):
//
//   1. STFT of the mono mix — Hann window 1024, hop 256 (≈5.8 ms @44.1k).
//   2. Log-compressed magnitudes, half-wave-rectified frame-to-frame
//      difference summed over bins = spectral flux. Unlike a plain energy
//      detector this also fires on pitch/timbre changes with no level dip
//      (e.g. slurred bass notes, chord changes).
//   3. Adaptive threshold: a median of the surrounding flux plus a floor
//      derived from the file's own flux distribution. Sensitivity scales
//      both, so quiet ghost notes appear as the slider goes up.
//   4. Peak-picking with a local-maximum window and a minimum inter-onset
//      gap.
//   5. Sample-accurate refinement: each coarse onset is moved to the start
//      of its attack (where the fine envelope first rises above a fraction
//      of the local peak), then snapped to the quietest nearby sample so
//      slices start clean instead of mid-transient.
//
// computeOnsetCurve() is the expensive part and depends only on the audio,
// so callers cache it per file; pickOnsets()/refineOnset() are cheap and
// re-run freely as the sensitivity slider or trim markers move.

"use strict";

export const ONSET_WINDOW = 1024;
export const ONSET_HOP = 256;

// ---------------------------------------------------------------------------
// FFT — iterative radix-2, in-place, complex interleaved-free (separate
// re/im arrays). Plenty fast for 1024-point frames.
// ---------------------------------------------------------------------------

function makeFft(n) {
  const levels = Math.log2(n);
  if (!Number.isInteger(levels)) throw new Error("FFT size must be a power of 2");
  const cos = new Float32Array(n / 2);
  const sin = new Float32Array(n / 2);
  for (let i = 0; i < n / 2; i++) {
    cos[i] = Math.cos((2 * Math.PI * i) / n);
    sin[i] = Math.sin((2 * Math.PI * i) / n);
  }
  const rev = new Uint32Array(n);
  for (let i = 0; i < n; i++) {
    let r = 0;
    for (let b = 0; b < levels; b++) r = (r << 1) | ((i >>> b) & 1);
    rev[i] = r;
  }
  return function fft(re, im) {
    for (let i = 0; i < n; i++) {
      const j = rev[i];
      if (j > i) {
        let t = re[i];
        re[i] = re[j];
        re[j] = t;
        t = im[i];
        im[i] = im[j];
        im[j] = t;
      }
    }
    for (let size = 2; size <= n; size *= 2) {
      const half = size / 2;
      const step = n / size;
      for (let i = 0; i < n; i += size) {
        for (let j = i, k = 0; j < i + half; j++, k += step) {
          const l = j + half;
          const tre = re[l] * cos[k] + im[l] * sin[k];
          const tim = im[l] * cos[k] - re[l] * sin[k];
          re[l] = re[j] - tre;
          im[l] = im[j] - tim;
          re[j] += tre;
          im[j] += tim;
        }
      }
    }
  };
}

// ---------------------------------------------------------------------------
// Onset strength curve.
// ---------------------------------------------------------------------------

/**
 * Computes the spectral-flux onset-strength curve of `mono`.
 * Returns `{ flux, hop, p95 }` where `flux[n]` covers samples
 * `n*hop .. n*hop+window`. Cache this per file — it only depends on the
 * audio, not on sensitivity or trim.
 */
export function computeOnsetCurve(mono, _sampleRate) {
  const n = ONSET_WINDOW;
  const hop = ONSET_HOP;
  const bins = n / 2;
  const frameCount = Math.max(0, Math.floor((mono.length - n) / hop) + 1);
  const flux = new Float32Array(frameCount);
  if (frameCount === 0) return { flux, hop, p95: 0 };

  const fft = makeFft(n);
  const window = new Float32Array(n);
  for (let i = 0; i < n; i++) window[i] = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (n - 1));

  const re = new Float32Array(n);
  const im = new Float32Array(n);
  let prev = new Float32Array(bins);
  let curr = new Float32Array(bins);

  for (let f = 0; f < frameCount; f++) {
    const at = f * hop;
    for (let i = 0; i < n; i++) {
      re[i] = mono[at + i] * window[i];
      im[i] = 0;
    }
    fft(re, im);
    let sum = 0;
    for (let k = 1; k < bins; k++) {
      // Log compression keeps loud hits from drowning out quiet ones.
      const mag = Math.log1p(10 * Math.hypot(re[k], im[k]));
      curr[k] = mag;
      const d = mag - prev[k];
      if (d > 0) sum += d;
    }
    flux[f] = sum / bins;
    const swap = prev;
    prev = curr;
    curr = swap;
  }
  // First frame's flux compares against silence — that's the legitimate
  // "sound starts" onset, but scale it like its neighbours.
  if (frameCount > 1) flux[0] = Math.min(flux[0], flux[1] * 2);

  return { flux, hop };
}

/// A flux frame covers samples `f*hop .. f*hop+window`; its value peaks on
/// the first frames whose window newly contains the event, i.e. the event
/// sits near the *end* of the window. This offset maps a peak frame back
/// to the event's approximate sample position (measured empirically at
/// ~window-hop; percussive cuts are then refined sample-accurately).
const FRAME_EVENT_OFFSET = ONSET_WINDOW - ONSET_HOP;

// ---------------------------------------------------------------------------
// Peak picking.
// ---------------------------------------------------------------------------

/**
 * Picks coarse onset positions (in samples) from a cached curve.
 * `sensitivity` 0..1: low = only strong hits, high = ghost notes too.
 * Only onsets inside `[startFrame, endFrame)` are returned.
 */
export function pickOnsets(curve, sampleRate, startFrame, endFrame, sensitivity, maxCount = 128) {
  const { flux, hop } = curve;
  if (flux.length === 0) return [];
  const sens = Math.min(1, Math.max(0, sensitivity));

  // Onsets near the trim start still peak in frames whose window starts a
  // little before it, so widen the frame range by the event offset.
  const lo = Math.max(0, Math.floor((startFrame - FRAME_EVENT_OFFSET) / hop));
  const hi = Math.min(flux.length, Math.ceil(endFrame / hop));

  // The floor is relative to the strongest onset *within the picked
  // range*, so trimming to a quiet section still slices it, while steady
  // material (whose flux is just analysis noise, orders of magnitude below
  // a real hit) produces nothing.
  let pmax = 0;
  for (let f = lo; f < hi; f++) if (flux[f] > pmax) pmax = flux[f];
  if (pmax <= 1e-6) return [];
  const floor = pmax * (0.26 - 0.235 * sens); // 26 % .. 2.5 % of the top hit
  const lambda = 2.6 - 1.6 * sens; // median multiplier, 2.6 .. 1.0
  const medianHalf = 8; // ±8 frames ≈ ±46 ms context
  const peakHalf = 3; // local max over ±3 frames
  const minGap = Math.max(1, Math.round((sampleRate * (0.075 - 0.05 * sens)) / hop)); // 75..25 ms

  const frames = [];
  let last = -Infinity;
  const scratch = [];

  for (let f = lo; f < hi; f++) {
    const v = flux[f];
    if (v <= floor) continue;

    // Local maximum within ±peakHalf.
    let isPeak = true;
    for (let k = Math.max(0, f - peakHalf); k <= Math.min(flux.length - 1, f + peakHalf); k++) {
      if (flux[k] > v) {
        isPeak = false;
        break;
      }
    }
    if (!isPeak) continue;

    // Adaptive threshold: median of surrounding context.
    scratch.length = 0;
    for (let k = Math.max(0, f - medianHalf); k <= Math.min(flux.length - 1, f + medianHalf); k++) {
      scratch.push(flux[k]);
    }
    scratch.sort((a, b) => a - b);
    const med = scratch[Math.floor(scratch.length / 2)];
    if (v < floor + lambda * med) continue;

    if (f - last < minGap) {
      // Within the refractory gap: keep whichever peak is stronger.
      if (frames.length && flux[frames[frames.length - 1]] < v) {
        frames[frames.length - 1] = f;
        last = f;
      }
      continue;
    }
    frames.push(f);
    last = f;
    if (frames.length >= maxCount) break;
  }

  return frames
    .map((f) => f * hop + FRAME_EVENT_OFFSET)
    .filter((s) => s >= startFrame && s < endFrame);
}

// ---------------------------------------------------------------------------
// Sample-accurate refinement.
// ---------------------------------------------------------------------------

const FINE_ENV_SPAN = 24; // samples per fine-envelope point

function fineEnv(mono, at, hi) {
  let peak = 0;
  const stop = Math.min(hi, at + FINE_ENV_SPAN);
  for (let i = at; i < stop; i++) {
    const a = Math.abs(mono[i]);
    if (a > peak) peak = a;
  }
  return peak;
}

/**
 * Moves a coarse onset (STFT-frame resolution) to the start of its attack:
 * find the local amplitude peak just after the onset, walk back to where
 * the fine envelope first drops below 15% of it, then snap to the quietest
 * sample in the immediate neighbourhood so the cut doesn't click.
 */
export function refineOnset(mono, sampleRate, coarse, lo, hi) {
  const searchAhead = Math.round(sampleRate * 0.04);
  const searchBack = Math.round(sampleRate * 0.03);
  const a = Math.max(lo, coarse - searchBack);
  const b = Math.min(hi, coarse + searchAhead);
  if (b - a < 4) return Math.min(Math.max(coarse, lo), hi);

  // Local amplitude peak of this transient.
  let peakAt = a;
  let peak = 0;
  for (let i = a; i < b; i++) {
    const v = Math.abs(mono[i]);
    if (v > peak) {
      peak = v;
      peakAt = i;
    }
  }
  if (peak <= 0) return Math.min(Math.max(coarse, lo), hi);

  // Walk back from the peak to the attack start. If the envelope never
  // drops (legato material — a pitch change with no level dip), keep the
  // coarse position instead of jumping to the search-window edge.
  const rise = peak * 0.15;
  let attack = Math.min(Math.max(coarse, lo), hi - 1);
  for (let i = peakAt; i >= a; i -= FINE_ENV_SPAN) {
    if (fineEnv(mono, i, hi) < rise) {
      attack = i;
      break;
    }
  }

  // Snap to the quietest sample just before the attack (declick).
  let best = attack;
  let bestAbs = Infinity;
  for (let i = Math.max(lo, attack - 96); i <= Math.min(hi - 1, attack + FINE_ENV_SPAN); i++) {
    const v = Math.abs(mono[i]);
    if (v < bestAbs) {
      bestAbs = v;
      best = i;
    }
  }
  return best;
}

// ---------------------------------------------------------------------------
// Top-level slice detection.
// ---------------------------------------------------------------------------

/**
 * Full slice-marker estimation for `[startFrame, endFrame)`. The first
 * marker is always the trim start (slice 1). `curve` may be passed in
 * (cached); otherwise it is computed here.
 */
export function detectSliceMarkers(mono, sampleRate, startFrame, endFrame, sensitivity, curve, maxCount = 128) {
  const c = curve || computeOnsetCurve(mono, sampleRate);
  const coarse = pickOnsets(c, sampleRate, startFrame, endFrame, sensitivity, maxCount);
  const minDistinct = Math.round(sampleRate * 0.02);
  const markers = [startFrame];
  for (const on of coarse) {
    const refined = refineOnset(mono, sampleRate, on, startFrame, endFrame);
    if (refined - markers[markers.length - 1] >= minDistinct && markers.length < maxCount) {
      markers.push(refined);
    }
  }
  return markers;
}
