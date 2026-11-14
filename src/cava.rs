use crate::config::Config;
use crate::DAEMON;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOUD: f32 = 0.01;

#[derive(Default, Clone)]
pub struct Audio {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub loud: Option<Instant>,
}

#[derive(Default)]
pub struct Control {
    pid: AtomicI32,
    paused: AtomicBool,
}

impl Control {
    pub fn set_paused(&self, p: bool) {
        if self.paused.swap(p, Ordering::SeqCst) != p {
            let pid = self.pid.load(Ordering::SeqCst);
            if pid > 0 {
                unsafe {
                    libc::kill(pid, if p { libc::SIGSTOP } else { libc::SIGCONT });
                }
            }
        }
    }
}

fn store(audio: &Mutex<Audio>, left: Vec<f32>, right: Vec<f32>) {
    let loud = left.iter().chain(right.iter()).any(|v| *v > LOUD);
    let mut a = audio.lock().unwrap();
    a.left = left;
    a.right = right;
    if loud {
        a.loud = Some(Instant::now());
    }
}

pub fn bands(spec: &[f32]) -> [f32; 3] {
    let n = spec.len();
    if n == 0 {
        return [0.0; 3];
    }
    let cut = |f: f32| ((n as f32 * f).round() as usize).clamp(1, n);
    let (a, b) = (cut(0.12), cut(0.5).max(cut(0.12) + 1).min(n));
    let band = |lo: usize, hi: usize| -> f32 {
        if hi <= lo {
            return 0.0;
        }
        let mut v: Vec<f32> = spec[lo..hi].to_vec();
        v.sort_by(|x, y| y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal));
        let k = (v.len() / 3).max(1);
        v[..k].iter().sum::<f32>() / k as f32
    };
    [band(0, a), band(a, b), band(b, n)]
}

pub fn split_stereo(raw: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let half = raw.len() / 2;
    let left = raw[..half].iter().rev().copied().collect();
    let right = raw[half..half * 2].to_vec();
    (left, right)
}

fn write_config(cfg: &Config) -> std::io::Result<PathBuf> {
    let dir = DAEMON.runtime_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("cava.conf");
    let text = format!(
        "[general]\nframerate = {fps}\nbars = {bars}\nautosens = 1\nsleep_timer = 0\nlower_cutoff_freq = 50\nhigher_cutoff_freq = 12000\n\n\
         [input]\nmethod = {input}\nsource = {source}\n\n\
         [output]\nmethod = raw\nraw_target = /dev/stdout\ndata_format = binary\nbit_format = 16bit\nchannels = stereo\nreverse = 0\n\n\
         [smoothing]\nnoise_reduction = 70\nmonstercat = 0\nwaves = 0\n",
        fps = cfg.fps.min(144),
        bars = cfg.bars_per_channel * 2,
        input = cfg.cava_input,
        source = cfg.cava_source,
    );
    std::fs::write(&path, text)?;
    Ok(path)
}

pub fn spawn(cfg: Config, audio: Arc<Mutex<Audio>>, ctl: Arc<Control>) {
    std::thread::Builder::new()
        .name("cava".into())
        .spawn(move || loop {
            let path = match write_config(&cfg) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("soundwav: cava config: {e}");
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            };
            let mut cmd = Command::new("cava");
            cmd.arg("-p").arg(&path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("soundwav: cannot start cava: {e}");
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            };
            let pid = child.id() as i32;
            ctl.pid.store(pid, Ordering::SeqCst);
            if ctl.paused.load(Ordering::SeqCst) {
                unsafe {
                    libc::kill(pid, libc::SIGSTOP);
                }
            }
            let mut out = child.stdout.take().unwrap();
            let n = (cfg.bars_per_channel * 2) as usize;
            let mut buf = vec![0u8; n * 2];
            let mut raw = vec![0f32; n];
            while out.read_exact(&mut buf).is_ok() {
                for (i, v) in raw.iter_mut().enumerate() {
                    *v = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f32 / 65535.0;
                }
                let (l, r) = split_stereo(&raw);
                store(&audio, l, r);
            }
            ctl.pid.store(0, Ordering::SeqCst);
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("soundwav: cava exited, restarting");
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("cava thread");
}

pub fn spawn_demo(cfg: Config, audio: Arc<Mutex<Audio>>) {
    std::thread::Builder::new()
        .name("demo".into())
        .spawn(move || {
            let start = Instant::now();
            let n = cfg.bars_per_channel as usize;
            let frame = Duration::from_secs_f64(1.0 / cfg.fps as f64);
            loop {
                let t = start.elapsed().as_secs_f32();
                let make = |phase: f32| -> Vec<f32> {
                    (0..n)
                        .map(|i| {
                            let u = i as f32 / (n - 1).max(1) as f32;
                            let beat = (0.5 + 0.5 * (t * 6.0 + phase).sin()).powf(3.0);
                            let bass = (1.0 - u * 4.0).max(0.0) * (0.55 + 0.45 * beat);
                            let mid = (0.35 + 0.3 * (t * 2.3 + u * 11.0 + phase).sin()) * (1.0 - u * 0.7);
                            let hat = (0.2 + 0.2 * (t * 9.1 + u * 37.0 + phase * 2.0).sin()) * u;
                            (bass + mid.max(0.0) + hat.max(0.0)).clamp(0.0, 1.0)
                        })
                        .collect()
                };
                store(&audio, make(0.0), make(1.3));
                std::thread::sleep(frame);
            }
        })
        .expect("demo thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bass_only_spectrum() {
        let spec: Vec<f32> = (0..96).map(|i| if i < 10 { 0.9 } else { 0.02 }).collect();
        let b = bands(&spec);
        assert!(b[0] > 0.8 && b[2] < 0.05, "{b:?}");
    }
}
