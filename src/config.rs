use crate::DAEMON;
use serde::{Deserialize, Deserializer};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use zoo_ipc::daemon::config_home;
use zoo_ipc::ThemeSource;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Bars,
    Waves,
}

impl Style {
    fn other(self) -> Self {
        match self {
            Style::Bars => Style::Waves,
            Style::Waves => Style::Bars,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Style::Bars => "bars",
            Style::Waves => "waves",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleCmd {
    Set(Style),
    Next,
}

impl StyleCmd {
    pub fn parse(arg: &str) -> Option<Self> {
        match arg.trim() {
            "bars" => Some(StyleCmd::Set(Style::Bars)),
            "waves" => Some(StyleCmd::Set(Style::Waves)),
            "next" | "toggle" => Some(StyleCmd::Next),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LayerKind {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppMatch {
    Any,
    List(Vec<String>),
}

impl AppMatch {
    pub fn matches(&self, app_id: Option<&str>) -> bool {
        match self {
            AppMatch::Any => true,
            AppMatch::List(list) => app_id.is_some_and(|a| list.iter().any(|p| p == a)),
        }
    }
}

impl<'de> Deserialize<'de> for AppMatch {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let list: Vec<String> = s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
        if list.iter().any(|p| p == "*") {
            Ok(AppMatch::Any)
        } else {
            Ok(AppMatch::List(list))
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub app_id: AppMatch,
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
    pub layer: LayerKind,
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
    pub style: Style,
    pub wave_colors: Option<Vec<String>>,
    pub theme: ThemeSource,
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
            app_id: AppMatch::List(vec!["kitty".into()]),
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
            layer: LayerKind::Top,
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
            style: Style::Waves,
            wave_colors: None,
            theme: ThemeSource::Auto,
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

static STYLE: Mutex<Option<Style>> = Mutex::new(None);

pub fn style(cfg: &Config) -> Style {
    STYLE.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(cfg.style)
}

pub fn set_style(cfg: &Config, cmd: StyleCmd) -> Style {
    let next = match cmd {
        StyleCmd::Set(s) => s,
        StyleCmd::Next => style(cfg).other(),
    };
    *STYLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(next);
    next
}

pub fn load(inline: Option<&str>) -> Config {
    let mut cfg: Config = DAEMON.load_config(inline);
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
    DAEMON.theme_file(&cfg.theme)
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
        let off = Config { theme: ThemeSource::Off, ..Config::default() };
        assert!(theme_path(&off).is_none());
        assert!(theme_path(&Config::default()).unwrap().ends_with("soundwav/theme.toml"));
        let custom = Config { theme: ThemeSource::File("/x/y.toml".into()), ..Config::default() };
        assert_eq!(theme_path(&custom).unwrap(), PathBuf::from("/x/y.toml"));
        assert!(read_theme(Some(Path::new("/nonexistent/soundwav-theme.toml"))).is_ok());
    }
}

#[cfg(test)]
mod enum_tests {
    use super::*;

    fn parse(text: &str) -> Config {
        toml::Value::Table(text.parse().unwrap()).try_into().unwrap()
    }

    #[test]
    fn settings_parse_into_enums() {
        let c = parse("style = \"bars\"\nlayer = \"overlay\"\napp_id = \"kitty, foot\"\ntheme = \"off\"");
        assert_eq!(c.style, Style::Bars);
        assert_eq!(c.layer, LayerKind::Overlay);
        assert_eq!(c.app_id, AppMatch::List(vec!["kitty".into(), "foot".into()]));
        assert_eq!(c.theme, ThemeSource::Off);
        assert_eq!(parse("app_id = \"*\"").app_id, AppMatch::Any);
    }

    #[test]
    fn bad_style_is_rejected() {
        let t: Result<Config, _> = toml::Value::Table("style = \"wavez\"".parse().unwrap()).try_into();
        assert!(t.is_err());
    }

    #[test]
    fn app_patterns() {
        assert!(AppMatch::Any.matches(Some("firefox")));
        assert!(AppMatch::Any.matches(None));
        let l = parse("app_id = \"kitty, foot\"").app_id;
        assert!(l.matches(Some("foot")));
        assert!(!l.matches(Some("alacritty")));
        assert!(!l.matches(None));
    }

    #[test]
    fn style_commands() {
        assert_eq!(StyleCmd::parse("next"), Some(StyleCmd::Next));
        assert_eq!(StyleCmd::parse("toggle"), Some(StyleCmd::Next));
        assert_eq!(StyleCmd::parse("bars"), Some(StyleCmd::Set(Style::Bars)));
        assert_eq!(StyleCmd::parse("sparkles"), None);
    }
}
