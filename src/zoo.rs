use crate::cava::Control;
use crate::config::{AppMatch, Config};
use crate::layout::{free_space, side_caps, tab_strip_cap, Params, Rect};
use crate::DAEMON;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zoo_ipc::zoolander::{self, Client, Snapshot};

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub output: String,
    pub rect: Rect,
    pub caps: [f32; 4],
    pub visible: bool,
    pub game: bool,
    pub fullscreen_outputs: Vec<String>,
}

struct Tracker {
    apps: AppMatch,
    target: Option<String>,
    strip: Option<(f32, f32, usize)>,
}

fn visible(c: &Client, ws: &HashMap<String, String>) -> bool {
    c.shown_on(ws) && c.in_front()
}

impl Tracker {
    fn compute(&mut self, snap: &Snapshot, p: &Params) -> Option<Scene> {
        let clients = &snap.clients;
        let ws = &snap.workspaces;
        let wanted = |c: &Client| self.apps.matches(c.app_id.as_deref());
        if let Some(f) = clients.iter().find(|c| c.focused && wanted(c)) {
            self.target = Some(f.id.clone());
        }
        let mut target = self.target.as_ref().and_then(|id| clients.iter().find(|c| &c.id == id));
        if target.is_none() {
            let focus_out = clients
                .iter()
                .find(|c| c.focused)
                .and_then(|c| c.output.clone())
                .or_else(|| snap.monitors.iter().find(|m| m.focused).map(|m| m.name.clone()));
            target = clients
                .iter()
                .filter(|c| wanted(c) && visible(c, ws))
                .max_by_key(|c| ((c.output == focus_out) as i64, c.width as i64 * c.height as i64));
            self.target = target.map(|c| c.id.clone());
        }
        let t = target?;
        let out = t.output.clone()?;
        let m = snap.monitor(&out)?;
        let on_screen: Vec<&Client> = clients.iter().filter(|c| c.output.as_deref() == Some(out.as_str()) && visible(c, ws)).collect();
        let game = on_screen.iter().any(|c| c.fullscreen);
        let obstacles: Vec<Rect> = on_screen.iter().filter(|c| c.id != t.id && !c.floating).map(|c| c.local(m).into()).collect();
        let area: Rect = m.usable(&snap.layers, |l| DAEMON.owns_namespace(&l.namespace)).into();
        let rect: Rect = t.local(m).into();
        let mut caps = side_caps(rect, area, &obstacles, t.fullscreen || t.maximized, p);
        if let (Some((width, offset, min)), Some(gid)) = (self.strip, t.group()) {
            let members = clients.iter().filter(|c| c.group() == Some(gid)).count();
            if members >= min {
                if let Some((side, cap)) = tab_strip_cap(free_space(rect, area, &obstacles, p.border).0, width, offset) {
                    caps[side] = caps[side].min(cap);
                }
            }
        }
        let mut fullscreen_outputs: Vec<String> = clients.iter().filter(|c| c.fullscreen && visible(c, ws)).filter_map(|c| c.output.clone()).collect();
        fullscreen_outputs.dedup();
        Some(Scene { output: out, rect, caps, visible: visible(t, ws), game, fullscreen_outputs })
    }
}

pub fn spawn(cfg: Config, scene: Arc<Mutex<Option<Scene>>>, ctl: Arc<Control>, demo: bool) {
    let params = Params {
        border: cfg.border,
        min_open: cfg.min_open,
        max_out: cfg.max_out,
        max_over: cfg.max_over,
        max_over_frac: cfg.max_over_frac,
    };
    let mut tracker = Tracker { apps: cfg.app_id.clone(), target: None, strip: (cfg.tab_min > 0).then_some((cfg.tab_strip_width, cfg.tab_strip_offset, cfg.tab_min)) };
    let lost = scene.clone();
    std::thread::Builder::new()
        .name("zoo".into())
        .spawn(move || {
            zoolander::watch(
                DAEMON.name(),
                |_, snap| {
                    let s = tracker.compute(snap, &params);
                    if !demo {
                        ctl.set_paused(s.as_ref().is_none_or(|s| !s.visible || s.game));
                    }
                    *scene.lock().unwrap() = s;
                    Ok(())
                },
                || *lost.lock().unwrap() = None,
            )
        })
        .expect("zoo thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoo_ipc::zoolander::{Layer, Monitor};

    fn client(id: &str, app: &str, x: i32, w: i32, focused: bool) -> Client {
        serde_json::from_value(serde_json::json!({
            "id": id, "app_id": app, "workspace": "1", "output": "DP-1",
            "x": x, "y": 100, "width": w, "height": 600, "focused": focused
        }))
        .unwrap()
    }

    fn snap(clients: Vec<Client>, layers: Vec<Layer>) -> Snapshot {
        let m: Monitor = serde_json::from_value(serde_json::json!({
            "name": "DP-1", "x": 0, "y": 0, "logical_width": 2560, "logical_height": 1440
        }))
        .unwrap();
        Snapshot { clients, monitors: vec![m], layers, workspaces: [("DP-1".to_string(), "1".to_string())].into() }
    }

    fn params() -> Params {
        Params { border: 5.0, min_open: 60.0, max_out: 220.0, max_over: 56.0, max_over_frac: 0.12 }
    }

    #[test]
    fn follows_the_focused_match() {
        let mut t = Tracker { apps: AppMatch::List(vec!["kitty".into()]), target: None, strip: None };
        let s = snap(vec![client("zoo-toplevel-1", "kitty", 100, 800, false), client("zoo-toplevel-2", "foot", 1200, 800, true)], vec![]);
        let scene = t.compute(&s, &params()).unwrap();
        assert_eq!(scene.rect, Rect::new(100.0, 100.0, 800.0, 600.0));
        assert!(scene.visible && !scene.game);
    }

    #[test]
    fn own_reserve_zones_do_not_shrink_the_area() {
        let reserve: Layer = serde_json::from_value(serde_json::json!({
            "namespace": DAEMON.sub_namespace("reserve"), "output": "DP-1",
            "anchor": {"top": false, "bottom": false, "left": false, "right": true}, "exclusive_zone": 2000
        }))
        .unwrap();
        let mut t = Tracker { apps: AppMatch::Any, target: None, strip: None };
        let s = snap(vec![client("zoo-toplevel-1", "kitty", 100, 800, true)], vec![reserve]);
        let scene = t.compute(&s, &params()).unwrap();
        assert!(scene.caps[1] > 0.0);
    }
}
