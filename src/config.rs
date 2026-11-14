use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
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
    pub wave_colors: Option<Vec<String>>,
    pub theme: String,
    pub theme_spread: f32,
    pub theme_boost: f32,
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
            wave_colors: None,
            theme: "auto".into(),
            theme_spread: 40.0,
            theme_boost: 1.25,
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

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Theme {
    pub accent: Option<String>,
    pub wave_colors: Option<Vec<String>>,
    pub colors: Option<Vec<String>>,
}

pub fn theme_path(cfg: &Config) -> Option<PathBuf> {
    let t = cfg.theme.trim();
    match t {
        "" | "off" | "none" => None,
        "auto" => Some(config_home().join("soundwav").join("theme.toml")),
        _ => Some(match t.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None => PathBuf::from(t),
        }),
    }
}

pub fn read_theme(path: Option<&Path>) -> Result<Theme, String> {
    let Some(path) = path else { return Ok(Theme::default()) };
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(_) => Ok(Theme::default()),
    }
}

pub fn watch_paths(cfg: &Config) -> Vec<PathBuf> {
    theme_path(cfg).into_iter().chain([config_home().join("cava").join("config")]).collect()
}

pub fn stamp(paths: &[PathBuf]) -> Vec<Option<(SystemTime, u64)>> {
    paths.iter().map(|p| std::fs::metadata(p).ok().and_then(|m| Some((m.modified().ok()?, m.len())))).collect()
}

fn hex_list(v: Option<&Vec<String>>) -> Option<Vec<[f32; 3]>> {
    v.map(|v| v.iter().filter_map(|s| parse_hex(s)).collect()).filter(|v: &Vec<[f32; 3]>| v.len() >= 2)
}

pub fn palette(cfg: &Config, theme: &Theme) -> [[f32; 3]; 3] {
    let list: Vec<[f32; 3]> = hex_list(cfg.colors.as_ref())
        .or_else(|| hex_list(theme.colors.as_ref()))
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

pub fn wave_palette(cfg: &Config, theme: &Theme) -> [[f32; 3]; 3] {
    let d = [[0.706, 0.298, 1.0], [1.0, 0.373, 0.718], [0.373, 0.831, 1.0]];
    let list = cfg.wave_colors.as_ref().or(theme.wave_colors.as_ref());
    if list.is_none() {
        if let Some(a) = theme.accent.as_deref().and_then(parse_hex) {
            return trio(a, cfg.theme_spread, cfg.theme_boost);
        }
    }
    std::array::from_fn(|i| list.and_then(|l| l.get(i)).and_then(|s| parse_hex(s)).unwrap_or(d[i]))
}

fn to_hsl(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    if d < 1e-6 {
        return [0.0, 0.0, l];
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    [h * 60.0, s.min(1.0), l]
}

fn from_hsl(h: [f32; 3]) -> [f32; 3] {
    let (hue, s, l) = (h[0].rem_euclid(360.0) / 60.0, h[1].clamp(0.0, 1.0), h[2].clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

pub fn trio(accent: [f32; 3], spread: f32, boost: f32) -> [[f32; 3]; 3] {
    let [h, s, l] = to_hsl(accent);
    let s = (s * boost.max(0.0)).min(1.0);
    [
        from_hsl([h - spread, s, (l - 0.06).max(0.35)]),
        from_hsl([h, s, l]),
        from_hsl([h + spread, s, (l + 0.08).min(0.88)]),
    ]
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    fn theme(text: &str) -> Theme {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn theme_colours_replace_defaults() {
        let t = theme("wave_colors = [\"#ff0000\", \"#00ff00\", \"#0000ff\"]");
        let p = wave_palette(&Config::default(), &t);
        assert_eq!(p, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn explicit_colours_beat_theme() {
        let t = theme("wave_colors = [\"#ff0000\", \"#00ff00\", \"#0000ff\"]");
        let cfg = Config { wave_colors: Some(vec!["#ffffff".into()]), ..Config::default() };
        let p = wave_palette(&cfg, &t);
        assert_eq!(p[0], [1.0, 1.0, 1.0]);
        assert_eq!(p[1], [1.0, 0.373, 0.718]);
    }

    #[test]
    fn theme_bar_colours_beat_cava() {
        let t = theme("colors = [\"#000000\", \"#ffffff\"]");
        let p = palette(&Config::default(), &t);
        assert_eq!(p, [[0.0; 3], [0.5; 3], [1.0; 3]]);
    }

    #[test]
    fn accent_builds_a_trio_around_it() {
        let t = theme("accent = \"#cd95ff\"");
        let cfg = Config { theme_boost: 1.0, ..Config::default() };
        let p = wave_palette(&cfg, &t);
        let a = parse_hex("#cd95ff").unwrap();
        for i in 0..3 {
            assert!((p[1][i] - a[i]).abs() < 0.01);
        }
        let hb = to_hsl(p[0])[0];
        let ht = to_hsl(p[2])[0];
        let ha = to_hsl(a)[0];
        assert!(((ha - hb).rem_euclid(360.0) - 40.0).abs() < 1.5);
        assert!(((ht - ha).rem_euclid(360.0) - 40.0).abs() < 1.5);
    }

    #[test]
    fn hsl_round_trip() {
        for c in [[0.2, 0.5, 0.9], [1.0, 0.0, 0.0], [0.75, 0.76, 0.62], [0.5, 0.5, 0.5]] {
            let r = from_hsl(to_hsl(c));
            for i in 0..3 {
                assert!((r[i] - c[i]).abs() < 1e-4, "{c:?} -> {r:?}");
            }
        }
    }

    #[test]
    fn theme_path_modes() {
        let off = Config { theme: "off".into(), ..Config::default() };
        assert!(theme_path(&off).is_none());
        assert!(theme_path(&Config::default()).unwrap().ends_with("soundwav/theme.toml"));
        let custom = Config { theme: "/x/y.toml".into(), ..Config::default() };
        assert_eq!(theme_path(&custom).unwrap(), PathBuf::from("/x/y.toml"));
        assert!(read_theme(Some(Path::new("/nonexistent/soundwav-theme.toml"))).is_ok());
    }
}
