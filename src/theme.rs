use crate::config::{palette, read_theme, stamp, theme_path, wave_palette, watch_paths, Config};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

const POLL: Duration = Duration::from_secs(1);
const EASE_S: f32 = 0.6;

pub struct Colours {
    pub bars: [[f32; 3]; 3],
    pub waves: [[f32; 3]; 3],
    target_bars: [[f32; 3]; 3],
    target_waves: [[f32; 3]; 3],
    watch: Vec<PathBuf>,
    stamp: Vec<Option<(SystemTime, u64)>>,
    watch_at: Instant,
}

impl Colours {
    pub fn new(cfg: &Config) -> Self {
        let theme = read_theme(theme_path(cfg).as_deref()).unwrap_or_else(|e| {
            eprintln!("soundwav: theme {e}, ignoring it");
            Default::default()
        });
        let bars = palette(cfg, &theme);
        let waves = wave_palette(cfg, &theme);
        let watch = watch_paths(cfg);
        let stamp = stamp(&watch);
        Self { bars, waves, target_bars: bars, target_waves: waves, watch, stamp, watch_at: Instant::now() }
    }

    pub fn update(&mut self, cfg: &Config, now: Instant, dt: f32) {
        if now.duration_since(self.watch_at) >= POLL {
            self.watch_at = now;
            let st = stamp(&self.watch);
            if st != self.stamp {
                self.stamp = st;
                match read_theme(theme_path(cfg).as_deref()) {
                    Ok(theme) => {
                        let (c, w) = (palette(cfg, &theme), wave_palette(cfg, &theme));
                        if c != self.target_bars || w != self.target_waves {
                            self.target_bars = c;
                            self.target_waves = w;
                            eprintln!("soundwav: theme colours updated");
                        }
                    }
                    Err(e) => eprintln!("soundwav: theme {e}, keeping colours"),
                }
            }
        }
        let k = 1.0 - (-dt / EASE_S).exp();
        for (cur, tgt) in self.bars.iter_mut().zip(self.target_bars).chain(self.waves.iter_mut().zip(self.target_waves)) {
            for i in 0..3 {
                cur[i] += (tgt[i] - cur[i]) * k;
            }
        }
    }
}
