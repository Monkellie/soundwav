use crate::render::App;
use crate::DAEMON;
use smithay_client_toolkit::{
    compositor::{CompositorState, Region},
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm},
};
use wayland_client::{
    protocol::{wl_output, wl_shm, wl_surface},
    QueueHandle,
};

const EDGES: [(Anchor, (u32, u32)); 4] = [
    (Anchor::TOP.union(Anchor::LEFT).union(Anchor::RIGHT), (0, 1)),
    (Anchor::RIGHT.union(Anchor::TOP).union(Anchor::BOTTOM), (1, 0)),
    (Anchor::BOTTOM.union(Anchor::LEFT).union(Anchor::RIGHT), (0, 1)),
    (Anchor::LEFT.union(Anchor::TOP).union(Anchor::BOTTOM), (1, 0)),
];

struct Zone {
    layer: LayerSurface,
    output: wl_output::WlOutput,
}

pub struct Reserves {
    pub shm: Shm,
    zones: [f32; 4],
    pool: Option<SlotPool>,
    list: Vec<Zone>,
    pool_failed: bool,
}

impl Reserves {
    pub fn new(shm: Shm, zones: [f32; 4]) -> Self {
        Self { shm, zones, pool: None, list: Vec::new(), pool_failed: false }
    }

    fn enabled(&self) -> bool {
        self.zones.iter().any(|z| z.round() > 0.0)
    }

    pub fn add(&mut self, compositor: &CompositorState, shell: &LayerShell, qh: &QueueHandle<App>, output: &wl_output::WlOutput) {
        for (i, (anchor, size)) in EDGES.into_iter().enumerate() {
            let z = self.zones[i].round() as i32;
            if z <= 0 {
                continue;
            }
            let surface = compositor.create_surface(qh);
            let layer = shell.create_layer_surface(qh, surface, Layer::Overlay, Some(DAEMON.sub_namespace("reserve")), Some(output));
            layer.set_anchor(anchor);
            layer.set_size(size.0, size.1);
            layer.set_exclusive_zone(z);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            if let Ok(r) = Region::new(compositor) {
                layer.set_input_region(Some(r.wl_region()));
            }
            layer.commit();
            self.list.push(Zone { layer, output: output.clone() });
        }
    }

    pub fn sync(&mut self, compositor: &CompositorState, shell: &LayerShell, qh: &QueueHandle<App>, outputs: &[(String, wl_output::WlOutput)], fullscreen: &[String]) {
        if !self.enabled() {
            return;
        }
        for (name, output) in outputs {
            let has = self.list.iter().any(|r| &r.output == output);
            let off = fullscreen.contains(name);
            if off && has {
                self.list.retain(|r| &r.output != output);
            } else if !off && !has {
                self.add(compositor, shell, qh, output);
            }
        }
    }

    pub fn drop_output(&mut self, output: &wl_output::WlOutput) {
        self.list.retain(|r| &r.output != output);
    }

    pub fn drop_surface(&mut self, surface: &wl_surface::WlSurface) {
        self.list.retain(|r| r.layer.wl_surface() != surface);
    }

    pub fn configure(&mut self, surface: &wl_surface::WlSurface, w: u32, h: u32) -> bool {
        let Some(i) = self.list.iter().position(|r| r.layer.wl_surface() == surface) else { return false };
        let (w, h) = (w.max(1) as i32, h.max(1) as i32);
        if self.pool.is_none() && !self.pool_failed {
            match SlotPool::new((w * h * 4).max(4096) as usize, &self.shm) {
                Ok(p) => self.pool = Some(p),
                Err(e) => {
                    eprintln!("soundwav: reserve zone buffer pool: {e}, zones stay unmapped");
                    self.pool_failed = true;
                }
            }
        }
        let Some(pool) = self.pool.as_mut() else { return true };
        let (buffer, canvas) = match pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("soundwav: reserve zone buffer: {e}");
                return true;
            }
        };
        canvas.fill(0);
        let layer = &self.list[i].layer;
        let ws = layer.wl_surface();
        ws.damage_buffer(0, 0, w, h);
        match buffer.attach_to(ws) {
            Ok(()) => layer.commit(),
            Err(e) => eprintln!("soundwav: reserve zone attach: {e}"),
        }
        true
    }
}
