use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub app_id: String,
    pub fps: u32,
    pub bars_per_channel: u32,
    pub bar_width: f32,
    pub bar_gap: f32,
    pub corner_radius: f32,
    pub border: f32,
    pub min_open: f32,
    pub max_out: f32,
    pub max_over: f32,
    pub max_over_frac: f32,
    pub over_alpha: f32,
    pub layer: String,
    pub glow: f32,
    pub opacity: f32,
    pub follow_ms: f32,
    pub fade_ms: f32,
    pub cava_input: String,
    pub cava_source: String,
    pub colors: Option<Vec<String>>,
    pub reserve: [f32; 4],
    pub tab_strip_width: f32,
    pub tab_strip_offset: f32,
    pub tab_min: usize,
    pub style: String,
    pub wave_colors: Vec<String>,
    pub wave_line: f32,
    pub wave_fill: f32,
    pub wave_step: f32,
    pub wave_speed: f32,
    pub corner_blend: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_id: "kitty".into(),
            fps: 60,
            bars_per_channel: 96,
            bar_width: 5.0,
            bar_gap: 3.0,
            corner_radius: 14.0,
            border: 5.0,
            min_open: 60.0,
            max_out: 220.0,
            max_over: 56.0,
            max_over_frac: 0.12,
            over_alpha: 0.8,
            layer: "top".into(),
            glow: 6.0,
            opacity: 0.95,
            follow_ms: 90.0,
            fade_ms: 250.0,
            cava_input: "pulse".into(),
            cava_source: "auto".into(),
            colors: None,
            reserve: [0.0; 4],
            tab_strip_width: 30.0,
            tab_strip_offset: 12.0,
            tab_min: 2,
            style: "waves".into(),
            wave_colors: vec!["#B44CFF".into(), "#FF5FB7".into(), "#5FD4FF".into()],
            wave_line: 2.5,
            wave_fill: 0.28,
            wave_step: 3.0,
            wave_speed: 1.0,
            corner_blend: 120.0,
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("soundwav")
}

pub static STYLE: AtomicU8 = AtomicU8::new(0);

pub fn wavy(cfg: &Config) -> bool {
    match STYLE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => cfg.style.trim().eq_ignore_ascii_case("waves"),
    }
}

pub fn set_style(cfg: &Config, arg: &str) -> Option<String> {
    let v = match arg.trim() {
        "bars" => 1,
        "waves" => 2,
        "next" | "toggle" => if wavy(cfg) { 1 } else { 2 },
        _ => return None,
    };
    STYLE.store(v, Ordering::Relaxed);
    Some(if v == 2 { "waves" } else { "bars" }.into())
}

fn merged(inline: Option<&str>) -> toml::Table {
    let path = config_home().join("soundwav").join("config.toml");
    let mut table = match std::fs::read_to_string(&path) {
        Ok(text) => text.parse::<toml::Table>().unwrap_or_else(|e| {
            eprintln!("soundwav: {}: {e}, ignoring it", path.display());
            toml::Table::new()
        }),
        Err(_) => toml::Table::new(),
    };
    if let Some(s) = inline {
        match s.parse::<toml::Table>() {
            Ok(t) => table.extend(t),
            Err(e) => eprintln!("soundwav: --toml: {e}, ignoring it"),
        }
    }
    table
}

pub fn load(inline: Option<&str>) -> Config {
    let mut cfg = toml::Value::Table(merged(inline)).try_into::<Config>().unwrap_or_else(|e| {
        eprintln!("soundwav: config: {e}, using defaults");
        Config::default()
    });
    cfg.fps = cfg.fps.clamp(10, 240);
    cfg.bars_per_channel = cfg.bars_per_channel.clamp(8, 256);
    cfg.bar_width = cfg.bar_width.max(1.0);
    cfg.bar_gap = cfg.bar_gap.max(0.0);
    cfg.wave_step = cfg.wave_step.clamp(1.0, 16.0);
    cfg.wave_line = cfg.wave_line.clamp(0.5, 12.0);
    cfg.wave_fill = cfg.wave_fill.clamp(0.0, 1.0);
    cfg.corner_blend = cfg.corner_blend.max(0.0);
    cfg
}

pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let t = s.trim().trim_matches(|c| c == '\'' || c == '"').trim_start_matches('#');
    if t.len() < 6 {
        return None;
    }
    let v = u32::from_str_radix(&t[..6], 16).ok()?;
    Some([((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0])
}

fn cava_gradient() -> Option<Vec<[f32; 3]>> {
    let text = std::fs::read_to_string(config_home().join("cava").join("config")).ok()?;
    let mut in_color = false;
    let mut found: Vec<(u32, [f32; 3])> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_color = l == "[color]";
            continue;
        }
        if !in_color || l.starts_with(';') || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let k = k.trim();
        if let Some(n) = k.strip_prefix("gradient_color_") {
            if let (Ok(i), Some(c)) = (n.parse::<u32>(), parse_hex(v)) {
                found.push((i, c));
            }
        }
    }
    found.sort_by_key(|f| f.0);
    (found.len() >= 2).then(|| found.into_iter().map(|f| f.1).collect())
}

pub fn palette(cfg: &Config) -> [[f32; 3]; 3] {
    let list: Vec<[f32; 3]> = cfg
        .colors
        .as_ref()
        .map(|v| v.iter().filter_map(|s| parse_hex(s)).collect())
        .filter(|v: &Vec<[f32; 3]>| v.len() >= 2)
        .or_else(cava_gradient)
        .unwrap_or_else(|| vec![[0.298, 0.153, 0.541], [0.804, 0.584, 1.0], [1.0, 0.996, 1.0]]);
    let n = list.len();
    let pick = |t: f32| {
        let f = t * (n - 1) as f32;
        let i = (f.floor() as usize).min(n - 2);
        let k = f - i as f32;
        let (a, b) = (list[i], list[i + 1]);
        [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k]
    };
    [pick(0.0), pick(0.5), pick(1.0)]
}

pub fn wave_palette(cfg: &Config) -> [[f32; 3]; 3] {
    let d = [[0.706, 0.298, 1.0], [1.0, 0.373, 0.718], [0.373, 0.831, 1.0]];
    std::array::from_fn(|i| cfg.wave_colors.get(i).and_then(|s| parse_hex(s)).unwrap_or(d[i]))
}
