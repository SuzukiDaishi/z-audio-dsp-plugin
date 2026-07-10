//! Factory wavetables with band-limited mip levels.
//!
//! Each table is `FRAMES` morph frames of `FRAME_LEN` samples, stored at
//! `MIPS` mip levels. Mip `k` contains only harmonics `1..=(MAX_HARMONICS >> k)`,
//! so the oscillator can pick a level whose highest partial stays below
//! Nyquist for the current playback increment — the core of alias-free
//! wavetable playback.
//!
//! Every frame is defined as a harmonic recipe (amplitude + phase per
//! partial) and rendered by additive synthesis. Per-harmonic sines come
//! from a complex-rotation recurrence (two multiply-adds per sample), so
//! table construction never calls `sin()` in the inner loop and finishes
//! comfortably inside `activate()`.

/// Samples per frame at mip 0.
pub const FRAME_LEN: usize = 2048;
/// Morph frames per table.
pub const FRAMES: usize = 8;
/// Mip levels: level `k` keeps harmonics `1..=(1024 >> k)`; level 10 keeps
/// only the fundamental.
pub const MIPS: usize = 11;
/// Highest partial present at mip 0 (Nyquist of a 2048-sample frame).
pub const MAX_HARMONICS: usize = FRAME_LEN / 2;

/// Number of factory tables (must match `table_name` and `build_table`).
pub const TABLE_COUNT: usize = 9;

pub fn table_name(index: usize) -> &'static str {
    match index {
        0 => "Basic Shapes",
        1 => "PWM",
        2 => "Harmonic Sweep",
        3 => "Metal Bell",
        4 => "Vowel Morph",
        5 => "Growl",
        6 => "FM Growl",
        7 => "Sync Saw",
        8 => "Digital Grit",
        _ => "?",
    }
}

/// One wavetable: `FRAMES × MIPS` frames, each `FRAME_LEN` f32 samples,
/// laid out `[frame][mip][sample]`.
pub struct Wavetable {
    data: Vec<f32>,
}

impl Wavetable {
    #[inline]
    fn index(frame: usize, mip: usize) -> usize {
        (frame * MIPS + mip) * FRAME_LEN
    }

    /// Borrow one band-limited frame.
    #[inline]
    pub fn frame(&self, frame: usize, mip: usize) -> &[f32] {
        let at = Self::index(frame.min(FRAMES - 1), mip.min(MIPS - 1));
        &self.data[at..at + FRAME_LEN]
    }

    /// Sample the table with linear interpolation in all three axes:
    /// `phase` in [0,1), `pos` in [0,1] (frame morph), and a crossfade
    /// `mip_frac` toward the next-coarser mip.
    #[inline]
    pub fn sample(&self, phase: f32, pos: f32, mip: usize, mip_frac: f32) -> f32 {
        let fpos = pos.clamp(0.0, 1.0) * (FRAMES - 1) as f32;
        let f0 = fpos as usize;
        let f1 = (f0 + 1).min(FRAMES - 1);
        let ffrac = fpos - f0 as f32;

        let x = phase * FRAME_LEN as f32;
        let i0 = (x as usize) & (FRAME_LEN - 1);
        let i1 = (i0 + 1) & (FRAME_LEN - 1);
        let xfrac = x - (x as usize) as f32;

        let mip1 = (mip + 1).min(MIPS - 1);
        let read = |frame: usize, m: usize| -> f32 {
            let base = Self::index(frame, m);
            let a = self.data[base + i0];
            let b = self.data[base + i1];
            a + (b - a) * xfrac
        };

        let m0 = {
            let a = read(f0, mip);
            let b = read(f1, mip);
            a + (b - a) * ffrac
        };
        if mip_frac <= 0.0 || mip == mip1 {
            return m0;
        }
        let m1 = {
            let a = read(f0, mip1);
            let b = read(f1, mip1);
            a + (b - a) * ffrac
        };
        m0 + (m1 - m0) * mip_frac
    }
}

/// All factory tables, built once at startup/activate.
pub struct WavetableSet {
    tables: Vec<Wavetable>,
}

impl WavetableSet {
    pub fn factory() -> Self {
        let mut tables = Vec::with_capacity(TABLE_COUNT);
        for t in 0..TABLE_COUNT {
            tables.push(build_table(t));
        }
        Self { tables }
    }

    #[inline]
    pub fn table(&self, index: usize) -> &Wavetable {
        &self.tables[index.min(TABLE_COUNT - 1)]
    }
}

/// Harmonic recipe of one frame: cosine/sine coefficients per partial.
/// `cos_amp[h-1]`/`sin_amp[h-1]` weight partial `h`.
struct Recipe {
    cos_amp: Vec<f32>,
    sin_amp: Vec<f32>,
}

impl Recipe {
    fn new() -> Self {
        Self {
            cos_amp: vec![0.0; MAX_HARMONICS],
            sin_amp: vec![0.0; MAX_HARMONICS],
        }
    }
}

fn build_table(table: usize) -> Wavetable {
    let mut data = vec![0.0f32; FRAMES * MIPS * FRAME_LEN];
    for frame in 0..FRAMES {
        let morph = frame as f32 / (FRAMES - 1) as f32;
        let recipe = match table {
            0 => basic_shapes(morph),
            1 => pwm(morph),
            2 => harmonic_sweep(morph),
            3 => metal_bell(morph),
            4 => vowel_morph(morph),
            5 => growl(morph),
            6 => fm_growl(morph),
            7 => sync_saw(morph),
            _ => digital_grit(morph),
        };
        render_mips(&recipe, &mut data, frame);
    }
    Wavetable { data }
}

/// Render every mip of one frame by additive synthesis, then normalize the
/// full-band mip to ±1 and apply the same gain to all mips (so switching
/// mip never changes loudness).
fn render_mips(recipe: &Recipe, data: &mut [f32], frame: usize) {
    let base = (frame * MIPS) * FRAME_LEN;
    {
        let (mip0, rest) = data[base..].split_at_mut(FRAME_LEN);
        // Full-band render once; coarser mips re-render with truncated
        // harmonic counts (each halves the work, so the total is ~2x mip 0).
        additive(recipe, MAX_HARMONICS, mip0);
        let mut level = 1usize;
        let mut chunks = rest.chunks_exact_mut(FRAME_LEN);
        while level < MIPS {
            let out = chunks.next().expect("MIPS-1 chunks after mip 0");
            additive(recipe, MAX_HARMONICS >> level, out);
            level += 1;
        }
    }
    let mut peak = 0.0f32;
    for &v in &data[base..base + FRAME_LEN] {
        peak = peak.max(v.abs());
    }
    if peak > 1.0e-9 {
        let g = 1.0 / peak;
        for v in &mut data[base..base + MIPS * FRAME_LEN] {
            *v *= g;
        }
    }
}

/// Sum partials `1..=harmonics` of `recipe` into `out` (len `FRAME_LEN`).
///
/// For each partial `h` the pair `(cos(hθ), sin(hθ))` is advanced by a
/// complex rotation of `hθ_step` per sample — no trig in the inner loop.
fn additive(recipe: &Recipe, harmonics: usize, out: &mut [f32]) {
    out.fill(0.0);
    let step = core::f64::consts::TAU / FRAME_LEN as f64;
    for h in 1..=harmonics.min(MAX_HARMONICS) {
        let ca = recipe.cos_amp[h - 1];
        let sa = recipe.sin_amp[h - 1];
        if ca == 0.0 && sa == 0.0 {
            continue;
        }
        let (rot_s, rot_c) = (step * h as f64).sin_cos();
        // Start at θ=0: cos=1, sin=0.
        let mut c = 1.0f64;
        let mut s = 0.0f64;
        for v in out.iter_mut() {
            *v += ca * c as f32 + sa * s as f32;
            let nc = c * rot_c - s * rot_s;
            s = s * rot_c + c * rot_s;
            c = nc;
        }
    }
}

/// Classic analog morph: sine → triangle → saw → square → 12.5% pulse.
/// Adjacent shape recipes are linearly crossfaded in the harmonic domain.
fn basic_shapes(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let segments = 4.0;
    let x = morph * segments;
    let seg = (x as usize).min(3);
    let t = x - seg as f32;
    let (a, b) = (seg, seg + 1);
    for shape in [a, b] {
        let w = if shape == a { 1.0 - t } else { t };
        if w == 0.0 {
            continue;
        }
        match shape {
            0 => r.sin_amp[0] += w, // sine
            1 => {
                // triangle: odd h, 1/h^2, alternating sign
                for h in (1..=MAX_HARMONICS).step_by(2) {
                    let sign = if (h / 2) % 2 == 0 { 1.0 } else { -1.0 };
                    r.sin_amp[h - 1] +=
                        w * sign * (8.0 / core::f32::consts::PI.powi(2)) / (h * h) as f32;
                }
            }
            2 => {
                // saw: all h, 1/h
                for h in 1..=MAX_HARMONICS {
                    let sign = if h % 2 == 1 { 1.0 } else { -1.0 };
                    r.sin_amp[h - 1] += w * sign * (2.0 / core::f32::consts::PI) / h as f32;
                }
            }
            3 => {
                // square: odd h, 1/h
                for h in (1..=MAX_HARMONICS).step_by(2) {
                    r.sin_amp[h - 1] += w * (4.0 / core::f32::consts::PI) / h as f32;
                }
            }
            _ => {
                // 12.5% pulse: sin(π h d)/h series
                let d = 0.125f32;
                for h in 1..=MAX_HARMONICS {
                    let a_h = (core::f32::consts::PI * h as f32 * d).sin()
                        * (4.0 / core::f32::consts::PI)
                        / h as f32;
                    r.sin_amp[h - 1] += w * a_h;
                }
            }
        }
    }
    r
}

/// Pulse-width morph 50% → 5%.
fn pwm(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let d = 0.5 - 0.45 * morph;
    for h in 1..=MAX_HARMONICS {
        r.sin_amp[h - 1] =
            (core::f32::consts::PI * h as f32 * d).sin() * (4.0 / core::f32::consts::PI) / h as f32;
    }
    r
}

/// A Gaussian band of harmonics sweeping from the fundamental up to ~64th
/// partial, over a quiet saw bed — evolving pad material.
fn harmonic_sweep(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let center = 1.0 + morph * morph * 63.0;
    let width = 1.5 + morph * 6.0;
    for h in 1..=MAX_HARMONICS {
        let hf = h as f32;
        let d = (hf - center) / width;
        let band = (-d * d).exp() / hf.sqrt();
        let bed = 0.06 / hf;
        r.sin_amp[h - 1] = band + bed;
    }
    r
}

/// Sparse stretched partials whose upper register blooms with the morph —
/// bell/metallic flavor while staying strictly harmonic (alias-safe).
fn metal_bell(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    // Pseudo-inharmonic set: partials at h, rounded from h^1.31 spacing.
    let mut h = 1.0f32;
    let stretch = 1.18 + 0.25 * morph;
    let mut k = 0;
    while (h as usize) <= MAX_HARMONICS && k < 24 {
        let idx = h as usize;
        let decay = 1.0 / (1.0 + k as f32 * (1.4 - morph));
        let shimmer = 1.0 + morph * (k as f32 * 0.35);
        r.sin_amp[idx - 1] += decay * shimmer.min(3.0);
        // Slight cosine content de-phases the partials for a rounder attack.
        r.cos_amp[idx - 1] += 0.3 * decay * if k % 2 == 0 { 1.0 } else { -1.0 };
        h = (h * stretch).max(h + 1.0);
        k += 1;
    }
    r
}

/// Male vowel formants A-E-I-O-U — F1/F2/F3 center frequencies in Hz.
/// Shared with the engine's Formant filter so the "Vowel Morph" table and
/// the filter agree on what each vowel sounds like.
pub const VOWEL_FORMANTS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0], // a
    [530.0, 1840.0, 2480.0], // e
    [270.0, 2290.0, 3010.0], // i
    [570.0, 840.0, 2410.0],  // o
    [300.0, 870.0, 2240.0],  // u
];
/// -6 dB bandwidths of the three formant bands, in Hz.
pub const VOWEL_BANDWIDTHS: [f32; 3] = [90.0, 110.0, 170.0];
/// Relative level of the three formant bands.
pub const VOWEL_AMPS: [f32; 3] = [1.0, 0.63, 0.32];

/// Interpolate `VOWEL_FORMANTS`/`VOWEL_BANDWIDTHS` at a vowel position in
/// [0,1] mapped across A-E-I-O-U. Returns ([f1,f2,f3], [bw1,bw2,bw3]).
pub fn vowel_at(pos: f32) -> ([f32; 3], [f32; 3]) {
    let x = pos.clamp(0.0, 1.0) * (VOWEL_FORMANTS.len() - 1) as f32;
    let i0 = (x as usize).min(VOWEL_FORMANTS.len() - 2);
    let t = x - i0 as f32;
    let mut freqs = [0.0f32; 3];
    let mut bws = [0.0f32; 3];
    for k in 0..3 {
        let a = VOWEL_FORMANTS[i0][k];
        let b = VOWEL_FORMANTS[i0 + 1][k];
        freqs[k] = a + (b - a) * t;
        bws[k] = VOWEL_BANDWIDTHS[k];
    }
    (freqs, bws)
}

/// Vocal formant bands over a quiet harmonic bed. The morph sweeps the
/// vowel A → E → I → O → U; a 55 Hz (A1) fundamental is assumed so the
/// formant peaks land on realistic partial numbers for bass playing.
fn vowel_morph(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    const F0: f32 = 55.0;
    let (freqs, bws) = vowel_at(morph);
    // Formants live below ~3.2 kHz at f0=55 Hz → nothing above h≈128.
    let top = 128.min(MAX_HARMONICS);
    for h in 1..=top {
        let hf = h as f32;
        let mut a = 0.03 / hf; // bed keeps the fundamental present
        for k in 0..3 {
            let c = freqs[k] / F0;
            let sigma = (bws[k] / F0).max(0.5);
            let d = (hf - c) / sigma;
            a += VOWEL_AMPS[k] * (-0.5 * d * d).exp();
        }
        r.sin_amp[h - 1] = a;
    }
    r
}

/// Aggressive growl: odd-biased sawish bed with two resonant peaks that
/// sweep upward with the morph, plus alternating cosine content that
/// de-phases the partials for extra snarl.
fn growl(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let c1 = 3.0 + 10.0 * morph;
    let w1 = 1.5f32;
    let c2 = 7.0 + 24.0 * morph;
    let w2 = 2.5f32;
    for h in 1..=MAX_HARMONICS {
        let hf = h as f32;
        let base = (1.0 / hf) * if h % 2 == 1 { 1.0 } else { 0.35 };
        let d1 = (hf - c1) / w1;
        let d2 = (hf - c2) / w2;
        let peaks = 1.0 + 2.5 * (-d1 * d1).exp() + 1.8 * (-d2 * d2).exp();
        r.sin_amp[h - 1] = base * peaks;
        r.cos_amp[h - 1] = 0.15 * base * if h % 2 == 0 { 1.0 } else { -1.0 };
    }
    r
}

/// Naive DFT of one `FRAME_LEN`-sample cycle into a harmonic recipe —
/// the analysis twin of `additive()`, using the same complex-rotation
/// recurrence with f64 accumulation. Only partials `1..=max_h` are kept,
/// so time-domain generators stay alias-safe once mipped.
fn recipe_from_waveform(wave: &[f32], max_h: usize) -> Recipe {
    debug_assert_eq!(wave.len(), FRAME_LEN);
    let mut r = Recipe::new();
    let step = core::f64::consts::TAU / FRAME_LEN as f64;
    let norm = 2.0 / FRAME_LEN as f64;
    for h in 1..=max_h.min(MAX_HARMONICS) {
        let (rot_s, rot_c) = (step * h as f64).sin_cos();
        let mut c = 1.0f64;
        let mut s = 0.0f64;
        let mut acc_c = 0.0f64;
        let mut acc_s = 0.0f64;
        for &x in wave {
            acc_c += x as f64 * c;
            acc_s += x as f64 * s;
            let nc = c * rot_c - s * rot_s;
            s = s * rot_c + c * rot_s;
            c = nc;
        }
        r.cos_amp[h - 1] = (acc_c * norm) as f32;
        r.sin_amp[h - 1] = (acc_s * norm) as f32;
    }
    r
}

/// Two-operator 1:1 FM (`sin(θ + I·sin θ)`) rendered in the time domain
/// and analyzed back to harmonics. The integer carrier:modulator ratio
/// keeps the cycle exactly periodic, so the spectrum is strictly harmonic
/// and alias-safe. The morph drives the modulation index 0 → 8.
fn fm_growl(morph: f32) -> Recipe {
    let index = 8.0 * morph * morph;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let theta = core::f64::consts::TAU * n as f64 / FRAME_LEN as f64;
        *v = (theta + index as f64 * theta.sin()).sin() as f32;
    }
    // Carson bandwidth ≈ (I+2) partials at max index — 256 is generous.
    recipe_from_waveform(&wave, 256)
}

/// Hard-synced saw: a slave saw at `r = 1 + 5·morph` reset every master
/// cycle. Rendered in the time domain and DFT'd; the reset discontinuity
/// band-limits exactly like the plain saw recipe (1/h tail).
fn sync_saw(morph: f32) -> Recipe {
    let ratio = 1.0 + 5.0 * morph;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let p = n as f32 / FRAME_LEN as f32 * ratio;
        *v = 2.0 * (p - p.floor()) - 1.0;
    }
    recipe_from_waveform(&wave, MAX_HARMONICS)
}

/// Deterministic per-harmonic hash in [0,1) — xorshift-style, mirrors the
/// engine's `Rng` recipe so "random" phases are reproducible.
fn hash01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747796405).wrapping_add(2891336453) | 1;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    (x >> 8) as f32 / (1 << 24) as f32
}

/// Harsh digital spectrum: slow 1/h^0.35 rolloff with pseudo-random
/// phases and comb-like notches. The morph opens the harmonic ceiling
/// (24 → 512 partials) and tightens the notch spacing.
fn digital_grit(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let hmax = (24.0 + morph * morph * 488.0) as usize;
    let notch_every = (16.0 - (13.0 * morph).round()).max(2.0) as usize;
    for h in 1..=hmax.min(MAX_HARMONICS) {
        let mut a = 1.0 / (h as f32).powf(0.35);
        if h % notch_every == 0 {
            a *= 0.15;
        }
        let phase = hash01(h as u32) * core::f32::consts::TAU;
        r.cos_amp[h - 1] = a * phase.cos();
        r.sin_amp[h - 1] = a * phase.sin();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Goertzel power at integer harmonic `h` of one frame.
    fn harmonic_power(frame: &[f32], h: usize) -> f64 {
        let w = core::f64::consts::TAU * h as f64 / frame.len() as f64;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in frame {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let real = s1 - s2 * w.cos();
        let imag = s2 * w.sin();
        (real * real + imag * imag) / (frame.len() as f64 / 2.0).powi(2)
    }

    #[test]
    fn mips_suppress_harmonics_above_their_band() {
        let set = WavetableSet::factory();
        for t in 0..TABLE_COUNT {
            let table = set.table(t);
            for mip in 1..MIPS {
                let allowed = MAX_HARMONICS >> mip;
                let frame = table.frame(FRAMES / 2, mip);
                // Everything above the band must be numerically silent.
                for h in [allowed + 1, (allowed + 1) * 2, MAX_HARMONICS - 1] {
                    if h > MAX_HARMONICS {
                        continue;
                    }
                    let p = harmonic_power(frame, h);
                    assert!(
                        p < 1.0e-8,
                        "table {t} mip {mip}: harmonic {h} leaks power {p}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_frame_is_normalized_and_finite() {
        let set = WavetableSet::factory();
        for t in 0..TABLE_COUNT {
            for f in 0..FRAMES {
                let frame = set.table(t).frame(f, 0);
                let peak = frame.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(frame.iter().all(|v| v.is_finite()));
                assert!(
                    (0.9..=1.0001).contains(&peak),
                    "table {t} frame {f} peak {peak}"
                );
            }
        }
    }

    #[test]
    fn basic_shapes_first_frame_is_a_sine() {
        let set = WavetableSet::factory();
        let frame = set.table(0).frame(0, 0);
        let fundamental = harmonic_power(frame, 1);
        let second = harmonic_power(frame, 2);
        assert!(fundamental > 0.9);
        assert!(second < 1.0e-8);
    }

    #[test]
    fn recipe_from_waveform_round_trips() {
        // Analyze a known additive render and compare coefficients.
        let mut source = Recipe::new();
        source.sin_amp[0] = 0.8;
        source.sin_amp[2] = 0.3;
        source.cos_amp[4] = -0.2;
        let mut wave = vec![0.0f32; FRAME_LEN];
        additive(&source, MAX_HARMONICS, &mut wave);
        let got = recipe_from_waveform(&wave, 8);
        for h in 1..=8 {
            assert!(
                (got.sin_amp[h - 1] - source.sin_amp[h - 1]).abs() < 1.0e-4,
                "sin h{h}: {} vs {}",
                got.sin_amp[h - 1],
                source.sin_amp[h - 1]
            );
            assert!(
                (got.cos_amp[h - 1] - source.cos_amp[h - 1]).abs() < 1.0e-4,
                "cos h{h}: {} vs {}",
                got.cos_amp[h - 1],
                source.cos_amp[h - 1]
            );
        }
    }

    #[test]
    fn vowel_table_peaks_near_f1() {
        // Frame 0 is the "a" vowel: F1=730 Hz at f0=55 Hz → partial ~13.
        // Power there must dominate a far-off partial (~h=40).
        let set = WavetableSet::factory();
        let frame = set.table(4).frame(0, 0);
        let near_f1 = harmonic_power(frame, 13);
        let far = harmonic_power(frame, 40);
        assert!(
            near_f1 > far * 10.0,
            "F1 region should dominate: near={near_f1} far={far}"
        );
    }

    #[test]
    fn fm_table_bandwidth_grows_with_morph() {
        // Morph 0 is a pure sine; the last frame must carry real energy in
        // upper partials.
        let set = WavetableSet::factory();
        let quiet = set.table(6).frame(0, 0);
        let bright = set.table(6).frame(FRAMES - 1, 0);
        assert!(harmonic_power(quiet, 5) < 1.0e-6);
        assert!(harmonic_power(bright, 5) > 1.0e-4);
    }

    #[test]
    fn sampling_interpolates_between_frames() {
        let set = WavetableSet::factory();
        let table = set.table(0);
        // Morph midpoint should differ from both endpoints somewhere.
        let a = table.sample(0.25, 0.0, 0, 0.0);
        let b = table.sample(0.25, 1.0, 0, 0.0);
        let mid = table.sample(0.25, 0.5, 0, 0.0);
        assert!(a.is_finite() && b.is_finite() && mid.is_finite());
        assert!((mid - a).abs() > 1.0e-6 || (mid - b).abs() > 1.0e-6);
    }
}
