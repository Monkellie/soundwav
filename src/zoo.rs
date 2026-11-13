use crate::cava::Control;
use crate::config::Config;
use crate::layout::{free_space, side_caps, tab_strip_cap, Params, Rect};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub output: String,
    pub rect: Rect,
    pub caps: [f32; 4],
    pub visible: bool,
    pub game: bool,
    pub fullscreen_outputs: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
struct TabGroup {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default = "yes")]
    front: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
struct Client {
    id: String,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    special: bool,
    #[serde(default)]
    output: Option<String>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(default)]
    floating: bool,
    #[serde(default)]
    fullscreen: bool,
    #[serde(default)]
    maximized: bool,
    #[serde(default)]
    minimized: bool,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    tab_group: Option<TabGroup>,
}

#[derive(Deserialize, Clone, Debug)]
struct Monitor {
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    focused: bool,
    x: i32,
    y: i32,
    logical_width: i32,
    logical_height: i32,
}

#[derive(Deserialize, Clone, Debug)]
struct Monitors {
    outputs: Vec<Monitor>,
}

#[derive(Deserialize, Clone, Debug, Default)]
struct Anchor {
    top: bool,
    bottom: bool,
    left: bool,
    right: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
struct Margin {
    top: i32,
    right: i32,
    bottom: i32,
    left: i32,
}

#[derive(Deserialize, Clone, Debug)]
struct Layer {
    #[serde(default)]
    namespace: String,
    output: String,
    #[serde(default)]
    anchor: Anchor,
    #[serde(default)]
    exclusive_zone: i32,
    #[serde(default)]
    margin: Margin,
    #[serde(default = "yes")]
    mapped: bool,
}

fn socket_path() -> Option<PathBuf> {
    let rt = std::env::var_os("XDG_RUNTIME_DIR")?;
    let wd = std::env::var("WAYLAND_DISPLAY").ok()?;
    let last = wd.rsplit('/').next().unwrap_or(&wd).to_string();
    Some(PathBuf::from(rt).join(format!("zoolander-{last}.sock")))
}

fn parse_clients(raw: &str) -> Result<Vec<Client>, String> {
    let all: Vec<serde_json::Value> = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    Ok(all.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

fn request(path: &PathBuf, cmd: &str) -> Result<String, String> {
    let mut s = UnixStream::connect(path).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
    s.write_all(cmd.as_bytes()).map_err(|e| e.to_string())?;
    s.write_all(b"\n").map_err(|e| e.to_string())?;
    s.shutdown(std::net::Shutdown::Write).ok();
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let mut lines = out.splitn(3, '\n');
    let _greeting = lines.next();
    let status = lines.next().unwrap_or("");
    let rest = lines.next().unwrap_or("");
    if status == "OK json" {
        Ok(rest.trim().to_string())
    } else if let Some(v) = status.strip_prefix("OK ") {
        Ok(if rest.is_empty() { v.to_string() } else { format!("{v}\n{rest}") })
    } else {
        Err(status.to_string())
    }
}

const WS_QUERY: &str = "local s = \"\" for i = 0, zl.monitor_count() - 1 do local m = zl.monitor_info(i) if m and m.active_workspace then s = s .. m.name .. \"=\" .. tostring(m.active_workspace.name) .. \";\" end end return s";

fn parse_ws(s: &str) -> HashMap<String, String> {
    s.split(';')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn reserved_area(m: &Monitor, layers: &[Layer]) -> Rect {
    let (mut t, mut r, mut b, mut l) = (0i32, 0i32, 0i32, 0i32);
    for ly in layers.iter().filter(|ly| ly.output == m.name && ly.mapped && ly.exclusive_zone > 0 && !ly.namespace.starts_with("soundwav")) {
        let a = &ly.anchor;
        let z = ly.exclusive_zone;
        if a.top && !a.bottom && a.left == a.right {
            t = t.max(z + ly.margin.top);
        } else if a.bottom && !a.top && a.left == a.right {
            b = b.max(z + ly.margin.bottom);
        } else if a.left && !a.right && a.top == a.bottom {
            l = l.max(z + ly.margin.left);
        } else if a.right && !a.left && a.top == a.bottom {
            r = r.max(z + ly.margin.right);
        }
    }
    Rect::new(l as f32, t as f32, (m.logical_width - l - r) as f32, (m.logical_height - t - b) as f32)
}

fn app_matches(pattern: &str, app_id: Option<&str>) -> bool {
    pattern.split(',').map(str::trim).any(|p| p == "*" || Some(p) == app_id)
}

struct Tracker {
    app_id: String,
    target: Option<String>,
    strip: Option<(f32, f32, usize)>,
}

impl Tracker {
    fn compute(&mut self, clients: &[Client], ws: &HashMap<String, String>, mons: &[Monitor], layers: &[Layer], p: &Params) -> Option<Scene> {
        let visible = |c: &Client| {
            !c.minimized
                && !c.special
                && c.tab_group.as_ref().map_or(true, |t| t.front)
                && match (&c.output, &c.workspace) {
                    (Some(o), Some(w)) => ws.get(o) == Some(w),
                    _ => false,
                }
        };
        let is_term = |c: &Client| app_matches(&self.app_id, c.app_id.as_deref());
        if let Some(f) = clients.iter().find(|c| c.focused && is_term(c)) {
            self.target = Some(f.id.clone());
        }
        let mut target = self.target.as_ref().and_then(|id| clients.iter().find(|c| &c.id == id));
        if target.is_none() {
            let focus_out = clients
                .iter()
                .find(|c| c.focused)
                .and_then(|c| c.output.clone())
                .or_else(|| mons.iter().find(|m| m.focused).map(|m| m.name.clone()));
            target = clients
                .iter()
                .filter(|c| is_term(c) && visible(c))
                .max_by_key(|c| ((c.output == focus_out) as i64, c.width as i64 * c.height as i64));
            self.target = target.map(|c| c.id.clone());
        }
        let t = target?;
        let out = t.output.clone()?;
        let m = mons.iter().find(|m| m.name == out && m.enabled)?;
        let local = |c: &Client| Rect::new((c.x - m.x) as f32, (c.y - m.y) as f32, c.width as f32, c.height as f32);
        let on_screen: Vec<&Client> = clients.iter().filter(|c| c.output.as_deref() == Some(out.as_str()) && visible(c)).collect();
        let game = on_screen.iter().any(|c| c.fullscreen);
        let obstacles: Vec<Rect> = on_screen.iter().filter(|c| c.id != t.id && !c.floating).map(|c| local(c)).collect();
        let area = reserved_area(m, layers);
        let rect = local(t);
        let mut caps = side_caps(rect, area, &obstacles, t.fullscreen || t.maximized, p);
        if let (Some((width, offset, min)), Some(gid)) = (self.strip, t.tab_group.as_ref().and_then(|g| g.id)) {
            let members = clients.iter().filter(|c| c.tab_group.as_ref().and_then(|g| g.id) == Some(gid)).count();
            if members >= min {
                if let Some((side, cap)) = tab_strip_cap(free_space(rect, area, &obstacles, p.border).0, width, offset) {
                    caps[side] = caps[side].min(cap);
                }
            }
        }
        let mut fullscreen_outputs: Vec<String> = clients.iter().filter(|c| c.fullscreen && visible(c)).filter_map(|c| c.output.clone()).collect();
        fullscreen_outputs.dedup();
        Some(Scene { output: out, rect, caps, visible: visible(t), game, fullscreen_outputs })
    }
}

pub fn spawn(cfg: Config, scene: Arc<Mutex<Option<Scene>>>, ctl: Arc<Control>, demo: bool) {
    std::thread::Builder::new()
        .name("zoo".into())
        .spawn(move || {
            let params = Params {
                border: cfg.border,
                min_open: cfg.min_open,
                max_out: cfg.max_out,
                max_over: cfg.max_over,
                max_over_frac: cfg.max_over_frac,
            };
            let mut tracker = Tracker { app_id: cfg.app_id.clone(), target: None, strip: (cfg.tab_min > 0).then_some((cfg.tab_strip_width, cfg.tab_strip_offset, cfg.tab_min)) };
            let mut mons: Vec<Monitor> = Vec::new();
            let mut layers: Vec<Layer> = Vec::new();
            let mut slow_at: Option<Instant> = None;
            let mut warned = false;
            let mut failing: Option<Instant> = None;
            loop {
                let Some(path) = socket_path() else {
                    eprintln!("soundwav: XDG_RUNTIME_DIR or WAYLAND_DISPLAY unset");
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                };
                let result = (|| -> Result<Option<Scene>, String> {
                    if slow_at.map_or(true, |t| t.elapsed() > Duration::from_secs(1)) {
                        let m: Monitors = serde_json::from_str(&request(&path, "monitors-json")?).map_err(|e| e.to_string())?;
                        mons = m.outputs;
                        layers = serde_json::from_str(&request(&path, "layers-json")?).map_err(|e| e.to_string())?;
                        slow_at = Some(Instant::now());
                    }
                    let clients = parse_clients(&request(&path, "clients-json")?)?;
                    let ws = parse_ws(&request(&path, &format!("eval {WS_QUERY}"))?);
                    Ok(tracker.compute(&clients, &ws, &mons, &layers, &params))
                })();
                match result {
                    Ok(s) => {
                        warned = false;
                        failing = None;
                        if !demo {
                            ctl.set_paused(s.as_ref().map_or(true, |s| !s.visible || s.game));
                        }
                        *scene.lock().unwrap() = s;
                        std::thread::sleep(Duration::from_millis(66));
                    }
                    Err(e) => {
                        if !warned {
                            eprintln!("soundwav: zoolander control socket: {e}");
                            warned = true;
                        }
                        let since = *failing.get_or_insert_with(Instant::now);
                        if since.elapsed() > Duration::from_secs(3) {
                            *scene.lock().unwrap() = None;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        })
        .expect("zoo thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_patterns() {
        assert!(app_matches("*", Some("firefox")));
        assert!(app_matches("*", None));
        assert!(app_matches("kitty", Some("kitty")));
        assert!(!app_matches("kitty", Some("foot")));
        assert!(app_matches("kitty, foot", Some("foot")));
    }

    #[test]
    fn ws_parse() {
        let m = parse_ws("DP-1=2;HDMI-A-1=12;");
        assert_eq!(m.get("DP-1").unwrap(), "2");
        assert_eq!(m.get("HDMI-A-1").unwrap(), "12");
    }

    #[test]
    fn area_from_bar_and_dock() {
        let m = Monitor { name: "DP-1".into(), enabled: true, focused: true, x: 0, y: 0, logical_width: 2560, logical_height: 1440 };
        let bar = Layer { namespace: "otter-bar".into(), output: "DP-1".into(), anchor: Anchor { top: true, bottom: false, left: true, right: true }, exclusive_zone: 38, margin: Margin::default(), mapped: true };
        let dock = Layer { namespace: "otter-dock".into(), output: "DP-1".into(), anchor: Anchor { top: false, bottom: true, left: false, right: false }, exclusive_zone: 84, margin: Margin::default(), mapped: true };
        let own = Layer { namespace: "soundwav-reserve".into(), output: "DP-1".into(), anchor: Anchor { top: false, bottom: false, left: false, right: true }, exclusive_zone: 40, margin: Margin::default(), mapped: true };
        let a = reserved_area(&m, &[bar, dock, own]);
        assert_eq!(a, Rect::new(0.0, 38.0, 2560.0, 1318.0));
    }
}

#[cfg(test)]
mod null_id_tests {
    use super::parse_clients;

    #[test]
    fn popup_without_id_is_skipped() {
        let raw = r#"[{"id":null,"app_id":"steam","x":1,"y":2,"width":3,"height":4,"floating":true},{"id":"zoo-toplevel-1","app_id":"kitty","x":0,"y":0,"width":10,"height":10}]"#;
        let c = parse_clients(raw).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].id, "zoo-toplevel-1");
    }
}
