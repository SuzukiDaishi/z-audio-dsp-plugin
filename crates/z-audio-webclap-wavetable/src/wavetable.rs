//! Factory wavetables with band-limited mip levels.
//!
//! Each table is `FRAMES` morph frames of `FRAME_LEN` samples, stored at
//! `MIPS` mip levels. Mip `k` contains only harmonics `1..=(MAX_HARMONICS >> k)`,
//! so the oscillator can pick a level whose highest partial stays below
//! Nyquist for the current playback increment — the core of alias-free
//! wavetable playback.
//!
//! Every frame is defined as a harmonic recipe (amplitude + phase per
//! partial) and rendered to samples by an inverse FFT (`synthesize`), so
//! building all `TABLE_COUNT` tables stays comfortably inside the
//! `activate()` budget; a slow additive renderer survives in the tests as
//! the oracle the FFT path is checked against.

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
pub const TABLE_COUNT: usize = 32;

/// Indices 0-8 predate the big factory expansion and are frozen for
/// saved-state compatibility; new tables are append-only.
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
        9 => "Octave Stack",
        10 => "Soft Square",
        11 => "Tri Fold",
        12 => "Pulse Train",
        13 => "Choir Ahh",
        14 => "Vowel Talk",
        15 => "Throat",
        16 => "Bit Steps",
        17 => "Sync Square",
        18 => "VOSIM",
        19 => "FM Bell",
        20 => "FM Bass",
        21 => "FM Fold",
        22 => "Glass",
        23 => "Gamelan",
        24 => "Drawbars",
        25 => "Even/Odd",
        26 => "Pipe Organ",
        27 => "String Machine",
        28 => "Pluck String",
        29 => "Breath",
        30 => "Spectral Noise",
        31 => "Solid Sub",
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

/// All factory tables, built once per process and shared by every engine
/// instance (the set is immutable after construction).
pub struct WavetableSet {
    tables: Vec<Wavetable>,
}

impl WavetableSet {
    /// The process-wide factory set. The first call pays the full additive
    /// build; later calls (more plugin instances, every unit test) get the
    /// cached `Arc` for free.
    pub fn factory() -> std::sync::Arc<WavetableSet> {
        static FACTORY: std::sync::OnceLock<std::sync::Arc<WavetableSet>> =
            std::sync::OnceLock::new();
        FACTORY
            .get_or_init(|| std::sync::Arc::new(Self::build_all()))
            .clone()
    }

    fn build_all() -> Self {
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
            8 => digital_grit(morph),
            9 => octave_stack(morph),
            10 => soft_square(morph),
            11 => tri_fold(morph),
            12 => pulse_train(morph),
            13 => choir_ahh(morph),
            14 => vowel_talk(morph),
            15 => throat(morph),
            16 => bit_steps(morph),
            17 => sync_square(morph),
            18 => vosim(morph),
            19 => fm_bell(morph),
            20 => fm_bass(morph),
            21 => fm_fold(morph),
            22 => glass(morph),
            23 => gamelan(morph),
            24 => drawbars(morph),
            25 => even_odd(morph),
            26 => pipe_organ(morph),
            27 => string_machine(morph),
            28 => pluck_string(morph),
            29 => breath(morph),
            30 => spectral_noise(morph),
            _ => solid_sub(morph),
        };
        render_mips(&recipe, &mut data, frame);
    }
    // One gain for the whole table: peak measured across every frame's
    // full-band mip, applied to all frames and mips, so neither morphing
    // nor mip switching ever changes loudness.
    let mut peak = 0.0f32;
    for frame in 0..FRAMES {
        let base = (frame * MIPS) * FRAME_LEN;
        for &v in &data[base..base + FRAME_LEN] {
            peak = peak.max(v.abs());
        }
    }
    if peak > 1.0e-9 {
        let g = 1.0 / peak;
        for v in &mut data {
            *v *= g;
        }
    }
    Wavetable { data }
}

/// Render every mip of one frame (inverse-FFT synthesis). Normalization
/// happens once per table in `build_table` — a single gain across all
/// frames and mips, so neither morphing nor mip switching changes
/// loudness.
fn render_mips(recipe: &Recipe, data: &mut [f32], frame: usize) {
    let base = (frame * MIPS) * FRAME_LEN;
    let (mip0, rest) = data[base..].split_at_mut(FRAME_LEN);
    // Full-band render once; coarser mips re-render with truncated
    // harmonic counts.
    synthesize(recipe, MAX_HARMONICS, mip0);
    let mut level = 1usize;
    let mut chunks = rest.chunks_exact_mut(FRAME_LEN);
    while level < MIPS {
        let out = chunks.next().expect("MIPS-1 chunks after mip 0");
        synthesize(recipe, MAX_HARMONICS >> level, out);
        level += 1;
    }
}

/// In-place iterative radix-2 complex FFT of length `re.len()` (a power
/// of two), evaluating `x[n] = Σ_k X[k]·e^{+i·2πkn/N}` (no 1/N factor) —
/// i.e. the inverse transform as `synthesize` needs it.
fn ifft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 0..n {
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
    }
    // Butterflies with the +i (inverse) twiddle sign.
    let mut len = 2usize;
    while len <= n {
        let ang = core::f64::consts::TAU / len as f64;
        let (ws, wc) = ang.sin_cos();
        let mut at = 0usize;
        while at < n {
            let mut cr = 1.0f64;
            let mut ci = 0.0f64;
            for k in 0..len / 2 {
                let (a, b) = (at + k, at + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wc - ci * ws;
                ci = ci * wc + cr * ws;
                cr = ncr;
            }
            at += len;
        }
        len <<= 1;
    }
}

/// Render partials `1..=harmonics` of `recipe` into `out` (len
/// `FRAME_LEN`) via an inverse FFT — O(N log N) per mip instead of the
/// O(N·harmonics) additive sum, which keeps the 32-table factory build
/// inside the activate() budget.
fn synthesize(recipe: &Recipe, harmonics: usize, out: &mut [f32]) {
    let n = FRAME_LEN;
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    for h in 1..=harmonics.min(MAX_HARMONICS) {
        let ca = recipe.cos_amp[h - 1] as f64;
        let sa = recipe.sin_amp[h - 1] as f64;
        if ca == 0.0 && sa == 0.0 {
            continue;
        }
        if h == n / 2 {
            // Nyquist bin: cos alternates ±1; a sine partial is zero at
            // every sample point (matching the additive reference).
            re[h] += ca;
        } else {
            // ca·cos(hθ) + sa·sin(hθ)  ⇔  X[h] = (ca − i·sa)/2 and the
            // conjugate-mirror bin X[N−h] = (ca + i·sa)/2.
            re[h] += 0.5 * ca;
            im[h] -= 0.5 * sa;
            re[n - h] += 0.5 * ca;
            im[n - h] += 0.5 * sa;
        }
    }
    ifft(&mut re, &mut im);
    for (v, &r) in out.iter_mut().zip(re.iter()) {
        *v = r as f32;
    }
}

/// Reference additive renderer (complex-rotation recurrence, no trig in
/// the inner loop). The production path uses `synthesize`; this stays as
/// the independent oracle the FFT path is tested against.
#[cfg(test)]
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
                // 12.5% pulse, cosine phase: the true centered pulse shape.
                // (Sine phase would scramble the partials into a waveform
                // with a ~4.6 crest factor — same spectrum, but it starves
                // the normalizer and hits drive stages badly.)
                let d = 0.125f32;
                for h in 1..=MAX_HARMONICS {
                    let a_h = (core::f32::consts::PI * h as f32 * d).sin()
                        * (4.0 / core::f32::consts::PI)
                        / h as f32;
                    r.cos_amp[h - 1] += w * a_h;
                }
            }
        }
    }
    r
}

/// Pulse-width morph 50% → 5%. Cosine phase renders the true centered
/// pulse (low crest factor); sine phase would smear it into a spiky
/// pseudo-pulse that dominates the table normalizer.
fn pwm(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let d = 0.5 - 0.45 * morph;
    for h in 1..=MAX_HARMONICS {
        r.cos_amp[h - 1] =
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

// ---------------------------------------------------------------------------
// Analog / classic (9-12)
// ---------------------------------------------------------------------------

/// Saw layers at 1x, 2x and 3x the fundamental — hoover/supersaw flavor.
/// The morph brings in the octave and then the twelfth.
fn octave_stack(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let layers = [
        (1usize, 1.0f32),
        (2, morph),
        (3, ((morph - 0.5).max(0.0)) * 1.2),
    ];
    for (k, g) in layers {
        if g <= 0.0 {
            continue;
        }
        let mut h = k;
        while h <= MAX_HARMONICS {
            let rank = h / k;
            let sign = if rank % 2 == 1 { 1.0 } else { -1.0 };
            r.sin_amp[h - 1] += g * sign * (2.0 / core::f32::consts::PI) * k as f32 / h as f32;
            h += k;
        }
    }
    r
}

/// Rounded square whose edge hardens with the morph (Gaussian rolloff on
/// the odd-harmonic series).
fn soft_square(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let hc = 6.0 + 250.0 * morph * morph;
    for h in (1..=256.min(MAX_HARMONICS)).step_by(2) {
        let hf = h as f32;
        r.sin_amp[h - 1] = (4.0 / core::f32::consts::PI) / hf * (-(hf / hc) * (hf / hc)).exp();
    }
    r
}

/// Sine through a wavefolder: `sin(g·sin θ)` — warm triangle-ish folds
/// that turn brassy as the morph raises the fold gain.
fn tri_fold(morph: f32) -> Recipe {
    let gain = (1.0 + 5.0 * morph) as f64 * core::f64::consts::FRAC_PI_2;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let theta = core::f64::consts::TAU * n as f64 / FRAME_LEN as f64;
        *v = (gain * theta.sin()).sin() as f32;
    }
    recipe_from_waveform(&wave, 128)
}

/// Raised-cosine impulse train — analog buzz. The morph narrows the pulse
/// (widens the spectrum).
fn pulse_train(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let hc = 4.0 + 124.0 * morph;
    for h in 1..=256.min(MAX_HARMONICS) {
        let hf = h as f32;
        r.cos_amp[h - 1] = (-(hf / hc) * (hf / hc)).exp();
    }
    r
}

// ---------------------------------------------------------------------------
// Vocal / formant (13-15)
// ---------------------------------------------------------------------------

/// Softer, breathier sibling of Vowel Morph: wider formant bands around a
/// lower reference pitch plus hashed-phase "air" partials. Morph drifts
/// the vowel a→o and raises the breathiness.
fn choir_ahh(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    const F0: f32 = 82.0;
    let (freqs, bws) = vowel_at(0.75 * morph);
    for h in 1..=128.min(MAX_HARMONICS) {
        let hf = h as f32;
        let mut a = 0.04 / hf;
        for k in 0..3 {
            let c = freqs[k] / F0;
            let sigma = (2.0 * bws[k] / F0).max(1.0);
            let d = (hf - c) / sigma;
            a += VOWEL_AMPS[k] * (-0.5 * d * d).exp();
        }
        r.sin_amp[h - 1] = a;
    }
    let air = 0.03 + 0.09 * morph;
    for h in 40..=96.min(MAX_HARMONICS) {
        let phase = hash01(h as u32 * 11) * core::f32::consts::TAU;
        r.cos_amp[h - 1] += air * phase.cos();
        r.sin_amp[h - 1] += air * phase.sin();
    }
    r
}

/// Talky F2-forward vowel: the morph runs the u→o→i "wow→yee" vector —
/// made to be swept by an LFO or envelope.
fn vowel_talk(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    const F0: f32 = 110.0;
    const AMPS: [f32; 3] = [0.7, 1.0, 0.35];
    let (freqs, bws) = vowel_at(1.0 - 0.5 * morph);
    for h in 1..=64.min(MAX_HARMONICS) {
        let hf = h as f32;
        let mut a = 0.05 / hf;
        for k in 0..3 {
            let c = freqs[k] / F0;
            let sigma = (bws[k] / F0).max(0.8);
            let d = (hf - c) / sigma;
            a += AMPS[k] * (-0.5 * d * d).exp();
        }
        r.sin_amp[h - 1] = a;
    }
    r
}

/// Glottal growl bed: odd-biased rolloff with a single sweeping resonance.
/// Companion to Growl for neuro basses.
fn throat(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let p = 1.6 - 0.8 * morph;
    let c = 6.0 + 30.0 * morph;
    for h in 1..=128.min(MAX_HARMONICS) {
        let hf = h as f32;
        let base = (1.0 / hf.powf(p)) * if h % 2 == 1 { 1.0 } else { 0.6 };
        let d = (hf - c) / 3.0;
        let a = base * (1.0 + 2.5 * (-d * d).exp());
        r.sin_amp[h - 1] = a;
        r.cos_amp[h - 1] = 0.2 * a * if h % 2 == 0 { 1.0 } else { -1.0 };
    }
    r
}

// ---------------------------------------------------------------------------
// Digital / harsh (16-18)
// ---------------------------------------------------------------------------

/// Amplitude-quantized saw — the morph lowers the bit depth from 16
/// levels down to 2.
fn bit_steps(morph: f32) -> Recipe {
    let levels = (16.0 - 14.0 * morph).round().max(2.0);
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let saw = 2.0 * (n as f32 / FRAME_LEN as f32) - 1.0;
        *v = (saw * levels).round() / levels;
    }
    recipe_from_waveform(&wave, 512)
}

/// Hard-synced square, slave ratio 1→7 — screaming sync lead partner to
/// Sync Saw.
fn sync_square(morph: f32) -> Recipe {
    let ratio = 1.0 + 6.0 * morph;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let p = n as f32 / FRAME_LEN as f32 * ratio;
        *v = if p - p.floor() < 0.5 { 1.0 } else { -1.0 };
    }
    recipe_from_waveform(&wave, MAX_HARMONICS)
}

/// VOSIM-style resonant burst: a sine at partial `k` under a cubic decay
/// window (which zeroes the cycle seam). Morph sweeps the resonance 2→16.
fn vosim(morph: f32) -> Recipe {
    let k = (2.0 + 14.0 * morph) as f64;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let p = n as f64 / FRAME_LEN as f64;
        let window = (1.0 - p).powi(3);
        *v = ((core::f64::consts::TAU * k * p).sin() * window) as f32;
    }
    recipe_from_waveform(&wave, 256)
}

// ---------------------------------------------------------------------------
// FM (19-21)
// ---------------------------------------------------------------------------

/// 1:3 FM — clangy bell/EP sidebands, strictly harmonic.
fn fm_bell(morph: f32) -> Recipe {
    let index = (0.5 + 5.5 * morph) as f64;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let theta = core::f64::consts::TAU * n as f64 / FRAME_LEN as f64;
        *v = (theta + index * (3.0 * theta).sin()).sin() as f32;
    }
    recipe_from_waveform(&wave, 256)
}

/// 2:1 FM — sub-heavy sidebands for bass work.
fn fm_bass(morph: f32) -> Recipe {
    let index = (7.0 * morph) as f64;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let theta = core::f64::consts::TAU * n as f64 / FRAME_LEN as f64;
        *v = (2.0 * theta + index * theta.sin()).sin() as f32;
    }
    recipe_from_waveform(&wave, 128)
}

/// Nested (feedback-style) FM: sine → saw-adjacent → chaotic-bright.
fn fm_fold(morph: f32) -> Recipe {
    let index = (3.2 * morph) as f64;
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let theta = core::f64::consts::TAU * n as f64 / FRAME_LEN as f64;
        *v = (theta + index * (theta + index * theta.sin()).sin()).sin() as f32;
    }
    recipe_from_waveform(&wave, 512)
}

// ---------------------------------------------------------------------------
// Bell / metallic (22-23)
// ---------------------------------------------------------------------------

/// Sparse partials at ~squared spacing — crystalline; the morph compresses
/// the spacing toward a harmonic ring.
fn glass(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let power = 2.0 - 0.5 * morph;
    for k in 1..=11u32 {
        let idx = ((k as f32).powf(power).round() as usize).clamp(1, 128);
        let kf = k as f32;
        r.sin_amp[idx - 1] += 1.0 / kf.powf(1.1);
        r.cos_amp[idx - 1] += 0.4 * if k % 2 == 0 { 1.0 } else { -1.0 } / kf;
    }
    r
}

/// Two metallic partial sets crossfaded by the morph, upper partials
/// blooming toward the B set.
fn gamelan(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    const SET_A: [usize; 8] = [1, 3, 7, 12, 19, 27, 38, 51];
    const SET_B: [usize; 8] = [1, 4, 9, 15, 23, 33, 45, 60];
    for (k, (&a, &b)) in SET_A.iter().zip(SET_B.iter()).enumerate() {
        let kf = k as f32;
        let ga = (1.0 - morph) / (1.0 + 0.5 * kf);
        let gb = morph * (1.0 + 0.2 * kf) / (1.0 + 0.5 * kf);
        r.sin_amp[a - 1] += ga;
        r.sin_amp[b - 1] += gb;
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        r.cos_amp[a - 1] += 0.3 * ga * sign;
        r.cos_amp[b - 1] += 0.3 * gb * sign;
    }
    r
}

// ---------------------------------------------------------------------------
// Organ / additive (24-26)
// ---------------------------------------------------------------------------

/// Three Hammond-style drawbar registrations, morph-faded flute → jazz →
/// full. Sixteen partials at most — almost free to build.
fn drawbars(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    // (partial, flute, jazz, full)
    const BARS: [(usize, f32, f32, f32); 9] = [
        (1, 1.0, 1.0, 1.0),
        (2, 0.9, 0.9, 0.9),
        (3, 0.8, 0.8, 0.85),
        (4, 0.0, 0.7, 0.75),
        (6, 0.0, 0.0, 0.6),
        (8, 0.0, 0.0, 0.55),
        (10, 0.0, 0.0, 0.45),
        (12, 0.0, 0.0, 0.4),
        (16, 0.0, 0.6, 0.5),
    ];
    for (h, flute, jazz, full) in BARS {
        let a = if morph < 0.5 {
            let t = morph * 2.0;
            flute + (jazz - flute) * t
        } else {
            let t = (morph - 0.5) * 2.0;
            jazz + (full - jazz) * t
        };
        r.sin_amp[h - 1] = a;
    }
    r
}

/// Hollow odd-harmonic tone ⇔ fundamental-plus-evens octaver blend.
fn even_odd(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    for h in 1..=128.min(MAX_HARMONICS) {
        let hf = h as f32;
        let odd = if h % 2 == 1 { 1.0 / hf } else { 0.0 };
        let even = if h % 2 == 0 {
            1.0 / hf
        } else if h == 1 {
            0.8
        } else {
            0.0
        };
        r.sin_amp[h - 1] = (1.0 - morph) * odd + morph * even;
    }
    r
}

/// Principal pipes plus mixture ranks and a whisper of chiff; the morph
/// draws more ranks.
fn pipe_organ(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    for h in 1..=8 {
        let hf = h as f32;
        let odd_lift = if h % 2 == 1 { 1.15 } else { 1.0 };
        r.sin_amp[h - 1] = odd_lift / hf.powf(1.7);
    }
    // Octave ranks 2', 1' ... and quint ranks.
    for (rank, h) in [(1.0f32, 2usize), (2.0, 4), (3.0, 8), (4.0, 16)] {
        r.sin_amp[h - 1] += 0.5 * morph / rank;
    }
    for (i, h) in [3usize, 6, 12].iter().enumerate() {
        r.sin_amp[h - 1] += 0.35 * morph / (i as f32 + 1.0);
    }
    let chiff = 0.02 * morph;
    for h in 20..=40 {
        let phase = hash01(h as u32 * 23) * core::f32::consts::TAU;
        r.cos_amp[h - 1] += chiff * phase.cos();
        r.sin_amp[h - 1] += chiff * phase.sin();
    }
    r
}

// ---------------------------------------------------------------------------
// String machine (27-28)
// ---------------------------------------------------------------------------

/// Ensemble saw with a comb whose notches sweep with the morph — the
/// classic string-machine shimmer, phases gently blurred.
fn string_machine(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let p = 1.3 - 0.25 * morph;
    for h in 1..=512.min(MAX_HARMONICS) {
        let hf = h as f32;
        let comb = 0.55
            + 0.45 * (core::f32::consts::TAU * hf / 9.0 + core::f32::consts::TAU * morph).cos();
        let a = comb / hf.powf(p);
        r.sin_amp[h - 1] = a;
        r.cos_amp[h - 1] = 0.2 * a * (core::f32::consts::TAU * hash01(h as u32 * 3)).sin();
    }
    r
}

/// Plucked-string spectrum, `sin(π h β)/h^1.6` — the morph moves the
/// pluck point from the middle (hollow) toward the bridge (bright).
fn pluck_string(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let beta = 0.5 - 0.38 * morph;
    for h in 1..=256.min(MAX_HARMONICS) {
        let hf = h as f32;
        r.sin_amp[h - 1] = (core::f32::consts::PI * hf * beta).sin() / hf.powf(1.6);
    }
    r
}

// ---------------------------------------------------------------------------
// Noise-tinged (29-30)
// ---------------------------------------------------------------------------

/// A tonal bed with a hashed-phase band of "air" sweeping upward — flute
/// breath to airy hiss.
fn breath(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    for h in 1..=16 {
        r.sin_amp[h - 1] = 0.5 / h as f32;
    }
    let c = 40.0 + 300.0 * morph * morph;
    let w = 30.0 + 80.0 * morph;
    for h in 24..=512.min(MAX_HARMONICS) {
        let hf = h as f32;
        let d = (hf - c) / w;
        let amp = 0.35 * (-d * d).exp() * (0.5 + 0.5 * hash01(h as u32 * 7));
        if amp < 1.0e-5 {
            continue;
        }
        let phase = hash01(h as u32 * 13) * core::f32::consts::TAU;
        r.cos_amp[h - 1] += amp * phase.cos();
        r.sin_amp[h - 1] += amp * phase.sin();
    }
    r
}

/// Frozen noise snapshots: every frame gets a different deterministic
/// spectrum, so morphing scans an evolving texture.
fn spectral_noise(morph: f32) -> Recipe {
    let mut r = Recipe::new();
    let seed = (morph * (FRAMES - 1) as f32).round() as u32 * 1009;
    let hc = 60.0 + 452.0 * morph;
    for h in 1..=512.min(MAX_HARMONICS) {
        let hf = h as f32;
        let rough = hash01(h as u32 * 31 + seed);
        let amp = rough * rough / hf.powf(0.3) * (-(hf / hc) * (hf / hc)).exp();
        if amp < 1.0e-5 {
            continue;
        }
        let phase = hash01(h as u32 * 57 + seed) * core::f32::consts::TAU;
        r.cos_amp[h - 1] = amp * phase.cos();
        r.sin_amp[h - 1] = amp * phase.sin();
    }
    r
}

// ---------------------------------------------------------------------------
// Sub (31)
// ---------------------------------------------------------------------------

/// Saturated sub: sine (plus a touch of 2nd harmonic) through tanh — the
/// morph adds weight and drive. Bass workhorse.
fn solid_sub(morph: f32) -> Recipe {
    let gain = (1.0 + 4.0 * morph) as f64;
    let norm = gain.tanh();
    let mut wave = vec![0.0f32; FRAME_LEN];
    for (n, v) in wave.iter_mut().enumerate() {
        let theta = core::f64::consts::TAU * n as f64 / FRAME_LEN as f64;
        let x = theta.sin() + 0.3 * morph as f64 * (2.0 * theta).sin();
        *v = ((gain * x).tanh() / norm) as f32;
    }
    recipe_from_waveform(&wave, 64)
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
    fn tables_are_normalized_and_finite() {
        // Normalization is per table: every frame stays within ±1 and the
        // loudest frame of each table sits at the normalization target.
        let set = WavetableSet::factory();
        for t in 0..TABLE_COUNT {
            let mut table_peak = 0.0f32;
            for f in 0..FRAMES {
                let frame = set.table(t).frame(f, 0);
                let peak = frame.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(frame.iter().all(|v| v.is_finite()));
                assert!(peak <= 1.0001, "table {t} frame {f} peak {peak}");
                table_peak = table_peak.max(peak);
            }
            assert!(
                (0.9..=1.0001).contains(&table_peak),
                "table {t} peak {table_peak}"
            );
        }
    }

    #[test]
    fn normalization_is_per_table_not_per_frame() {
        // PWM: the narrow-pulse frame sets the table peak, so the square
        // frame must sit well below 1.0 (per-frame norm would put it at ~1).
        let set = WavetableSet::factory();
        let square = set.table(1).frame(0, 0);
        let peak = square.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            peak < 0.9,
            "PWM frame 0 peak {peak} suggests per-frame normalization"
        );
    }

    #[test]
    fn basic_shapes_first_frame_is_a_sine() {
        let set = WavetableSet::factory();
        let frame = set.table(0).frame(0, 0);
        let fundamental = harmonic_power(frame, 1);
        let second = harmonic_power(frame, 2);
        // The table gain is set by the loudest frame (the 12.5% pulse), so
        // the sine frame sits below full scale — assert purity, not level.
        assert!(fundamental > 0.2);
        assert!(second < fundamental * 1.0e-6);
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
    fn fft_synthesis_matches_the_additive_oracle() {
        // A recipe with content on both axes, dense low end, a Nyquist
        // partial, and arbitrary values — synthesize() must agree with the
        // straight additive sum everywhere.
        let mut r = Recipe::new();
        for h in 1..=64usize {
            r.sin_amp[h - 1] = 1.0 / h as f32;
            r.cos_amp[h - 1] = if h % 3 == 0 { -0.2 / h as f32 } else { 0.0 };
        }
        r.sin_amp[999] = 0.4;
        r.cos_amp[MAX_HARMONICS - 1] = 0.25; // Nyquist bin
        let mut fft_out = vec![0.0f32; FRAME_LEN];
        let mut add_out = vec![0.0f32; FRAME_LEN];
        synthesize(&r, MAX_HARMONICS, &mut fft_out);
        additive(&r, MAX_HARMONICS, &mut add_out);
        for (i, (a, b)) in fft_out.iter().zip(add_out.iter()).enumerate() {
            assert!(
                (a - b).abs() < 1.0e-4,
                "sample {i}: fft {a} vs additive {b}"
            );
        }
    }

    #[test]
    fn factory_build_time_is_reported() {
        // Visibility only (no assert — CI timing varies): run with
        // `--nocapture` to see the first-build cost that activate() pays.
        let start = std::time::Instant::now();
        let _set = WavetableSet::factory();
        println!(
            "WavetableSet::factory() ({} tables): {:?} (0 if already cached)",
            TABLE_COUNT,
            start.elapsed()
        );
    }

    #[test]
    fn table_names_are_unique_and_short() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..TABLE_COUNT {
            let name = table_name(i);
            assert_ne!(name, "?", "table {i} has no name");
            assert!(name.len() <= 14, "table {i} name too long: {name}");
            assert!(seen.insert(name), "duplicate table name {name}");
        }
        assert_eq!(table_name(TABLE_COUNT), "?");
    }

    #[test]
    fn ui_table_names_match_rust() {
        // TABLE_NAMES in the UI must list every factory table (drift guard).
        let main_js = include_str!("../ui/main.js");
        for i in 0..TABLE_COUNT {
            let quoted = format!("\"{}\"", table_name(i));
            assert!(
                main_js.contains(&quoted),
                "ui/main.js TABLE_NAMES is missing {quoted}"
            );
        }
    }

    #[test]
    fn drawbars_have_no_off_registration_partials() {
        // h=7 and h=13 are not drawbar partials in any registration.
        let set = WavetableSet::factory();
        for frame in [0, FRAMES - 1] {
            let f = set.table(24).frame(frame, 0);
            assert!(harmonic_power(f, 7) < 1.0e-8);
            assert!(harmonic_power(f, 13) < 1.0e-8);
        }
    }

    #[test]
    fn pluck_string_has_comb_null_at_pluck_point() {
        // Frame 0: β = 0.5 → sin(π h β) = 0 for every even h.
        let set = WavetableSet::factory();
        let f = set.table(28).frame(0, 0);
        assert!(harmonic_power(f, 2) < 1.0e-6);
        assert!(harmonic_power(f, 1) > 1.0e-3);
    }

    #[test]
    fn solid_sub_frame_zero_is_nearly_sine() {
        let set = WavetableSet::factory();
        let f = set.table(31).frame(0, 0);
        let fund = harmonic_power(f, 1);
        let third = harmonic_power(f, 3);
        assert!(fund > 0.5);
        assert!(third < fund * 0.05, "third/fund = {}", third / fund);
    }

    #[test]
    fn spectral_noise_frames_differ() {
        let set = WavetableSet::factory();
        let a = set.table(30).frame(0, 0);
        let b = set.table(30).frame(FRAMES - 1, 0);
        let diff: f64 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| ((x - y) as f64).abs())
            .sum::<f64>()
            / a.len() as f64;
        assert!(diff > 0.05, "noise frames should be distinct: {diff}");
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
