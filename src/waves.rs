use crate::cava::{bands, Audio};
use crate::config::Config;
use crate::gpu::{Batch, WVert};
use crate::layout::{self, Rect, WaveIn, WaveSample};

const BAND_GAIN: [f32; 3] = [1.0, 1.25, 1.8];
const BAND_SPEED: [f32; 3] = [0.6, 1.4, 3.2];
const ATTACK_S: f32 = 0.025;
const DECAY_S: f32 = 0.22;
const SLOW_BASS_S: f32 = 0.6;
const KICK_DECAY_S: f32 = 0.15;

#[derive(Default)]
struct Env {
    fast: [[f32; 3]; 2],
    slow_bass: [f32; 2],
    kick: [f32; 2],
    phase: [f32; 3],
}

#[derive(Default)]
pub struct Waves {
    env: Env,
    samples: Vec<WaveSample>,
}

impl Waves {
    pub fn update(&mut self, audio: &Audio, wave_speed: f32, dt: f32) {
        let (bl, br) = (bands(&audio.left), bands(&audio.right));
        let up = 1.0 - (-dt / ATTACK_S).exp();
        let down = 1.0 - (-dt / DECAY_S).exp();
        let slow = 1.0 - (-dt / SLOW_BASS_S).exp();
        let kdecay = (-dt / KICK_DECAY_S).exp();
        let env = &mut self.env;
        for (c, src) in [bl, br].into_iter().enumerate() {
            for b in 0..3 {
                let target = (src[b] * BAND_GAIN[b]).clamp(0.0, 1.0);
                let cur = &mut env.fast[c][b];
                *cur += (target - *cur) * if target > *cur { up } else { down };
            }
            let f = env.fast[c][0];
            env.slow_bass[c] += (f - env.slow_bass[c]) * slow;
            env.kick[c] = (env.kick[c] * kdecay).max((f - env.slow_bass[c]) * 1.6).clamp(0.0, 1.0);
        }
        let wrap = std::f32::consts::TAU * 10.0;
        for (b, (phase, speed)) in env.phase.iter_mut().zip(BAND_SPEED).enumerate() {
            let energy = 0.5 * (env.fast[0][b] + env.fast[1][b]);
            *phase = (*phase + speed * wave_speed * (0.35 + energy) * dt).rem_euclid(wrap);
        }
    }

    pub fn build(&mut self, rect: Rect, caps: [f32; 4], cfg: &Config, batch: &mut Batch) {
        let e = &self.env;
        let bass = |c: usize| (e.fast[c][0] + e.kick[c] * 0.6).clamp(0.0, 1.0);
        let w = WaveIn {
            env_l: [bass(0), e.fast[0][1], e.fast[0][2]],
            env_r: [bass(1), e.fast[1][1], e.fast[1][2]],
            phase: e.phase,
            floor: 2.0,
        };
        layout::waves(rect, cfg.corner_radius, caps, cfg.border, cfg.corner_blend, cfg.wave_step, &w, &mut self.samples);
        let n = self.samples.len();
        if n < 8 {
            return;
        }
        let hw = cfg.wave_line * 0.5 + cfg.glow * 2.5;
        let verts = &mut batch.verts;
        for b in 0..3 {
            let crest = |i: usize| {
                let s = &self.samples[i % n];
                [s.base[0] + s.normal[0] * s.off[b], s.base[1] + s.normal[1] * s.off[b]]
            };
            let start = verts.len() as u32;
            for i in 0..=n {
                let s = &self.samples[i % n];
                let c = crest(i);
                verts.push(WVert { pos: s.base, v: 0.0, kind: 0.0, band: b as f32, over: s.over });
                verts.push(WVert { pos: c, v: 1.0, kind: 0.0, band: b as f32, over: s.over });
            }
            batch.strips.push((start, verts.len() as u32 - start));
            let start = verts.len() as u32;
            for i in 0..=n {
                let s = &self.samples[i % n];
                let c = crest(i);
                let (a, z) = (crest(i + n - 2), crest(i + 2));
                let t = [z[0] - a[0], z[1] - a[1]];
                let l = (t[0] * t[0] + t[1] * t[1]).sqrt();
                let mut nr = if l > 1e-4 { [t[1] / l, -t[0] / l] } else { s.normal };
                if nr[0] * s.normal[0] + nr[1] * s.normal[1] < 0.0 {
                    nr = [-nr[0], -nr[1]];
                }
                let (ca, zc) = ([c[0] - a[0], c[1] - a[1]], [z[0] - c[0], z[1] - c[1]]);
                let cross = (ca[0] * zc[1] - ca[1] * zc[0]).abs();
                let len = |v: [f32; 2]| (v[0] * v[0] + v[1] * v[1]).sqrt();
                let radius = if cross > 1e-5 { len(ca) * len(zc) * l / (2.0 * cross) } else { f32::MAX };
                let bend = ((a[0] + z[0]) * 0.5 - c[0]) * nr[0] + ((a[1] + z[1]) * 0.5 - c[1]) * nr[1];
                let limit = (radius * 0.8).max(cfg.wave_line * 0.5);
                let mut outer = hw;
                let mut inner = hw.min(s.off[b] + 1.0);
                if bend > 0.0 {
                    outer = outer.min(limit);
                } else {
                    inner = inner.min(limit);
                }
                verts.push(WVert { pos: [c[0] + nr[0] * outer, c[1] + nr[1] * outer], v: outer, kind: 1.0, band: b as f32, over: s.over });
                verts.push(WVert { pos: [c[0] - nr[0] * inner, c[1] - nr[1] * inner], v: -inner, kind: 1.0, band: b as f32, over: s.over });
            }
            batch.strips.push((start, verts.len() as u32 - start));
        }
    }
}
