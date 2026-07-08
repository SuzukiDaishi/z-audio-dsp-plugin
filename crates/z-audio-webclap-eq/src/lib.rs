//! Z Audio EQ — a Pro-Q-style 8-band parametric EQ, packaged as a real
//! WCLAP audio-effect plugin.
//!
//! Bands: bell / low shelf / high shelf / low cut / high cut / notch, cut
//! slopes 6-48 dB/oct, per-band Stereo/Mid/Side/Left/Right placement, and
//! band-solo listen ("hear just this band's region"). The engine also runs
//! a pre + post FFT tap and pushes spectrum frames to the UI:
//!
//!   plugin → UI  "ZEQS" u8 kind(0=pre,1=post) u8 0 u16 bins f32 rate
//!                bins×f32 dB
//!
//! Parameter edits ride the standard `{set:[id,value]}` path (ids in
//! `params.rs`, block 700-773). The original 3-band EQ surface (submodule
//! ids 40-57) is retired here; the native VST3/CLAP EQ keeps it with its
//! own UI snapshot under `crates/z-audio-eq-plugin/ui`.

use std::sync::OnceLock;

use wclap_plugin::{
    init_plugin, send_to_ui, silence, ParamDef, Plugin, PluginDef, ProcessCtx, ProcessStatus,
};

pub mod engine;
pub mod params;

use engine::{apply_param, param_value, EqEngine, SpectrumTap, FFT_BINS};
use params::param_defs;

static PLUGIN_DEF: PluginDef = PluginDef {
    id: b"dev.zaudio.simple-eq\0",
    name: b"Z Audio EQ\0",
    vendor: b"zukky\0",
    url: b"https://github.com/SuzukiDaishi/z-audio-dsp\0",
    version: b"0.2.0\0",
    description: b"Pro-Q-style 8-band parametric EQ with band solo and spectrum analyzer\0",
    features: &[b"audio-effect\0", b"equalizer\0", b"eq\0", b"stereo\0"],
    audio_inputs: 1,
    audio_outputs: 1,
    note_inputs: 0,
    ui_path: Some(b"/ui/index.html\0"),
};

static PARAMS: OnceLock<Vec<ParamDef>> = OnceLock::new();

const SPECTRUM_PRE: u8 = 0;
const SPECTRUM_POST: u8 = 1;

struct ZAudioWebEq {
    engine: EqEngine,
    tap_pre: SpectrumTap,
    tap_post: SpectrumTap,
    packet: Vec<u8>,
    ui_seen: bool,
}

impl ZAudioWebEq {
    fn push_spectrum(&mut self, kind: u8) {
        let tap = if kind == SPECTRUM_PRE {
            &mut self.tap_pre
        } else {
            &mut self.tap_post
        };
        if !tap.frame_ready {
            return;
        }
        tap.frame_ready = false;

        self.packet.clear();
        self.packet.extend_from_slice(b"ZEQS");
        self.packet.push(kind);
        self.packet.push(0);
        self.packet
            .extend_from_slice(&(FFT_BINS as u16).to_le_bytes());
        self.packet
            .extend_from_slice(&self.engine.sample_rate().to_le_bytes());
        for db in &tap.frame_db {
            self.packet.extend_from_slice(&db.to_le_bytes());
        }
        send_to_ui(&self.packet);
    }
}

impl Plugin for ZAudioWebEq {
    fn new() -> Self {
        Self {
            engine: EqEngine::new(48_000.0),
            tap_pre: SpectrumTap::new(),
            tap_post: SpectrumTap::new(),
            packet: Vec::with_capacity(16 + FFT_BINS * 4),
            ui_seen: false,
        }
    }

    fn activate(&mut self, sample_rate: f64, _max_frames: u32) {
        let params = *self.engine.params();
        self.engine = EqEngine::new(sample_rate as f32);
        self.engine.set_params(params);
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn params() -> &'static [ParamDef] {
        PARAMS.get_or_init(param_defs)
    }

    fn get_param(&self, id: u32) -> f64 {
        param_value(self.engine.params(), id)
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let mut p = *self.engine.params();
        apply_param(&mut p, id, value);
        self.engine.set_params(p);
    }

    fn on_ui_message(&mut self, bytes: &[u8]) -> bool {
        if bytes == b"\x65ready" {
            self.ui_seen = true;
            return true;
        }
        false
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus {
        match ctx.stereo_io() {
            Some(io) => {
                if self.ui_seen {
                    self.tap_pre.push(io.input_l, io.input_r);
                }
                self.engine
                    .process(io.input_l, io.input_r, io.output_l, io.output_r);
                if self.ui_seen {
                    self.tap_post.push(io.output_l, io.output_r);
                    self.push_spectrum(SPECTRUM_PRE);
                    self.push_spectrum(SPECTRUM_POST);
                }
            }
            None => silence(ctx),
        }
        ProcessStatus::Continue
    }
}

#[no_mangle]
pub extern "C" fn _initialize() {
    init_plugin::<ZAudioWebEq>(&PLUGIN_DEF);
}

#[cfg(test)]
mod tests {
    use super::engine::{apply_param, param_value};
    use super::params::param_defs;

    #[test]
    fn set_get_round_trips_across_the_surface() {
        let mut p = crate::engine::EqParams::default();
        for def in param_defs() {
            apply_param(&mut p, def.id, def.max);
            assert!(
                (param_value(&p, def.id) - def.max).abs() < 1e-6,
                "id {} did not round-trip",
                def.id
            );
        }
    }
}
