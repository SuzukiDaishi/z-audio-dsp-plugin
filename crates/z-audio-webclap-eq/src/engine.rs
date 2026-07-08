//! The pro EQ engine: 8 bands of RBJ biquads with cut slopes up to
//! 48 dB/oct, per-band Stereo/Mid/Side/Left/Right placement, band-solo
//! listen filters, output trim, and an FFT spectrum tap for the UI.
//!
//! Everything here is allocation-free on the audio path; coefficients are
//! recomputed on parameter changes only.

use crate::params::*;

pub const MAX_STAGES: usize = 4;

// ---------------------------------------------------------------------------
// Parameters.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct BandParams {
    pub enabled: bool,
    pub ftype: u8,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub slope: u8,
    pub placement: u8,
    pub solo: bool,
}

impl BandParams {
    fn default_for(band: usize) -> Self {
        Self {
            enabled: false,
            ftype: TYPE_BELL,
            freq_hz: DEFAULT_FREQS[band.min(DEFAULT_FREQS.len() - 1)],
            gain_db: 0.0,
            q: 0.71,
            slope: SLOPE_12,
            placement: PLACE_STEREO,
            solo: false,
        }
    }
}

#[derive(Clone, Copy)]
pub struct EqParams {
    pub bands: [BandParams; BAND_COUNT as usize],
    pub output_db: f32,
}

impl Default for EqParams {
    fn default() -> Self {
        let mut bands = [BandParams::default_for(0); BAND_COUNT as usize];
        for (i, band) in bands.iter_mut().enumerate() {
            *band = BandParams::default_for(i);
        }
        Self {
            bands,
            output_db: 0.0,
        }
    }
}

pub fn apply_param(p: &mut EqParams, id: u32, value: f64) {
    let v = value as f32;
    if id == P_OUTPUT {
        p.output_db = v.clamp(-24.0, 24.0);
        return;
    }
    if !(BAND_BASE..BAND_BASE + BAND_COUNT * BAND_FIELDS).contains(&id) {
        return;
    }
    let band = &mut p.bands[((id - BAND_BASE) / BAND_FIELDS) as usize];
    match (id - BAND_BASE) % BAND_FIELDS {
        F_ENABLE => band.enabled = v >= 0.5,
        F_TYPE => band.ftype = v.clamp(0.0, (TYPE_COUNT - 1) as f32).round() as u8,
        F_FREQ => band.freq_hz = v.clamp(FREQ_MIN, FREQ_MAX),
        F_GAIN => band.gain_db = v.clamp(GAIN_MIN, GAIN_MAX),
        F_Q => band.q = v.clamp(Q_MIN, Q_MAX),
        F_SLOPE => band.slope = v.clamp(0.0, (SLOPE_COUNT - 1) as f32).round() as u8,
        F_PLACEMENT => band.placement = v.clamp(0.0, (PLACE_COUNT - 1) as f32).round() as u8,
        F_SOLO => band.solo = v >= 0.5,
        _ => {}
    }
}

pub fn param_value(p: &EqParams, id: u32) -> f64 {
    if id == P_OUTPUT {
        return p.output_db as f64;
    }
    if !(BAND_BASE..BAND_BASE + BAND_COUNT * BAND_FIELDS).contains(&id) {
        return 0.0;
    }
    let band = &p.bands[((id - BAND_BASE) / BAND_FIELDS) as usize];
    (match (id - BAND_BASE) % BAND_FIELDS {
        F_ENABLE => band.enabled as u8 as f32,
        F_TYPE => band.ftype as f32,
        F_FREQ => band.freq_hz,
        F_GAIN => band.gain_db,
        F_Q => band.q,
        F_SLOPE => band.slope as f32,
        F_PLACEMENT => band.placement as f32,
        F_SOLO => band.solo as u8 as f32,
        _ => 0.0,
    }) as f64
}

// ---------------------------------------------------------------------------
// Biquads (RBJ cookbook, coefficients normalized so a0 == 1).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct Coeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn norm(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    pub fn peaking(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Self::norm(
            1.0 + alpha * a,
            -2.0 * cw,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cw,
            1.0 - alpha / a,
        )
    }

    pub fn low_shelf(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        let sq = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) - (a - 1.0) * cw + sq),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
            a * ((a + 1.0) - (a - 1.0) * cw - sq),
            (a + 1.0) + (a - 1.0) * cw + sq,
            -2.0 * ((a - 1.0) + (a + 1.0) * cw),
            (a + 1.0) + (a - 1.0) * cw - sq,
        )
    }

    pub fn high_shelf(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        let sq = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) + (a - 1.0) * cw + sq),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
            a * ((a + 1.0) + (a - 1.0) * cw - sq),
            (a + 1.0) - (a - 1.0) * cw + sq,
            2.0 * ((a - 1.0) - (a + 1.0) * cw),
            (a + 1.0) - (a - 1.0) * cw - sq,
        )
    }

    pub fn low_pass(fs: f32, f0: f32, q: f32) -> Self {
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Self::norm(
            (1.0 - cw) / 2.0,
            1.0 - cw,
            (1.0 - cw) / 2.0,
            1.0 + alpha,
            -2.0 * cw,
            1.0 - alpha,
        )
    }

    pub fn high_pass(fs: f32, f0: f32, q: f32) -> Self {
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Self::norm(
            (1.0 + cw) / 2.0,
            -(1.0 + cw),
            (1.0 + cw) / 2.0,
            1.0 + alpha,
            -2.0 * cw,
            1.0 - alpha,
        )
    }

    pub fn notch(fs: f32, f0: f32, q: f32) -> Self {
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Self::norm(
            1.0,
            -2.0 * cw,
            1.0,
            1.0 + alpha,
            -2.0 * cw,
            1.0 - alpha,
        )
    }

    /// Constant-peak band pass (0 dB at f0), used by the band-solo listen.
    pub fn band_pass(fs: f32, f0: f32, q: f32) -> Self {
        let w0 = angular(fs, f0);
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Self::norm(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cw, 1.0 - alpha)
    }

    /// 6 dB/oct one-pole low pass expressed as a (degenerate) biquad.
    pub fn one_pole_low(fs: f32, f0: f32) -> Self {
        let p = (-angular(fs, f0)).exp();
        Self {
            b0: 1.0 - p,
            b1: 0.0,
            b2: 0.0,
            a1: -p,
            a2: 0.0,
        }
    }

    /// 6 dB/oct one-pole high pass expressed as a (degenerate) biquad.
    pub fn one_pole_high(fs: f32, f0: f32) -> Self {
        let p = (-angular(fs, f0)).exp();
        let g = (1.0 + p) / 2.0;
        Self {
            b0: g,
            b1: -g,
            b2: 0.0,
            a1: -p,
            a2: 0.0,
        }
    }

    /// Magnitude response at `f` — mirrored by the UI curve renderer.
    pub fn magnitude(&self, fs: f32, f: f32) -> f32 {
        let w = core::f32::consts::TAU * f / fs;
        let cw = w.cos();
        let c2w = (2.0 * w).cos();
        let num = self.b0 * self.b0
            + self.b1 * self.b1
            + self.b2 * self.b2
            + 2.0 * (self.b0 * self.b1 + self.b1 * self.b2) * cw
            + 2.0 * self.b0 * self.b2 * c2w;
        let den = 1.0
            + self.a1 * self.a1
            + self.a2 * self.a2
            + 2.0 * (self.a1 + self.a1 * self.a2) * cw
            + 2.0 * self.a2 * c2w;
        (num.max(0.0) / den.max(1e-12)).sqrt()
    }
}

fn angular(fs: f32, f0: f32) -> f32 {
    core::f32::consts::TAU * (f0.clamp(1.0, fs * 0.49)) / fs
}

#[derive(Clone, Copy, Default)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    /// Transposed direct form II.
    #[inline]
    fn tick(&mut self, c: &Coeffs, x: f32) -> f32 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

// ---------------------------------------------------------------------------
// Band cascades.
// ---------------------------------------------------------------------------

/// Butterworth per-stage Q values for cascaded 12 dB sections.
const Q_24: [f32; 2] = [0.5412, 1.3066];
const Q_48: [f32; 4] = [0.5098, 0.6013, 0.9000, 2.5629];

/// Compiled coefficients for one band: up to 4 stages plus the solo-listen
/// filter. Exposed so the UI-response tests can pin the same math.
#[derive(Clone, Copy)]
pub struct BandKernel {
    pub stages: [Coeffs; MAX_STAGES],
    pub stage_count: usize,
    pub listen: Coeffs,
}

impl Default for BandKernel {
    fn default() -> Self {
        Self {
            stages: [Coeffs::IDENTITY; MAX_STAGES],
            stage_count: 0,
            listen: Coeffs::IDENTITY,
        }
    }
}

/// Builds the stage cascade for a band — the single source of truth for
/// how band parameters map to filters (the UI mirrors it via magnitudes).
pub fn band_kernel(fs: f32, band: &BandParams) -> BandKernel {
    let mut kernel = BandKernel::default();
    let f = band.freq_hz;
    let q = band.q;
    match band.ftype {
        TYPE_LOW_SHELF => {
            kernel.stages[0] = Coeffs::low_shelf(fs, f, q.max(0.3), band.gain_db);
            kernel.stage_count = 1;
            kernel.listen = Coeffs::low_pass(fs, f, 0.7071);
        }
        TYPE_HIGH_SHELF => {
            kernel.stages[0] = Coeffs::high_shelf(fs, f, q.max(0.3), band.gain_db);
            kernel.stage_count = 1;
            kernel.listen = Coeffs::high_pass(fs, f, 0.7071);
        }
        TYPE_LOW_CUT => {
            build_cut(&mut kernel, fs, f, q, band.slope, /*high_pass=*/ true);
            kernel.listen = Coeffs::low_pass(fs, f, 0.7071);
        }
        TYPE_HIGH_CUT => {
            build_cut(&mut kernel, fs, f, q, band.slope, /*high_pass=*/ false);
            kernel.listen = Coeffs::high_pass(fs, f, 0.7071);
        }
        TYPE_NOTCH => {
            kernel.stages[0] = Coeffs::notch(fs, f, q);
            kernel.stage_count = 1;
            kernel.listen = Coeffs::band_pass(fs, f, q.max(0.5));
        }
        _ => {
            kernel.stages[0] = Coeffs::peaking(fs, f, q, band.gain_db);
            kernel.stage_count = 1;
            kernel.listen = Coeffs::band_pass(fs, f, q.max(0.5));
        }
    }
    kernel
}

fn build_cut(kernel: &mut BandKernel, fs: f32, f: f32, q: f32, slope: u8, high_pass: bool) {
    let reso = (q / 0.7071).max(0.14); // band Q scales the resonant stage
    let make = |stage_q: f32| {
        if high_pass {
            Coeffs::high_pass(fs, f, stage_q)
        } else {
            Coeffs::low_pass(fs, f, stage_q)
        }
    };
    match slope {
        SLOPE_6 => {
            kernel.stages[0] = if high_pass {
                Coeffs::one_pole_high(fs, f)
            } else {
                Coeffs::one_pole_low(fs, f)
            };
            kernel.stage_count = 1;
        }
        SLOPE_24 => {
            kernel.stages[0] = make(Q_24[0]);
            kernel.stages[1] = make(Q_24[1] * reso);
            kernel.stage_count = 2;
        }
        SLOPE_48 => {
            kernel.stages[0] = make(Q_48[0]);
            kernel.stages[1] = make(Q_48[1]);
            kernel.stages[2] = make(Q_48[2]);
            kernel.stages[3] = make(Q_48[3] * reso);
            kernel.stage_count = 4;
        }
        _ => {
            kernel.stages[0] = make(0.7071 * reso);
            kernel.stage_count = 1;
        }
    }
}

/// dB response of one band at `f` (product of its stages).
pub fn band_response_db(fs: f32, band: &BandParams, f: f32) -> f32 {
    let kernel = band_kernel(fs, band);
    let mut mag = 1.0f32;
    for stage in &kernel.stages[..kernel.stage_count] {
        mag *= stage.magnitude(fs, f);
    }
    20.0 * mag.max(1e-9).log10()
}

// ---------------------------------------------------------------------------
// Engine.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct BandRuntime {
    kernel: BandKernel,
    // Two lanes: stereo/L-R use lane 0 = left, lane 1 = right; mid/side use
    // lane 0 for the processed component.
    states: [[BiquadState; 2]; MAX_STAGES],
    listen_states: [BiquadState; 2],
}

pub struct EqEngine {
    params: EqParams,
    sample_rate: f32,
    bands: [BandRuntime; BAND_COUNT as usize],
}

impl EqEngine {
    pub fn new(sample_rate: f32) -> Self {
        let mut engine = Self {
            params: EqParams::default(),
            sample_rate: sample_rate.max(1.0),
            bands: [BandRuntime::default(); BAND_COUNT as usize],
        };
        engine.rebuild_all();
        engine
    }

    pub fn params(&self) -> &EqParams {
        &self.params
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn set_params(&mut self, p: EqParams) {
        self.params = p;
        self.rebuild_all();
    }

    pub fn reset(&mut self) {
        for band in &mut self.bands {
            band.states = [[BiquadState::default(); 2]; MAX_STAGES];
            band.listen_states = [BiquadState::default(); 2];
        }
    }

    fn rebuild_all(&mut self) {
        for i in 0..BAND_COUNT as usize {
            self.bands[i].kernel = band_kernel(self.sample_rate, &self.params.bands[i]);
        }
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let out_gain = 10.0_f32.powf(self.params.output_db / 20.0);
        let any_solo = self
            .params
            .bands
            .iter()
            .any(|b| b.enabled && b.solo);

        for i in 0..out_l.len() {
            let mut l = in_l[i];
            let mut r = in_r[i];

            for (bi, band) in self.params.bands.iter().enumerate() {
                if !band.enabled {
                    continue;
                }
                let rt = &mut self.bands[bi];
                let n = rt.kernel.stage_count;
                match band.placement {
                    PLACE_MID | PLACE_SIDE => {
                        let mid = (l + r) * 0.5;
                        let side = (l - r) * 0.5;
                        let (mut x, other) = if band.placement == PLACE_MID {
                            (mid, side)
                        } else {
                            (side, mid)
                        };
                        for s in 0..n {
                            x = rt.states[s][0].tick(&rt.kernel.stages[s], x);
                        }
                        let (mid, side) = if band.placement == PLACE_MID {
                            (x, other)
                        } else {
                            (other, x)
                        };
                        l = mid + side;
                        r = mid - side;
                    }
                    PLACE_LEFT => {
                        for s in 0..n {
                            l = rt.states[s][0].tick(&rt.kernel.stages[s], l);
                        }
                    }
                    PLACE_RIGHT => {
                        for s in 0..n {
                            r = rt.states[s][1].tick(&rt.kernel.stages[s], r);
                        }
                    }
                    _ => {
                        for s in 0..n {
                            l = rt.states[s][0].tick(&rt.kernel.stages[s], l);
                            r = rt.states[s][1].tick(&rt.kernel.stages[s], r);
                        }
                    }
                }
            }

            // Band solo: replace the output with the soloed bands' regions
            // of the processed signal, so "listen to just this band" hears
            // the EQ'd result inside the band's range.
            if any_solo {
                let mut sl = 0.0f32;
                let mut sr = 0.0f32;
                for (bi, band) in self.params.bands.iter().enumerate() {
                    if !band.enabled || !band.solo {
                        continue;
                    }
                    let rt = &mut self.bands[bi];
                    sl += rt.listen_states[0].tick(&rt.kernel.listen, l);
                    sr += rt.listen_states[1].tick(&rt.kernel.listen, r);
                }
                l = sl;
                r = sr;
            }

            out_l[i] = l * out_gain;
            out_r[i] = r * out_gain;
        }
    }
}

// ---------------------------------------------------------------------------
// Spectrum tap: Hann-windowed 2048-point FFT pushed to the UI.
// ---------------------------------------------------------------------------

pub const FFT_SIZE: usize = 2048;
pub const FFT_BINS: usize = FFT_SIZE / 2;
const FFT_HOP: usize = 2048;

pub struct SpectrumTap {
    ring: Vec<f32>,
    write: usize,
    since_frame: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    bit_rev: Vec<u16>,
    /// dB magnitudes for the most recent frame.
    pub frame_db: Vec<f32>,
    pub frame_ready: bool,
}

impl SpectrumTap {
    pub fn new() -> Self {
        let mut window = vec![0.0f32; FFT_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            *w = 0.5
                - 0.5 * (core::f32::consts::TAU * i as f32 / (FFT_SIZE as f32 - 1.0)).cos();
        }
        let bits = FFT_SIZE.trailing_zeros();
        let bit_rev = (0..FFT_SIZE)
            .map(|i| (i.reverse_bits() >> (usize::BITS - bits)) as u16)
            .collect();
        Self {
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            since_frame: 0,
            window,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            bit_rev,
            frame_db: vec![-120.0; FFT_BINS],
            frame_ready: false,
        }
    }

    /// Feeds mono samples; sets `frame_ready` when a new frame is computed.
    pub fn push(&mut self, l: &[f32], r: &[f32]) {
        for i in 0..l.len() {
            self.ring[self.write] = (l[i] + r[i]) * 0.5;
            self.write = (self.write + 1) % FFT_SIZE;
            self.since_frame += 1;
        }
        if self.since_frame >= FFT_HOP {
            self.since_frame = 0;
            self.compute();
            self.frame_ready = true;
        }
    }

    fn compute(&mut self) {
        // Unroll the ring into re[] in time order, windowed.
        for i in 0..FFT_SIZE {
            let src = (self.write + i) % FFT_SIZE;
            self.re[i] = self.ring[src] * self.window[i];
            self.im[i] = 0.0;
        }
        fft_in_place(&mut self.re, &mut self.im, &self.bit_rev);
        // Hann coherent gain is 0.5; normalize a full-scale sine to 0 dB.
        let scale = 2.0 / (FFT_SIZE as f32 * 0.5);
        for bin in 0..FFT_BINS {
            let mag = (self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin]).sqrt() * scale;
            self.frame_db[bin] = 20.0 * mag.max(1e-7).log10();
        }
    }
}

/// Iterative radix-2 Cooley-Tukey.
fn fft_in_place(re: &mut [f32], im: &mut [f32], bit_rev: &[u16]) {
    let n = re.len();
    for i in 0..n {
        let j = bit_rev[i] as usize;
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -core::f32::consts::TAU / len as f32;
        let (wl_im, wl_re) = ang.sin_cos();
        let mut start = 0;
        while start < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let t_re = re[b] * w_re - im[b] * w_im;
                let t_im = re[b] * w_im + im[b] * w_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next_re = w_re * wl_re - w_im * wl_im;
                w_im = w_re * wl_im + w_im * wl_re;
                w_re = next_re;
            }
            start += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, fs: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * freq * i as f32 / fs).sin() * amp)
            .collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len().max(1) as f32).sqrt()
    }

    /// Runs `input` through the engine (same signal both channels) and
    /// returns the steady-state RMS of the last half of the left output.
    fn run(engine: &mut EqEngine, input: &[f32]) -> f32 {
        let n = input.len();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        engine.process(input, input, &mut l, &mut r);
        rms(&l[n / 2..])
    }

    fn enabled_band(ftype: u8, freq: f32, gain: f32, q: f32) -> BandParams {
        BandParams {
            enabled: true,
            ftype,
            freq_hz: freq,
            gain_db: gain,
            q,
            slope: SLOPE_12,
            placement: PLACE_STEREO,
            solo: false,
        }
    }

    #[test]
    fn flat_config_is_a_passthrough() {
        let mut e = EqEngine::new(48_000.0);
        let input = sine(1_000.0, 48_000.0, 4_800, 0.5);
        let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn bell_boosts_its_center_and_leaves_the_far_field() {
        let mut p = EqParams::default();
        p.bands[0] = enabled_band(TYPE_BELL, 1_000.0, 12.0, 1.0);
        let center = {
            let mut e = EqEngine::new(48_000.0);
            e.set_params(p);
            run(&mut e, &sine(1_000.0, 48_000.0, 9_600, 0.1))
        };
        let far = {
            let mut e = EqEngine::new(48_000.0);
            e.set_params(p);
            run(&mut e, &sine(60.0, 48_000.0, 9_600, 0.1))
        };
        let center_db = 20.0 * (center / 0.0707).log10();
        let far_db = 20.0 * (far / 0.0707).log10();
        assert!((center_db - 12.0).abs() < 0.5, "center {center_db} dB");
        assert!(far_db.abs() < 1.0, "far field {far_db} dB");
    }

    #[test]
    fn low_cut_slopes_attenuate_progressively() {
        // 1 kHz low cut, measure a 250 Hz sine (2 octaves below).
        let mut previous = f32::MAX;
        for slope in [SLOPE_6, SLOPE_12, SLOPE_24, SLOPE_48] {
            let mut p = EqParams::default();
            p.bands[0] = BandParams {
                slope,
                ..enabled_band(TYPE_LOW_CUT, 1_000.0, 0.0, 0.71)
            };
            let mut e = EqEngine::new(48_000.0);
            e.set_params(p);
            let level = run(&mut e, &sine(250.0, 48_000.0, 19_200, 0.5));
            assert!(
                level < previous * 0.6,
                "slope {slope}: {level} not steeper than {previous}"
            );
            previous = level;
        }
        // 48 dB/oct two octaves down: ~96 dB — effectively silent.
        assert!(previous < 0.001);
    }

    #[test]
    fn notch_kills_its_center() {
        let mut p = EqParams::default();
        p.bands[0] = enabled_band(TYPE_NOTCH, 1_000.0, 0.0, 4.0);
        let mut e = EqEngine::new(48_000.0);
        e.set_params(p);
        let level = run(&mut e, &sine(1_000.0, 48_000.0, 48_000, 0.5));
        assert!(level < 0.01, "notch leaked {level}");
    }

    #[test]
    fn band_solo_isolates_the_band_region() {
        let mut p = EqParams::default();
        p.bands[0] = BandParams {
            solo: true,
            ..enabled_band(TYPE_BELL, 1_000.0, 0.0, 4.0)
        };
        let inside = {
            let mut e = EqEngine::new(48_000.0);
            e.set_params(p);
            run(&mut e, &sine(1_000.0, 48_000.0, 19_200, 0.5))
        };
        let outside = {
            let mut e = EqEngine::new(48_000.0);
            e.set_params(p);
            run(&mut e, &sine(100.0, 48_000.0, 19_200, 0.5))
        };
        assert!((inside - 0.3535).abs() < 0.05, "in-band level {inside}");
        assert!(
            outside < inside * 0.12,
            "out-of-band leak {outside} vs {inside}"
        );
    }

    #[test]
    fn side_placement_leaves_mono_content_untouched() {
        let mut p = EqParams::default();
        p.bands[0] = BandParams {
            placement: PLACE_SIDE,
            ..enabled_band(TYPE_BELL, 1_000.0, 18.0, 1.0)
        };
        let mut e = EqEngine::new(48_000.0);
        e.set_params(p);
        // Mono input: side channel is zero, so an 18 dB side bell must not
        // change anything.
        let input = sine(1_000.0, 48_000.0, 9_600, 0.4);
        let (mut l, mut r) = (vec![0.0; 9_600], vec![0.0; 9_600]);
        e.process(&input, &input, &mut l, &mut r);
        for (a, b) in input.iter().zip(l.iter()) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn left_placement_only_touches_the_left_channel() {
        let mut p = EqParams::default();
        p.bands[0] = BandParams {
            placement: PLACE_LEFT,
            ..enabled_band(TYPE_BELL, 1_000.0, 12.0, 1.0)
        };
        let mut e = EqEngine::new(48_000.0);
        e.set_params(p);
        let input = sine(1_000.0, 48_000.0, 19_200, 0.1);
        let (mut l, mut r) = (vec![0.0; 19_200], vec![0.0; 19_200]);
        e.process(&input, &input, &mut l, &mut r);
        assert!(rms(&l[9_600..]) > rms(&r[9_600..]) * 2.0);
        for (a, b) in input.iter().zip(r.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn output_trim_scales_everything() {
        let mut p = EqParams::default();
        p.output_db = -6.0;
        let mut e = EqEngine::new(48_000.0);
        e.set_params(p);
        let input = vec![1.0f32; 64];
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        e.process(&input, &input, &mut l, &mut r);
        assert!((l[10] - 0.501).abs() < 0.01);
    }

    #[test]
    fn band_response_matches_processed_gain() {
        // The analytic response the UI draws must match what the audio
        // path actually does.
        let band = enabled_band(TYPE_BELL, 2_000.0, -9.0, 2.0);
        let mut p = EqParams::default();
        p.bands[0] = band;
        let mut e = EqEngine::new(48_000.0);
        e.set_params(p);
        let level = run(&mut e, &sine(2_000.0, 48_000.0, 19_200, 0.2));
        let measured_db = 20.0 * (level / (0.2 / core::f32::consts::SQRT_2)).log10();
        let analytic_db = band_response_db(48_000.0, &band, 2_000.0);
        assert!(
            (measured_db - analytic_db).abs() < 0.5,
            "measured {measured_db} vs analytic {analytic_db}"
        );
    }

    #[test]
    fn spectrum_tap_finds_a_sine_peak() {
        let mut tap = SpectrumTap::new();
        let fs = 48_000.0;
        let freq = 1_500.0;
        let input = sine(freq, fs, FFT_SIZE * 2, 0.8);
        tap.push(&input, &input);
        assert!(tap.frame_ready);
        let peak_bin = (0..FFT_BINS)
            .max_by(|a, b| tap.frame_db[*a].total_cmp(&tap.frame_db[*b]))
            .unwrap();
        let peak_hz = peak_bin as f32 * fs / FFT_SIZE as f32;
        assert!(
            (peak_hz - freq).abs() < fs / FFT_SIZE as f32 * 2.0,
            "peak at {peak_hz} Hz"
        );
        // ~0.8 full scale ≈ -1.9 dB.
        assert!(
            (tap.frame_db[peak_bin] - (-1.9)).abs() < 1.5,
            "peak {} dB",
            tap.frame_db[peak_bin]
        );
    }
}
