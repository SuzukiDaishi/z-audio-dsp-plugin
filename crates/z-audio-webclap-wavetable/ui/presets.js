// Z Audio Wave Synth — factory presets.
//
// Every preset is a diff against Init (all params at their declared
// defaults); applyPreset in main.js resets first, then overlays `set`.
//
// FORMAT CONTRACT — parsed by the src/lib.rs test `factory_presets_are_valid`:
//   * every param pair is written `NNN: <number>` — a 3-digit id, a colon,
//     one space, then a plain numeric literal (optional leading -, decimal
//     point allowed; no exponents, no expressions)
//   * pairs appear only inside `set: {` ... `}` blocks, and a set block
//     contains no nested braces
//   * preset/group names never contain a colon or the sequence digits+colon
// Param id map lives in src/params.rs (web ids 500-607).

"use strict";

export const PRESET_GROUPS = [
  {
    name: "Init",
    presets: [
      { name: "Init", set: {} },
    ],
  },
  {
    name: "Bass",
    presets: [
      {
        // Classic LFO-wobble dubstep bass on the Growl table.
        name: "Wub Machine",
        set: {
          511: 5, 512: 0.35, 513: -1,
          551: 1, 552: 350, 553: 0.55,
          570: 0, 571: 2,
          580: 2, 581: 9, 582: 0.6,
          583: 2, 584: 1, 585: 0.3,
          604: 1, 605: 0, 606: 0.45, 607: 0.8,
        },
      },
      {
        // FM growl over a synced saw layer through formant vowels.
        name: "Talking Reese",
        set: {
          511: 6, 512: 0.4, 513: -1, 523: 7, 524: 0.5,
          530: 1, 531: 7, 532: 0.2, 533: -1, 542: 0.55,
          551: 7, 552: 600, 553: 0.4,
          575: 6,
          580: 3, 581: 1, 582: 0.5,
          583: 1, 584: 5, 585: 0.4,
          604: 1, 605: 0, 606: 0.35, 607: 1,
        },
      },
      {
        // Snarling throat bass through a keytracked comb.
        name: "Neuro Snarl",
        set: {
          511: 15, 512: 0.3, 513: -1, 516: 3, 517: 0.2, 523: 2, 524: 0.5,
          551: 5, 552: 110, 553: 0.6, 555: 1,
          565: 0.002, 566: 0.4, 567: 0.1,
          580: 1, 581: 1, 582: 0.7,
          583: 1, 584: 12, 585: 0.4,
          604: 1, 605: 1, 606: 0.4, 607: 0.8,
        },
      },
      {
        // Vowel-table bite with sine-shaped drive.
        name: "Vowel Bite",
        set: {
          511: 4, 512: 0.25, 513: -1,
          551: 7, 552: 900, 553: 0.5,
          571: 4.5,
          580: 2, 581: 1, 582: 0.6,
          604: 1, 605: 3, 606: 0.5, 607: 0.7,
        },
      },
      {
        // Clean saturated sub, morph adds weight.
        name: "Solid Sub",
        set: {
          511: 31, 512: 0.6, 513: -1,
          551: 0, 552: 800, 553: 0.15,
          563: 0.25,
        },
      },
      {
        // Punchy FM bass with an envelope-kicked index.
        name: "FM Knuckle",
        set: {
          511: 20, 512: 0.2, 513: -1,
          551: 1, 552: 500, 553: 0.3,
          565: 0.002, 566: 0.35, 567: 0,
          580: 1, 581: 1, 582: 0.7,
          604: 1, 605: 0, 606: 0.3, 607: 1,
        },
      },
      {
        // Wide detuned grit + octave-stack layer, folded.
        name: "Grit Reese",
        set: {
          511: 8, 512: 0.3, 513: -1, 516: 6, 517: 0.4, 518: 0.9,
          530: 1, 531: 9, 532: 0.1, 533: -1, 542: 0.5,
          551: 1, 552: 300, 553: 0.25,
          571: 0.8,
          580: 2, 581: 9, 582: 0.35,
          604: 1, 605: 2, 606: 0.5, 607: 0.7,
        },
      },
      {
        // VOSIM squeeze with sample-and-hold stutter.
        name: "Squelch Step",
        set: {
          511: 18, 512: 0.4, 513: -1, 523: 5, 524: 0.6,
          551: 3, 552: 400, 553: 0.6,
          570: 4, 571: 5,
          580: 2, 581: 9, 582: 0.6,
          583: 1, 584: 1, 585: 0.5,
          604: 1, 605: 0, 606: 0.3, 607: 0.8,
        },
      },
      {
        // Bent octave-stack rave bass.
        name: "Hoover Down",
        set: {
          511: 9, 512: 0.8, 513: -1, 516: 5, 517: 0.35, 523: 1, 524: 0.6,
          551: 1, 552: 900, 553: 0.3,
          571: 3,
          580: 2, 581: 12, 582: 0.3,
          604: 1, 605: 1, 606: 0.35, 607: 0.7,
        },
      },
      {
        // Growl ring-modulated by a silent fifth-up sine.
        name: "Ring Growler",
        set: {
          511: 5, 512: 0.5, 513: -1, 523: 8, 524: 0.8,
          530: 1, 534: 7, 542: 0,
          551: 7, 552: 700, 553: 0.45,
          571: 3.5,
          580: 2, 581: 1, 582: 0.5,
          604: 1, 605: 0, 606: 0.4, 607: 0.9,
        },
      },
      {
        // Pulse-train chomp, amplitude-modulated by a sub sine.
        name: "AM Chomp",
        set: {
          511: 12, 512: 0.5, 523: 9, 524: 0.7,
          530: 1, 533: -1, 542: 0,
          551: 0, 552: 1200, 553: 0.35,
          565: 0.002, 566: 0.3,
          580: 1, 581: 9, 582: 0.55,
          604: 1, 605: 0, 606: 0.3, 607: 0.8,
        },
      },
      {
        // Bit-crushed, phase-quantized digital bass.
        name: "Quantum Bass",
        set: {
          511: 16, 512: 0.5, 513: -1, 523: 6, 524: 0.5,
          551: 1, 552: 600, 553: 0.3,
          571: 2.5,
          580: 2, 581: 1, 582: 0.4,
          604: 1, 605: 4, 606: 0.35, 607: 0.7,
        },
      },
      {
        // FM-fold table, mirrored phase, warp wobble.
        name: "Mirror Wub",
        set: {
          511: 21, 512: 0.4, 513: -1, 523: 4, 524: 0.5,
          551: 1, 552: 450, 553: 0.4,
          571: 3,
          580: 2, 581: 12, 582: 0.5,
          583: 2, 584: 9, 585: 0.3,
          604: 1, 605: 0, 606: 0.4, 607: 0.9,
        },
      },
      {
        // FM growl through an inverted keytracked comb.
        name: "Comb Growl",
        set: {
          511: 6, 512: 0.55, 513: -1,
          551: 6, 552: 220, 553: 0.55, 555: 1,
          575: 5,
          580: 3, 581: 1, 582: 0.55,
          604: 1, 605: 1, 606: 0.3, 607: 0.7,
        },
      },
      {
        // Sync-saw with a slow prowling notch.
        name: "Notch Prowler",
        set: {
          511: 7, 512: 0.35, 513: -1, 516: 4, 517: 0.3,
          551: 4, 552: 800, 553: 0.5,
          571: 0.4,
          580: 2, 581: 9, 582: 0.7,
          604: 1, 605: 2, 606: 0.35, 607: 0.6,
        },
      },
    ],
  },
  {
    name: "Lead",
    presets: [
      {
        // Wide synced hoover stack.
        name: "Hoover Rave",
        set: {
          511: 9, 512: 0.9, 516: 7, 517: 0.45, 518: 0.85, 523: 3, 524: 0.4,
          503: 0.08,
          551: 1, 552: 5000, 553: 0.2,
          604: 1, 605: 0, 606: 0.2, 607: 0.6,
        },
      },
      {
        // Screaming envelope-swept sync square.
        name: "Sync Scream",
        set: {
          511: 17, 512: 0.1,
          551: 2, 552: 300, 553: 0.3,
          565: 0.002, 566: 0.6, 567: 0.2,
          580: 1, 581: 1, 582: 0.8,
          604: 1, 605: 0, 606: 0.25, 607: 0.7,
        },
      },
      {
        // Slow-PWM analog lead.
        name: "PWM Classic",
        set: {
          511: 1, 512: 0.3, 516: 3, 517: 0.2,
          551: 0, 552: 6000, 553: 0.2,
          571: 0.7,
          580: 2, 581: 1, 582: 0.3,
        },
      },
      {
        // Airy glass lead with vibrato.
        name: "Glass Whistle",
        set: {
          511: 22, 512: 0.2, 513: 1,
          551: 3, 552: 2000, 553: 0.4,
          560: 0.05, 563: 0.5,
          571: 5.5,
          580: 2, 581: 2, 582: 0.05,
        },
      },
      {
        // Folded triangle scream with an envelope on the warp.
        name: "Fold Screamer",
        set: {
          511: 11, 512: 0.7, 523: 1, 524: 0.4,
          551: 1, 552: 4000, 553: 0.3,
          566: 0.5, 567: 0.3,
          580: 1, 581: 12, 582: 0.6,
          604: 1, 605: 2, 606: 0.6, 607: 0.8,
        },
      },
      {
        // Mono talking lead through the formant filter.
        name: "Talk Lead",
        set: {
          501: 1, 503: 0.1,
          511: 14, 512: 0.5,
          551: 7, 552: 1200, 553: 0.5,
          575: 4,
          580: 3, 581: 1, 582: 0.6,
        },
      },
      {
        // Chippy crushed lead.
        name: "Bit Lead",
        set: {
          511: 16, 512: 0.2,
          551: 0, 552: 8000, 553: 0.15,
          560: 0.002, 561: 0.3, 562: 0.6,
          604: 1, 605: 4, 606: 0.5, 607: 0.6,
        },
      },
      {
        // Even/odd morphing lead with an octave shadow.
        name: "Even Glide",
        set: {
          503: 0.06,
          511: 25, 512: 0.2,
          530: 1, 531: 25, 532: 0.8, 534: 12, 542: 0.35,
          551: 0, 552: 5000, 553: 0.2,
          571: 0.5,
          580: 2, 581: 1, 582: 0.5,
        },
      },
      {
        // Vocal-resonance sweep lead.
        name: "Vosim Voice",
        set: {
          511: 18, 512: 0.3, 513: 1,
          551: 3, 552: 1500, 553: 0.5,
          571: 1.2,
          580: 2, 581: 1, 582: 0.4,
        },
      },
      {
        // Bread-and-butter supersaw, amp envelope follows the filter.
        name: "Saw Hero",
        set: {
          503: 0.05,
          511: 0, 512: 0.5, 516: 6, 517: 0.35,
          551: 1, 552: 7000, 553: 0.2,
          580: 6, 581: 9, 582: 0.3,
          604: 1, 605: 0, 606: 0.2, 607: 0.5,
        },
      },
    ],
  },
  {
    name: "Pad",
    presets: [
      {
        // Slow choir with vowel drift.
        name: "Choir Dawn",
        set: {
          511: 13, 512: 0.2, 516: 4, 517: 0.2,
          551: 0, 552: 4000, 553: 0.15,
          560: 0.8, 563: 1.5,
          571: 0.1,
          580: 2, 581: 1, 582: 0.25,
        },
      },
      {
        // Harmonic band drifting through a saw bed.
        name: "Sweep Nebula",
        set: {
          511: 2, 512: 0.3, 516: 5, 517: 0.25,
          551: 0, 552: 5000, 553: 0.2,
          560: 1.2, 563: 2,
          571: 0.08,
          580: 2, 581: 1, 582: 0.4,
        },
      },
      {
        // String-machine ensemble with a slow pan.
        name: "Ensemble 74",
        set: {
          511: 27, 512: 0.4, 516: 6, 517: 0.28, 518: 0.9,
          551: 0, 552: 6000, 553: 0.15,
          560: 0.4, 563: 1.2,
          575: 0.3,
          580: 3, 581: 4, 582: 0.4,
        },
      },
      {
        // Pipe organ plus a breath layer, huge release.
        name: "Cathedral",
        set: {
          511: 26, 512: 0.6,
          530: 1, 531: 29, 532: 0.4, 542: 0.3,
          551: 0, 552: 5000, 553: 0.1,
          560: 0.3, 563: 2.5,
        },
      },
      {
        // Pure air pad over a high-pass floor.
        name: "Breath Pad",
        set: {
          511: 29, 512: 0.3,
          551: 2, 552: 200, 553: 0.2,
          560: 1.2, 563: 2,
          571: 0.15,
          580: 2, 581: 1, 582: 0.5,
        },
      },
      {
        // Metallic haze with a drifting notch and sine sheen.
        name: "Gamelan Haze",
        set: {
          511: 23, 512: 0.5, 513: 1, 516: 3, 517: 0.2,
          551: 4, 552: 1200, 553: 0.4,
          560: 0.9, 563: 2,
          571: 0.12,
          580: 2, 581: 9, 582: 0.5,
          604: 1, 605: 3, 606: 0.2, 607: 0.4,
        },
      },
      {
        // Scanned-noise texture pad.
        name: "Static Drift",
        set: {
          511: 30, 512: 0.2,
          551: 3, 552: 900, 553: 0.4,
          560: 1.5, 563: 2.5,
          571: 0.05, 575: 0.09,
          580: 2, 581: 1, 582: 0.3,
          583: 3, 584: 9, 585: 0.4,
        },
      },
      {
        // Vowel pad blended half-through the formant filter.
        name: "Formant Cloud",
        set: {
          511: 4, 512: 0.5, 516: 4, 517: 0.2,
          551: 7, 552: 800, 553: 0.35, 556: 0.6,
          560: 1, 563: 1.8,
          571: 0.07,
          580: 2, 581: 1, 582: 0.4,
        },
      },
      {
        // Soft square, mirrored, warp breathing slowly.
        name: "Dark Mirror",
        set: {
          511: 10, 512: 0.3, 523: 4, 524: 0.4,
          551: 0, 552: 1800, 553: 0.2,
          560: 0.7, 563: 1.6,
          575: 0.1,
          580: 3, 581: 12, 582: 0.25,
        },
      },
      {
        // Dual-osc evolving bed of Harmonic Sweep and strings.
        name: "Sweep and Swell",
        set: {
          511: 2, 512: 0.1,
          530: 1, 531: 27, 532: 0.6, 535: 8, 542: 0.5,
          551: 0, 552: 4500, 553: 0.15,
          560: 1.4, 563: 2.2,
          571: 0.06, 575: 0.08,
          580: 2, 581: 1, 582: 0.6,
          583: 3, 584: 5, 585: 0.4,
        },
      },
    ],
  },
  {
    name: "Keys",
    presets: [
      {
        // Percussive Hammond registration with a thunk envelope.
        name: "Drawbar Jazz",
        set: {
          511: 24, 512: 0.5,
          560: 0.002, 562: 0.9, 563: 0.08,
          565: 0.002, 566: 0.15, 567: 0,
          580: 1, 581: 1, 582: -0.4,
          604: 1, 605: 0, 606: 0.15, 607: 0.5,
        },
      },
      {
        // Glassy EP — velocity opens the filter.
        name: "EP Glass",
        set: {
          511: 22, 512: 0.1,
          530: 1, 533: -1, 542: 0.4,
          551: 0, 552: 2500, 553: 0.2,
          561: 1.2, 562: 0.4, 563: 0.4,
          580: 4, 581: 9, 582: 0.5,
        },
      },
      {
        // FM-bell electric piano.
        name: "Bell Keys",
        set: {
          511: 19, 512: 0.2,
          551: 0, 552: 6000, 553: 0.15,
          561: 1.5, 562: 0.3, 563: 0.8,
          566: 0.8, 567: 0,
          580: 1, 581: 1, 582: 0.5,
        },
      },
      {
        // Folded-tri reed piano, velocity dynamics.
        name: "Wurli Fold",
        set: {
          511: 11, 512: 0.3,
          551: 0, 552: 3000, 553: 0.2,
          561: 1, 562: 0.5, 563: 0.3,
          580: 4, 581: 3, 582: 0.5,
          604: 1, 605: 0, 606: 0.3, 607: 0.6,
        },
      },
      {
        // Snappy comb clav.
        name: "Clav Comb",
        set: {
          511: 28, 512: 0.15,
          551: 5, 552: 440, 553: 0.5, 555: 1,
          561: 0.6, 562: 0.3, 563: 0.1,
          604: 1, 605: 1, 606: 0.25, 607: 0.5,
        },
      },
      {
        // Buzzy pulse keys with an envelope-swept filter.
        name: "Pulse Piano",
        set: {
          511: 12, 512: 0.4,
          551: 1, 552: 1200, 553: 0.25,
          561: 1, 562: 0.5, 563: 0.5,
          566: 0.6,
          580: 1, 581: 9, 582: 0.6,
        },
      },
      {
        // Full pipe organ with a leslie-ish pan wobble.
        name: "Full Pipes",
        set: {
          511: 26, 512: 0.8,
          560: 0.01, 562: 1, 563: 0.12,
          571: 6,
          580: 2, 581: 4, 582: 0.15,
        },
      },
      {
        // High glassy synced keys, note-tracked brightness.
        name: "Sync Celesta",
        set: {
          511: 17, 512: 0.1, 513: 2,
          551: 0, 552: 8000, 553: 0.1,
          561: 1.5, 562: 0.2, 563: 1.2,
          580: 5, 581: 9, 582: 0.3,
          583: 4, 584: 3, 585: 0.4,
        },
      },
    ],
  },
  {
    name: "Pluck",
    presets: [
      {
        // Plucked-string pop with a keytracked low-pass snap.
        name: "Karplus Pop",
        set: {
          511: 28, 512: 0.5,
          551: 1, 552: 900, 553: 0.3, 555: 1,
          561: 0.5, 562: 0, 563: 0.3,
          565: 0.002, 566: 0.25, 567: 0,
          580: 1, 581: 9, 582: 0.8,
        },
      },
      {
        // Metallic mallet drop.
        name: "Gamelan Drop",
        set: {
          511: 23, 512: 0.2,
          551: 0, 552: 5000, 553: 0.15,
          561: 0.9, 562: 0, 563: 0.6,
          566: 0.2, 567: 0,
          580: 1, 581: 1, 582: -0.5,
        },
      },
      {
        // Classic bell pluck, velocity brightness.
        name: "Bell Pluck",
        set: {
          511: 3, 512: 0.3,
          551: 0, 552: 7000, 553: 0.1,
          561: 0.8, 562: 0, 563: 0.7,
          580: 4, 581: 9, 582: 0.4,
        },
      },
      {
        // Digital grit stab.
        name: "Grit Pluck",
        set: {
          511: 8, 512: 0.2,
          551: 1, 552: 1500, 553: 0.4,
          561: 0.4, 562: 0, 563: 0.2,
          566: 0.15, 567: 0,
          580: 1, 581: 1, 582: 0.6,
          604: 1, 605: 4, 606: 0.25, 607: 0.5,
        },
      },
      {
        // "Yoi" vowel snap pluck.
        name: "Vowel Yoi",
        set: {
          511: 14, 512: 0.7,
          551: 3, 552: 1200, 553: 0.45,
          561: 0.5, 562: 0, 563: 0.25,
          566: 0.2, 567: 0,
          580: 1, 581: 1, 582: -0.7,
        },
      },
      {
        // Quantize-warp zap.
        name: "Quant Pluck",
        set: {
          511: 10, 512: 0.8, 523: 6, 524: 0.6,
          551: 0, 552: 4000, 553: 0.2,
          561: 0.4, 562: 0, 563: 0.2,
          566: 0.18, 567: 0,
          580: 1, 581: 12, 582: -0.6,
        },
      },
      {
        // 808-ish sub pluck with a pitch blip.
        name: "Sub Pluck",
        set: {
          511: 31, 512: 0.4, 513: -1,
          551: 0, 552: 2000, 553: 0.1,
          561: 0.35, 562: 0, 563: 0.3,
          565: 0.002, 566: 0.05, 567: 0,
          580: 1, 581: 2, 582: 0.3,
        },
      },
      {
        // Breathy percussive tick.
        name: "Air Tick",
        set: {
          511: 29, 512: 0.7,
          551: 2, 552: 500, 553: 0.3,
          560: 0.002, 561: 0.4, 562: 0, 563: 0.3,
        },
      },
    ],
  },
  {
    name: "FX",
    presets: [
      {
        // Slow saw-LFO riser through a high-pass.
        name: "Riser Alarm",
        set: {
          511: 2, 512: 0,
          551: 2, 552: 400, 553: 0.4,
          560: 0.5, 562: 1, 563: 1,
          570: 2, 571: 0.15,
          575: 0.12,
          580: 2, 581: 1, 582: 1,
          583: 3, 584: 9, 585: 0.6,
        },
      },
      {
        // Sample-and-hold vowel babble.
        name: "Robot Talk",
        set: {
          511: 14, 512: 0.5,
          551: 7, 552: 900, 553: 0.55,
          570: 4, 571: 6,
          574: 4, 575: 3,
          580: 2, 581: 1, 582: 1,
          583: 3, 584: 9, 585: 0.5,
          604: 1, 605: 1, 606: 0.4, 607: 0.8,
        },
      },
      {
        // Crushed noise bursts.
        name: "Static Storm",
        set: {
          511: 30, 512: 1,
          551: 3, 552: 1500, 553: 0.7,
          570: 4, 571: 12,
          580: 2, 581: 9, 582: 0.8,
          604: 1, 605: 4, 606: 0.6, 607: 0.9,
        },
      },
      {
        // Ring-modulated bell clang.
        name: "Metal Clang",
        set: {
          511: 3, 512: 1, 523: 8, 524: 0.9,
          530: 1, 531: 3, 532: 0.5, 534: 6, 542: 0,
          561: 2, 562: 0, 563: 1.5,
          604: 1, 605: 2, 606: 0.5, 607: 0.8,
        },
      },
      {
        // Slow pitch/pan fly-by.
        name: "Doppler Pass",
        set: {
          511: 27, 512: 0.5,
          551: 1, 552: 3000, 553: 0.3,
          562: 1,
          570: 1, 571: 0.1,
          574: 0, 575: 0.3,
          580: 2, 581: 2, 582: 0.35,
          583: 3, 584: 4, 585: 1,
          586: 2, 587: 9, 588: 0.4,
        },
      },
      {
        // Square-LFO chopped glitch.
        name: "Glitch Gate",
        set: {
          511: 16, 512: 0.6,
          562: 1,
          570: 3, 571: 8,
          574: 4, 575: 4,
          580: 2, 581: 3, 582: -1,
          583: 3, 584: 1, 585: 0.8,
          604: 1, 605: 4, 606: 0.5, 607: 0.8,
        },
      },
      {
        // Detuned low choir wash.
        name: "Ghost Choir",
        set: {
          511: 13, 512: 0.6, 513: -1, 516: 5, 517: 0.3,
          551: 2, 552: 350, 553: 0.3,
          560: 1.8, 563: 2.5,
          571: 0.07,
          580: 2, 581: 1, 582: 0.6,
          604: 1, 605: 3, 606: 0.3, 607: 0.5,
        },
      },
      {
        // Sub-octave FM rumble drone.
        name: "Engine Room",
        set: {
          511: 21, 512: 0.5, 513: -2, 523: 7, 524: 0.6,
          530: 1, 533: -2, 542: 0,
          551: 0, 552: 300, 553: 0.3,
          560: 0.6, 562: 1, 563: 1,
          571: 0.5,
          580: 2, 581: 12, 582: 0.3,
        },
      },
    ],
  },
];
