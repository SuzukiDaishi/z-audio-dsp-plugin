# 11. 実装テンプレート

## Effect template

```rust
use crate::{Effect, ProcessContext};

pub struct MyEffect {
    sample_rate: f32,
    max_block_size: usize,
    params: MyEffectParams,
}

impl Default for MyEffect {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            max_block_size: 0,
            params: MyEffectParams::default(),
        }
    }
}

impl Effect for MyEffect {
    fn prepare(&mut self, sample_rate: f32, max_block_size: usize) {
        self.sample_rate = sample_rate;
        self.max_block_size = max_block_size;
        // allocate / resize here only
    }

    fn reset(&mut self) {
        // clear states, no allocation
    }

    fn process_stereo(&mut self, ctx: &ProcessContext, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let in_l = *l;
            let in_r = *r;
            let out_l = in_l;
            let out_r = in_r;
            *l = crate::math::flush_denormal(out_l);
            *r = crate::math::flush_denormal(out_r);
        }
    }
}
```

## Generator template

```rust
use crate::{Generator, ProcessContext};

pub struct MyGenerator {
    sample_rate: f32,
    phase: f32,
    frequency_hz: f32,
}

impl Generator for MyGenerator {
    fn prepare(&mut self, sample_rate: f32, _max_block_size: usize) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn process_mono(&mut self, _ctx: &ProcessContext, out: &mut [f32]) {
        let phase_inc = self.frequency_hz / self.sample_rate;
        for y in out {
            *y = (std::f32::consts::TAU * self.phase).sin();
            self.phase += phase_inc;
            self.phase -= self.phase.floor();
        }
    }
}
```

## DelayLine template

```rust
pub struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl DelayLine {
    pub fn new() -> Self {
        Self { buffer: Vec::new(), write_pos: 0 }
    }

    pub fn prepare(&mut self, max_delay_samples: usize) {
        self.buffer.resize(max_delay_samples.max(1), 0.0);
        self.write_pos = 0;
    }

    pub fn clear(&mut self) {
        for x in &mut self.buffer {
            *x = 0.0;
        }
        self.write_pos = 0;
    }

    pub fn push(&mut self, x: f32) {
        self.buffer[self.write_pos] = x;
        self.write_pos += 1;
        if self.write_pos >= self.buffer.len() {
            self.write_pos = 0;
        }
    }

    pub fn read_int(&self, delay_samples: usize) -> f32 {
        let len = self.buffer.len();
        let d = delay_samples.min(len - 1);
        let idx = (self.write_pos + len - d) % len;
        self.buffer[idx]
    }
}
```

## Compressor gain computer

```rust
pub fn compressor_gain_db(level_db: f32, threshold_db: f32, ratio: f32) -> f32 {
    if level_db <= threshold_db || ratio <= 1.0 {
        0.0
    } else {
        let compressed_db = threshold_db + (level_db - threshold_db) / ratio;
        compressed_db - level_db
    }
}
```

## Limiter gain computer

```rust
pub fn limiter_gain_db(level_db: f32, ceiling_db: f32) -> f32 {
    let excess = level_db - ceiling_db;
    if excess > 0.0 { -excess } else { 0.0 }
}
```

## FDN Hadamard 8

```rust
pub fn hadamard8(x: &mut [f32; 8]) {
    let mut h = 1;
    while h < 8 {
        let step = h * 2;
        let mut i = 0;
        while i < 8 {
            for j in 0..h {
                let a = x[i + j];
                let b = x[i + j + h];
                x[i + j] = a + b;
                x[i + j + h] = a - b;
            }
            i += step;
        }
        h *= 2;
    }

    let scale = 1.0 / (8.0_f32).sqrt();
    for v in x.iter_mut() {
        *v *= scale;
    }
}
```

## Plugin process skeleton for Effect

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
    self.sync_params_to_dsp();

    let num_channels = buffer.channels();
    if num_channels == 1 {
        let left = buffer.as_slice();
        self.scratch_r[..left.len()].copy_from_slice(left);
        self.effect.process_stereo(&self.ctx, left, &mut self.scratch_r[..left.len()]);
    } else {
        let (left, right) = split_stereo(buffer);
        self.effect.process_stereo(&self.ctx, left, right);
    }

    ProcessStatus::Normal
}
```

実際の nih-plug API に合わせて調整すること。
