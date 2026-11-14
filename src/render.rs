use crate::cava::Audio;
use crate::config::{self, Config, LayerKind, Style};
use crate::gpu::{to_linear, Batch, Gpu, Inst, Uniforms, WUniforms};
use crate::layout::{self, Bar, Rect};
use crate::reserve::Reserves;
use crate::theme::Colours;
use crate::waves::Waves;
use crate::zoo::Scene;
use crate::DAEMON;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
    shm::{Shm, ShmHandler},
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, QueueHandle,
};

const FRAME_TIMEOUT: Duration = Duration::from_secs(1);
const IDLE_TICK: Duration = Duration::from_millis(50);
const QUIET_AFTER: Duration = Duration::from_millis(1200);
const DEBUG_EVERY: Duration = Duration::from_secs(2);

impl From<LayerKind> for Layer {
    fn from(k: LayerKind) -> Self {
        match k {
            LayerKind::Background => Layer::Background,
            LayerKind::Bottom => Layer::Bottom,
            LayerKind::Top => Layer::Top,
            LayerKind::Overlay => Layer::Overlay,
        }
    }
}

struct OutSurf {
    gsurf: wgpu::Surface<'static>,
    layer: LayerSurface,
    output: wl_output::WlOutput,
    name: String,
    scale: i32,
    width: u32,
    height: u32,
    format: Option<wgpu::TextureFormat>,
    srgb: bool,
    configured: bool,
    frame_pending: Option<Instant>,
    needs_clear: bool,
}

struct Anim {
    output: Option<String>,
    rect: Rect,
    caps: [f32; 4],
    alpha: f32,
    cleared: bool,
}

pub struct App {
    conn: Connection,
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    gpu: Gpu,
    reserves: Reserves,
    surfs: Vec<OutSurf>,
    cfg: Config,
    colours: Colours,
    audio: Arc<Mutex<Audio>>,
    scene: Arc<Mutex<Option<Scene>>>,
    anim: Anim,
    last_tick: Instant,
    dbg_at: Instant,
    bars: Vec<Bar>,
    waves: Waves,
    batch: Batch,
}

impl App {
    fn configure_surface(&mut self, idx: usize) {
        let s = &mut self.surfs[idx];
        let Some(format) = self.gpu.configure(&s.gsurf, s.width, s.height, &s.name) else { return };
        s.format = Some(format);
        s.srgb = format.is_srgb();
        s.configured = true;
        s.needs_clear = true;
        s.frame_pending = None;
    }

    fn fill_batch(&mut self) {
        match config::style(&self.cfg) {
            Style::Waves => self.waves.build(self.anim.rect, self.anim.caps, &self.cfg, &mut self.batch),
            Style::Bars => {
                let pitch = self.cfg.bar_width + self.cfg.bar_gap;
                let (left, right) = {
                    let a = self.audio.lock().unwrap();
                    (a.left.clone(), a.right.clone())
                };
                layout::bars(self.anim.rect, self.cfg.corner_radius, self.anim.caps, pitch, self.cfg.border, &left, &right, &mut self.bars);
                let width = self.cfg.bar_width;
                self.batch.insts.extend(self.bars.iter().map(|b| Inst {
                    origin: b.origin,
                    dir: b.dir,
                    len: b.len,
                    cap: b.cap,
                    width,
                    over: if b.over { 1.0 } else { 0.0 },
                }));
            }
        }
    }

    fn draw(&mut self, idx: usize, qh: &QueueHandle<Self>, with_ring: bool) {
        let s = &self.surfs[idx];
        if !s.configured || !self.gpu.ready_for(s.format) {
            return;
        }
        let tex = match s.gsurf.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configure_surface(idx);
                return;
            }
            _ => return,
        };
        let srgb = s.srgb;
        let screen = [(s.width as f32).max(1.0), (s.height as f32).max(1.0)];
        self.batch.clear();
        if with_ring {
            self.fill_batch();
        }
        let lin = |c: [f32; 3]| if srgb { [to_linear(c[0]), to_linear(c[1]), to_linear(c[2]), 1.0] } else { [c[0], c[1], c[2], 1.0] };
        let (bars, waves) = (self.colours.bars, self.colours.waves);
        let u = Uniforms {
            screen,
            glow: self.cfg.glow,
            alpha: self.anim.alpha,
            c0: lin(bars[0]),
            c1: lin(bars[1]),
            c2: lin(bars[2]),
            over_alpha: self.cfg.over_alpha,
            pad: [0.0; 3],
        };
        let wu = WUniforms {
            screen,
            glow: self.cfg.glow,
            alpha: self.anim.alpha,
            c0: lin(waves[0]),
            c1: lin(waves[1]),
            c2: lin(waves[2]),
            over_alpha: self.cfg.over_alpha,
            line: self.cfg.wave_line,
            fill: self.cfg.wave_fill,
            pad: 0.0,
        };
        self.gpu.render(&tex, &u, &wu, &self.batch);
        let s = &mut self.surfs[idx];
        let ws = s.layer.wl_surface();
        ws.frame(qh, ws.clone());
        s.frame_pending = Some(Instant::now());
        self.gpu.queue.present(tex);
    }

    fn mark_clear(&mut self, output: &str) {
        for s in self.surfs.iter_mut().filter(|s| s.name == output) {
            s.needs_clear = true;
        }
    }

    fn follow(&mut self, sc: &Scene, k: f32) {
        if self.anim.output.as_deref() != Some(sc.output.as_str()) {
            if let Some(old) = self.anim.output.take() {
                self.mark_clear(&old);
            }
            self.anim.output = Some(sc.output.clone());
            self.anim.rect = sc.rect;
            self.anim.caps = sc.caps;
            self.anim.alpha = 0.0;
        } else if self.anim.alpha <= 0.0 {
            self.anim.rect = sc.rect;
            self.anim.caps = sc.caps;
        } else {
            self.anim.rect = self.anim.rect.lerp(&sc.rect, k);
            for i in 0..4 {
                self.anim.caps[i] += (sc.caps[i] - self.anim.caps[i]) * k;
            }
        }
    }

    fn tick(&mut self, qh: &QueueHandle<Self>) -> Duration {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;
        let audio_on = {
            let a = self.audio.lock().unwrap();
            self.waves.update(&a, self.cfg.wave_speed, dt);
            a.loud.is_some_and(|t| now.duration_since(t) < QUIET_AFTER)
        };
        self.colours.update(&self.cfg, now, dt);
        let scene = self.scene.lock().unwrap().clone();
        let follow = 1.0 - (-dt / (self.cfg.follow_ms.max(1.0) / 1000.0)).exp();
        let fade = 1.0 - (-dt / (self.cfg.fade_ms.max(1.0) / 1000.0)).exp();
        let mut want = false;
        if let Some(sc) = &scene {
            self.follow(sc, follow);
            want = sc.visible && !sc.game && audio_on;
        }
        let target = if want { self.cfg.opacity } else { 0.0 };
        self.anim.alpha += (target - self.anim.alpha) * fade;
        if target == 0.0 && self.anim.alpha < 0.004 {
            self.anim.alpha = 0.0;
        }
        if self.anim.alpha > 0.0 {
            self.anim.cleared = false;
        } else if !self.anim.cleared {
            self.anim.cleared = true;
            if let Some(o) = self.anim.output.clone() {
                self.mark_clear(&o);
            }
        }
        if std::env::var_os("SOUNDWAV_DEBUG").is_some() && now.duration_since(self.dbg_at) > DEBUG_EVERY {
            self.dbg_at = now;
            let confs: Vec<String> = self.surfs.iter().map(|s| format!("{}:{}x{}@{} conf={} pend={}", s.name, s.width, s.height, s.scale, s.configured, s.frame_pending.is_some())).collect();
            eprintln!("soundwav: scene={scene:?} audio_on={audio_on} alpha={:.2} out={:?} surfs={confs:?}", self.anim.alpha, self.anim.output);
        }
        if let Some(sc) = &scene {
            let outs: Vec<(String, wl_output::WlOutput)> = self.surfs.iter().map(|s| (s.name.clone(), s.output.clone())).collect();
            self.reserves.sync(&self.compositor, &self.layer_shell, qh, &outs, &sc.fullscreen_outputs);
        }
        let mut busy = self.anim.alpha > 0.0;
        for i in 0..self.surfs.len() {
            let s = &mut self.surfs[i];
            if let Some(t) = s.frame_pending {
                if now.duration_since(t) > FRAME_TIMEOUT {
                    s.frame_pending = None;
                } else {
                    busy |= s.needs_clear;
                    continue;
                }
            }
            if !s.configured {
                continue;
            }
            let active = self.anim.output.as_deref() == Some(s.name.as_str()) && self.anim.alpha > 0.0;
            if active {
                s.needs_clear = false;
                self.draw(i, qh, true);
            } else if s.needs_clear {
                s.needs_clear = false;
                self.draw(i, qh, false);
            }
        }
        let _ = self.conn.flush();
        if busy {
            Duration::from_secs_f64(1.0 / self.cfg.fps as f64)
        } else {
            IDLE_TICK
        }
    }

    fn add_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.reserves.add(&self.compositor, &self.layer_shell, qh, &output);
        let info = self.outputs.info(&output);
        let name = info.as_ref().and_then(|i| i.name.clone()).unwrap_or_default();
        let scale = info.as_ref().map(|i| i.scale_factor).unwrap_or(1).max(1);
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(qh, surface, self.cfg.layer.into(), Some(DAEMON.namespace()), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        if let Ok(r) = Region::new(&self.compositor) {
            layer.set_input_region(Some(r.wl_region()));
        }
        if let Err(e) = layer.set_buffer_scale(1) {
            eprintln!("soundwav: buffer scale 1 on {name}: {e:?}, the ring may render at the output scale");
        }
        layer.commit();
        let gsurf = match self.gpu.surface(&self.conn, layer.wl_surface()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("soundwav: wgpu surface for {name}: {e}");
                return;
            }
        };
        self.surfs.push(OutSurf {
            gsurf,
            layer,
            output,
            name,
            scale,
            width: 0,
            height: 0,
            format: None,
            srgb: false,
            configured: false,
            frame_pending: None,
            needs_clear: true,
        });
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(s) = self.surfs.iter_mut().find(|s| s.layer.wl_surface() == surface) {
            s.scale = factor.max(1);
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(s) = self.surfs.iter_mut().find(|s| s.layer.wl_surface() == surface) {
            s.frame_pending = None;
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_output(qh, output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Some(info) = self.outputs.info(&output) {
            if let Some(s) = self.surfs.iter_mut().find(|s| s.output == output) {
                if let Some(n) = info.name {
                    s.name = n;
                }
            }
        }
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.surfs.retain(|s| s.output != output);
        self.reserves.drop_output(&output);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.surfs.retain(|s| s.layer.wl_surface() != layer.wl_surface());
        self.reserves.drop_surface(layer.wl_surface());
    }
    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let (w, h) = configure.new_size;
        if self.reserves.configure(layer.wl_surface(), w, h) {
            return;
        }
        let Some(i) = self.surfs.iter().position(|s| s.layer.wl_surface() == layer.wl_surface()) else { return };
        let s = &mut self.surfs[i];
        if s.configured && s.width == w && s.height == h {
            return;
        }
        s.width = w.max(1);
        s.height = h.max(1);
        self.configure_surface(i);
        self.draw(i, qh, false);
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_registry!(App);
delegate_shm!(App);

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.reserves.shm
    }
}

pub fn run(cfg: Config, audio: Arc<Mutex<Audio>>, scene: Arc<Mutex<Option<Scene>>>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
    let (globals, event_queue) = registry_queue_init(&conn).map_err(|e| format!("wayland registry: {e}"))?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("layer shell: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
    let probe = compositor.create_surface(&qh);
    let gpu = Gpu::new(&conn, &probe);
    probe.destroy();
    let gpu = gpu?;
    let mut app = App {
        conn: conn.clone(),
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        gpu,
        reserves: Reserves::new(shm, cfg.reserve),
        surfs: Vec::new(),
        colours: Colours::new(&cfg),
        cfg,
        audio,
        scene,
        anim: Anim { output: None, rect: Rect::default(), caps: [0.0; 4], alpha: 0.0, cleared: true },
        last_tick: Instant::now(),
        dbg_at: Instant::now(),
        bars: Vec::new(),
        waves: Waves::default(),
        batch: Batch::default(),
    };
    let mut event_loop: EventLoop<App> = EventLoop::try_new().map_err(|e| format!("event loop: {e}"))?;
    WaylandSource::new(conn, event_queue).insert(event_loop.handle()).map_err(|e| format!("wayland source: {e}"))?;
    let tqh = qh.clone();
    event_loop
        .handle()
        .insert_source(Timer::immediate(), move |_, _, app: &mut App| TimeoutAction::ToDuration(app.tick(&tqh)))
        .map_err(|e| format!("timer: {e}"))?;
    loop {
        event_loop.dispatch(None, &mut app).map_err(|e| format!("dispatch: {e}"))?;
    }
}
