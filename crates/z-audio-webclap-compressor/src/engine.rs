//! Enhanced feed-forward compressor engine, shared by the WCLAP build
//! (this crate) and the native VST3/CLAP build (`z-audio-compressor-plugin`).
//!
//! Upgrades over the plain `z_audio_dsp::Compressor`:
//!
//! - **Log-domain ballistics** — attack/release smoothing runs on the gain
//!   reduction in dB, with a proper attack ramp instead of an instant snap,
//!   so hard-hit transients stop crackling.
//! - **Program-dependent auto release** — two parallel release followers
//!   (fast + slow) blend into a dual-slope recovery: punchy on transients,
//!   smooth on sustained material (SSL-bus style).
//! - **Sidechain high-pass** — a 2nd-order Butterworth HPF on the detector
//!   keeps bass energy from pumping the whole mix.
//! - **Lookahead** — up to 10 ms of detector lead time (audio is delayed and
//!   the latency reported to the host) so fast peaks are caught cleanly.
//! - **Auto makeup** — gain compensation derived from the static curve.
//! - **Warmth** — an optional unity-slope `tanh` saturator on the wet path
//!   that rounds peaks with low-order harmonics.
//!
//! The static soft-knee gain computer intentionally matches the transfer
//! curve drawn by the UI (`ui/main.js`).

use z_audio_dsp::DetectorMode;

/// Hard cap on the detector lead time (and thus reported latency).
pub const MAX_LOOKAHEAD_MS: f32 = 10.0;
/// Sidechain HPF frequencies at or below this are treated as "off".
pub const SC_HPF_OFF_HZ: f32 = 20.0;

/// Fraction of the static-curve makeup applied in auto-makeup mode.
/// Full compensation (1.0) overshoots perceived loudness on real program
/// material; half-way is the usual mix-friendly compromise.
const AUTO_MAKEUP_SCALE: f32 = 0.5;
/// Mean-square averaging window of the RMS detector.
const RMS_WINDOW_MS: f32 = 25.0;
/// Fast release leg of the auto-release blend, relative to `release_ms`.
const AUTO_FAST_SCALE: f32 = 0.35;
/// Slow release leg of the auto-release blend, relative to `release_ms`.
const AUTO_SLOW_SCALE: f32 = 2.5;
/// Weight of the fast leg in the auto-release blend.
const AUTO_FAST_WEIGHT: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnhancedCompressorParams {
    pub input_gain_db: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_gain_db: f32,
    pub mix: f32,
    pub detector_mode: DetectorMode,
    pub stereo_link: f32,
    pub sc_hpf_hz: f32,
    pub lookahead_ms: f32,
    pub auto_release: bool,
    pub auto_makeup: bool,
    pub warmth: f32,
}

impl Default for EnhancedCompressorParams {
    fn default() -> Self {
        Self {
            input_gain_db: 0.0,
            threshold_db: -18.0,
            ratio: 4.0,
            knee_db: 0.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            makeup_gain_db: 0.0,
            mix: 1.0,
            detector_mode: DetectorMode::Peak,
            stereo_link: 1.0,
            sc_hpf_hz: SC_HPF_OFF_HZ,
            lookahead_ms: 0.0,
            auto_release: true,
            auto_makeup: false,
            warmth: 0.15,
        }
    }
}

/// RBJ 2nd-order Butterworth high-pass, direct form 1.
#[derive(Debug, Clone, Copy, Default)]
struct HighPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl HighPass {
    fn configure(&mut self, sample_rate: f32, freq_hz: f32) {
        let freq = freq_hz.clamp(10.0, sample_rate * 0.45);
        let w0 = core::f32::consts::TAU * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / core::f32::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 + cos) * 0.5 / a0;
        self.b1 = -(1.0 + cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = flush(y);
        self.y1
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Per-channel detector + gain-reduction state.
#[derive(Debug, Clone, Copy, Default)]
struct ChannelState {
    hpf: HighPass,
    mean_square: f32,
    gr_fast_db: f32,
    gr_slow_db: f32,
}

impl ChannelState {
    fn reset(&mut self) {
        self.hpf.reset();
        self.mean_square = 0.0;
        self.gr_fast_db = 0.0;
        self.gr_slow_db = 0.0;
    }
}

pub struct EnhancedCompressor {
    sample_rate: f32,
    params: EnhancedCompressorParams,
    ch: [ChannelState; 2],
    // Cached per-sample coefficients, rebuilt in `configure`.
    rms_coeff: f32,
    attack_coeff: f32,
    release_fast_coeff: f32,
    release_slow_coeff: f32,
    hpf_on: bool,
    makeup_total_db: f32,
    // Lookahead delay for the (raw) audio path, one ring per channel.
    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    write: usize,
    delay_samples: usize,
    // Block-max gain reduction for metering; consumed by `take_gr_meter`.
    meter_gr_db: f32,
}

impl Default for EnhancedCompressor {
    fn default() -> Self {
        Self::new(EnhancedCompressorParams::default())
    }
}

impl EnhancedCompressor {
    pub fn new(params: EnhancedCompressorParams) -> Self {
        let mut comp = Self {
            sample_rate: 48_000.0,
            params,
            ch: [ChannelState::default(); 2],
            rms_coeff: 0.0,
            attack_coeff: 1.0,
            release_fast_coeff: 0.0,
            release_slow_coeff: 0.0,
            hpf_on: false,
            makeup_total_db: 0.0,
            delay_l: Vec::new(),
            delay_r: Vec::new(),
            write: 0,
            delay_samples: 0,
            meter_gr_db: 0.0,
        };
        comp.prepare(48_000.0, 512);
        comp
    }

    pub fn prepare(&mut self, sample_rate: f32, _max_block_size: usize) {
        self.sample_rate = sample_rate.max(1.0);
        let capacity = (self.sample_rate * MAX_LOOKAHEAD_MS / 1_000.0).ceil() as usize + 1;
        self.delay_l = vec![0.0; capacity];
        self.delay_r = vec![0.0; capacity];
        self.write = 0;
        self.params = sanitize(self.params);
        self.configure();
        self.reset();
    }

    pub fn set_params(&mut self, params: EnhancedCompressorParams) {
        self.params = sanitize(params);
        self.configure();
    }

    pub fn params(&self) -> EnhancedCompressorParams {
        self.params
    }

    pub fn reset(&mut self) {
        for ch in &mut self.ch {
            ch.reset();
        }
        self.delay_l.fill(0.0);
        self.delay_r.fill(0.0);
        self.write = 0;
        self.meter_gr_db = 0.0;
    }

    /// Detector lead time in samples — what the host should compensate.
    pub fn latency_samples(&self) -> u32 {
        self.delay_samples as u32
    }

    /// Block-max gain reduction (positive dB) since the last call.
    pub fn take_gr_meter(&mut self) -> f32 {
        let gr = self.meter_gr_db;
        self.meter_gr_db = 0.0;
        gr
    }

    fn configure(&mut self) {
        let p = &self.params;
        self.rms_coeff = one_pole_coeff(self.sample_rate, RMS_WINDOW_MS);
        self.attack_coeff = one_pole_coeff(self.sample_rate, p.attack_ms);
        let (fast_ms, slow_ms) = if p.auto_release {
            (
                (p.release_ms * AUTO_FAST_SCALE).max(20.0),
                p.release_ms * AUTO_SLOW_SCALE,
            )
        } else {
            (p.release_ms, p.release_ms)
        };
        self.release_fast_coeff = one_pole_coeff(self.sample_rate, fast_ms);
        self.release_slow_coeff = one_pole_coeff(self.sample_rate, slow_ms);

        self.hpf_on = p.sc_hpf_hz > SC_HPF_OFF_HZ + 0.5;
        if self.hpf_on {
            for ch in &mut self.ch {
                ch.hpf.configure(self.sample_rate, p.sc_hpf_hz);
            }
        }

        self.makeup_total_db = p.makeup_gain_db
            + if p.auto_makeup {
                -AUTO_MAKEUP_SCALE * compressor_gain_db(0.0, p.threshold_db, p.ratio, p.knee_db)
            } else {
                0.0
            };

        let max_delay = self.delay_l.len().saturating_sub(1);
        self.delay_samples =
            ((self.sample_rate * p.lookahead_ms / 1_000.0).round() as usize).min(max_delay);
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let p = self.params;
        let input_gain = db_to_linear(p.input_gain_db);
        let makeup = self.makeup_total_db;
        let mix = p.mix;
        let rms = p.detector_mode == DetectorMode::Rms;
        let warmth = p.warmth;
        let drive = 1.0 + 2.0 * warmth;
        let ring_len = self.delay_l.len();

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let raw_l = *l;
            let raw_r = *r;
            let in_l = raw_l * input_gain;
            let in_r = raw_r * input_gain;

            // Sidechain: optional HPF, then peak or RMS level per channel.
            let sc_l = if self.hpf_on {
                self.ch[0].hpf.process(in_l)
            } else {
                in_l
            };
            let sc_r = if self.hpf_on {
                self.ch[1].hpf.process(in_r)
            } else {
                in_r
            };
            let det_l = if rms {
                let ms = &mut self.ch[0].mean_square;
                *ms = flush(*ms + self.rms_coeff * (sc_l * sc_l - *ms));
                ms.sqrt()
            } else {
                sc_l.abs()
            };
            let det_r = if rms {
                let ms = &mut self.ch[1].mean_square;
                *ms = flush(*ms + self.rms_coeff * (sc_r * sc_r - *ms));
                ms.sqrt()
            } else {
                sc_r.abs()
            };
            let linked = det_l.max(det_r);
            let lvl_l = det_l + (linked - det_l) * p.stereo_link;
            let lvl_r = det_r + (linked - det_r) * p.stereo_link;

            let gr_l = self.gain_reduction_db(0, lvl_l, &p);
            let gr_r = self.gain_reduction_db(1, lvl_r, &p);
            self.meter_gr_db = self.meter_gr_db.max(gr_l.max(gr_r));

            // Lookahead: the detector runs on "now" while the audio path
            // (wet AND dry, so mix stays phase-aligned) reads the ring
            // `delay_samples` behind.
            self.delay_l[self.write] = raw_l;
            self.delay_r[self.write] = raw_r;
            let read = (self.write + ring_len - self.delay_samples) % ring_len;
            let dly_l = self.delay_l[read];
            let dly_r = self.delay_r[read];
            self.write = (self.write + 1) % ring_len;

            let gain_l = db_to_linear(makeup - gr_l);
            let gain_r = db_to_linear(makeup - gr_r);
            let mut wet_l = dly_l * input_gain * gain_l;
            let mut wet_r = dly_r * input_gain * gain_r;
            if warmth > 0.0 {
                wet_l += warmth * ((drive * wet_l).tanh() / drive - wet_l);
                wet_r += warmth * ((drive * wet_r).tanh() / drive - wet_r);
            }

            *l = flush(dly_l * (1.0 - mix) + wet_l * mix);
            *r = flush(dly_r * (1.0 - mix) + wet_r * mix);
        }
    }

    /// Static curve + smooth-branching ballistics for one channel.
    /// Returns the smoothed gain reduction in positive dB.
    fn gain_reduction_db(&mut self, ch: usize, level: f32, p: &EnhancedCompressorParams) -> f32 {
        let level_db = linear_to_db(level);
        let target = -compressor_gain_db(level_db, p.threshold_db, p.ratio, p.knee_db);
        let state = &mut self.ch[ch];
        state.gr_fast_db = flush(branch(
            state.gr_fast_db,
            target,
            self.attack_coeff,
            self.release_fast_coeff,
        ));
        if p.auto_release {
            state.gr_slow_db = flush(branch(
                state.gr_slow_db,
                target,
                self.attack_coeff,
                self.release_slow_coeff,
            ));
            AUTO_FAST_WEIGHT * state.gr_fast_db + (1.0 - AUTO_FAST_WEIGHT) * state.gr_slow_db
        } else {
            state.gr_fast_db
        }
    }
}

/// Smooth-branching one-pole: attack coefficient while the target rises,
/// release coefficient while it falls.
#[inline]
fn branch(state: f32, target: f32, attack: f32, release: f32) -> f32 {
    let coeff = if target > state { attack } else { release };
    state + coeff * (target - state)
}

/// One-pole smoothing coefficient for a time constant in milliseconds.
#[inline]
fn one_pole_coeff(sample_rate: f32, tau_ms: f32) -> f32 {
    let tau = (tau_ms * 1.0e-3).max(1.0e-6);
    1.0 - (-1.0 / (sample_rate * tau)).exp()
}

#[inline]
fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db * 0.05)
}

#[inline]
fn linear_to_db(linear: f32) -> f32 {
    20.0 * linear.max(1.0e-7).log10()
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

/// Soft-knee static gain computer. Returns the (non-positive) gain change
/// in dB for a detector level in dB. Mirrored by the UI transfer curve.
pub fn compressor_gain_db(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let ratio = ratio.max(1.0);
    if ratio <= 1.0 {
        return 0.0;
    }

    let knee = knee_db.max(0.0);
    if knee <= 0.0 {
        if level_db <= threshold_db {
            0.0
        } else {
            threshold_db + (level_db - threshold_db) / ratio - level_db
        }
    } else {
        let x = level_db - threshold_db;
        if x <= -knee * 0.5 {
            0.0
        } else if x >= knee * 0.5 {
            threshold_db + x / ratio - level_db
        } else {
            (1.0 / ratio - 1.0) * (x + knee * 0.5) * (x + knee * 0.5) / (2.0 * knee)
        }
    }
}

fn sanitize(p: EnhancedCompressorParams) -> EnhancedCompressorParams {
    EnhancedCompressorParams {
        input_gain_db: p.input_gain_db.clamp(-24.0, 24.0),
        threshold_db: p.threshold_db.clamp(-60.0, 0.0),
        ratio: p.ratio.clamp(1.0, 20.0),
        knee_db: p.knee_db.clamp(0.0, 24.0),
        attack_ms: p.attack_ms.clamp(0.1, 200.0),
        release_ms: p.release_ms.clamp(5.0, 2000.0),
        makeup_gain_db: p.makeup_gain_db.clamp(-24.0, 24.0),
        mix: p.mix.clamp(0.0, 1.0),
        detector_mode: p.detector_mode,
        stereo_link: p.stereo_link.clamp(0.0, 1.0),
        sc_hpf_hz: p.sc_hpf_hz.clamp(SC_HPF_OFF_HZ, 500.0),
        lookahead_ms: p.lookahead_ms.clamp(0.0, MAX_LOOKAHEAD_MS),
        auto_release: p.auto_release,
        auto_makeup: p.auto_makeup,
        warmth: p.warmth.clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(params: EnhancedCompressorParams) -> EnhancedCompressor {
        let mut c = EnhancedCompressor::new(params);
        c.prepare(48_000.0, 512);
        c
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * freq * i as f32 / 48_000.0).sin() * amp)
            .collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn ratio_one_is_unity() {
        assert_eq!(compressor_gain_db(-6.0, -18.0, 1.0, 0.0), 0.0);
    }

    #[test]
    fn threshold_above_reduces_gain() {
        let gain = compressor_gain_db(-6.0, -18.0, 4.0, 0.0);
        assert!(gain < -8.0 && gain > -10.0, "gain={gain}");
    }

    #[test]
    fn above_threshold_is_reduced_and_finite() {
        let mut c = comp(EnhancedCompressorParams {
            threshold_db: -24.0,
            ratio: 8.0,
            attack_ms: 0.1,
            warmth: 0.0,
            ..Default::default()
        });
        let mut l = vec![1.0_f32; 4_096];
        let mut r = vec![1.0_f32; 4_096];
        c.process_stereo(&mut l, &mut r);
        for s in l.iter().chain(r.iter()) {
            assert!(s.is_finite());
        }
        assert!(l[4_095] < 0.4, "last={}", l[4_095]);
        assert!(c.take_gr_meter() > 6.0);
    }

    #[test]
    fn mix_zero_is_dry() {
        let mut c = comp(EnhancedCompressorParams {
            threshold_db: -60.0,
            ratio: 20.0,
            mix: 0.0,
            ..Default::default()
        });
        let mut l = [1.0_f32; 128];
        let mut r = [1.0_f32; 128];
        c.process_stereo(&mut l, &mut r);
        assert!(l.iter().all(|s| (*s - 1.0).abs() < 1.0e-6));
    }

    #[test]
    fn lookahead_delays_audio_by_reported_latency() {
        let mut c = comp(EnhancedCompressorParams {
            ratio: 1.0,
            lookahead_ms: 5.0,
            mix: 1.0,
            warmth: 0.0,
            ..Default::default()
        });
        let latency = c.latency_samples() as usize;
        assert_eq!(latency, 240, "5 ms at 48 kHz");
        let n = 1_024;
        let mut l = vec![0.0_f32; n];
        l[100] = 1.0;
        let mut r = l.clone();
        c.process_stereo(&mut l, &mut r);
        let peak_at = l
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak_at, 100 + latency);
    }

    #[test]
    fn lookahead_catches_transient_harder_than_no_lookahead() {
        let squash = |lookahead_ms: f32| -> f32 {
            let mut c = comp(EnhancedCompressorParams {
                threshold_db: -30.0,
                ratio: 10.0,
                attack_ms: 5.0,
                lookahead_ms,
                warmth: 0.0,
                ..Default::default()
            });
            let latency = c.latency_samples() as usize;
            let n = 4_096;
            let mut l = vec![0.0_f32; n];
            for s in l.iter_mut().skip(1_000).take(480) {
                *s = 1.0;
            }
            let mut r = l.clone();
            c.process_stereo(&mut l, &mut r);
            peak(&l[1_000 + latency..1_000 + latency + 480])
        };
        let with = squash(MAX_LOOKAHEAD_MS);
        let without = squash(0.0);
        assert!(
            with < without * 0.7,
            "lookahead should pre-duck the burst: with={with} without={without}"
        );
    }

    #[test]
    fn sidechain_hpf_ignores_bass_pumping() {
        let gr_for = |sc_hpf_hz: f32| -> f32 {
            let mut c = comp(EnhancedCompressorParams {
                threshold_db: -30.0,
                ratio: 10.0,
                sc_hpf_hz,
                warmth: 0.0,
                ..Default::default()
            });
            let mut l = sine(50.0, 0.5, 24_000);
            let mut r = l.clone();
            c.process_stereo(&mut l, &mut r);
            c.take_gr_meter()
        };
        let open = gr_for(SC_HPF_OFF_HZ);
        let filtered = gr_for(300.0);
        assert!(
            filtered < open * 0.25,
            "50 Hz should barely register through a 300 Hz sidechain HPF: open={open} filtered={filtered}"
        );
    }

    #[test]
    fn auto_makeup_restores_level() {
        let out_for = |auto_makeup: bool| -> f32 {
            let mut c = comp(EnhancedCompressorParams {
                threshold_db: -24.0,
                ratio: 4.0,
                auto_makeup,
                warmth: 0.0,
                ..Default::default()
            });
            let mut l = sine(1_000.0, 0.5, 24_000);
            let mut r = l.clone();
            c.process_stereo(&mut l, &mut r);
            peak(&l[20_000..])
        };
        let plain = out_for(false);
        let compensated = out_for(true);
        assert!(
            compensated > plain * 1.5,
            "auto makeup should lift the compressed signal: plain={plain} compensated={compensated}"
        );
        assert!(compensated < 1.0, "but not into clipping");
    }

    #[test]
    fn auto_release_recovers_in_two_stages() {
        // After a loud burst, the auto-release GR should fall quickly at
        // first (fast leg) yet keep recovering long after (slow leg).
        // The post-burst signal is silence, so probe the GR meter.
        let mut c2 = comp(EnhancedCompressorParams {
            threshold_db: -30.0,
            ratio: 10.0,
            attack_ms: 1.0,
            release_ms: 200.0,
            auto_release: true,
            warmth: 0.0,
            ..Default::default()
        });
        let burst = sine(1_000.0, 0.8, 12_000);
        let mut bl = burst.clone();
        let mut br = burst;
        c2.process_stereo(&mut bl, &mut br);
        let gr_at_end = c2.take_gr_meter();
        // Run `silence` samples, then read the GR over the next 256.
        let mut probe_after = |silence: usize| -> f32 {
            let mut zl = vec![0.0_f32; silence];
            let mut zr = vec![0.0_f32; silence];
            c2.process_stereo(&mut zl, &mut zr);
            let _ = c2.take_gr_meter();
            let mut ml = [0.0_f32; 256];
            let mut mr = [0.0_f32; 256];
            c2.process_stereo(&mut ml, &mut mr);
            c2.take_gr_meter()
        };
        let gr_100ms = probe_after(4_800 - 256);
        let gr_1s = probe_after(43_200 - 256);
        assert!(gr_at_end > 6.0, "burst should compress: {gr_at_end}");
        assert!(
            gr_100ms < gr_at_end * 0.7,
            "fast leg should recover early: {gr_100ms} vs {gr_at_end}"
        );
        assert!(
            gr_100ms > 1.0,
            "slow leg should still be holding at 100 ms: {gr_100ms}"
        );
        assert!(
            gr_1s < gr_100ms * 0.5,
            "tail should keep recovering: {gr_1s} vs {gr_100ms}"
        );
    }

    #[test]
    fn warmth_is_transparent_at_low_level_and_bounded() {
        let mut c = comp(EnhancedCompressorParams {
            threshold_db: 0.0,
            ratio: 1.0,
            warmth: 1.0,
            ..Default::default()
        });
        let mut l = sine(1_000.0, 0.01, 4_800);
        let reference = l.clone();
        let mut r = l.clone();
        c.process_stereo(&mut l, &mut r);
        for (a, b) in l.iter().zip(reference.iter()) {
            assert!((a - b).abs() < 5.0e-4, "low level should stay linear");
        }
        let mut loud_l = vec![2.0_f32; 4_800];
        let mut loud_r = loud_l.clone();
        c.process_stereo(&mut loud_l, &mut loud_r);
        assert!(peak(&loud_l) <= 2.0, "saturator must not add gain");
    }
}
