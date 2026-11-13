use crate::cava::{bands, Audio};
use crate::config::{palette, wave_palette, Config};
use crate::layout::{self, Bar, Rect, WaveIn, WaveSample};
use crate::zoo::Scene;
use bytemuck::{Pod, Zeroable};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
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
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, Proxy, QueueHandle,
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    screen: [f32; 2],
    glow: f32,
    alpha: f32,
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    over_alpha: f32,
    pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    origin: [f32; 2],
    dir: [f32; 2],
    len: f32,
    cap: f32,
    width: f32,
    over: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WUniforms {
    screen: [f32; 2],
    glow: f32,
    alpha: f32,
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    over_alpha: f32,
    line: f32,
    fill: f32,
    pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WVert {
    pos: [f32; 2],
    v: f32,
    kind: f32,
    band: f32,
    over: f32,
}

struct Pipe {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    ibuf: wgpu::Buffer,
    icap: usize,
    wpipeline: wgpu::RenderPipeline,
    wubuf: wgpu::Buffer,
    wbind: wgpu::BindGroup,
    vbuf: wgpu::Buffer,
    vcap: usize,
}

#[derive(Default)]
struct Env {
    fast: [[f32; 3]; 2],
    slow_bass: [f32; 2],
    kick: [f32; 2],
    phase: [f32; 3],
}

const BAND_GAIN: [f32; 3] = [1.0, 1.25, 1.8];
const BAND_SPEED: [f32; 3] = [0.6, 1.4, 3.2];

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipe: Option<Pipe>,
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

struct Reserve {
    layer: LayerSurface,
    output: wl_output::WlOutput,
}

struct Anim {
    output: Option<String>,
    rect: Rect,
    caps: [f32; 4],
    alpha: f32,
    cleared: bool,
}

struct App {
    conn: Connection,
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    gpu: Gpu,
    shm: Shm,
    pool: Option<SlotPool>,
    reserves: Vec<Reserve>,
    surfs: Vec<OutSurf>,
    cfg: Config,
    colors: [[f32; 3]; 3],
    audio: Arc<Mutex<Audio>>,
    scene: Arc<Mutex<Option<Scene>>>,
    anim: Anim,
    last_tick: Instant,
    dbg_at: Instant,
    bars: Vec<Bar>,
    insts: Vec<Inst>,
    wcolors: [[f32; 3]; 3],
    env: Env,
    samples: Vec<WaveSample>,
    verts: Vec<WVert>,
    strips: Vec<(u32, u32)>,
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn make_pipe(gpu: &Gpu, format: wgpu::TextureFormat) -> Pipe {
    let d = &gpu.device;
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bars"),
        source: wgpu::ShaderSource::Wgsl(include_str!("bars.wgsl").into()),
    });
    let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("u"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bars"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32, 4 => Float32, 5 => Float32];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Inst>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &attrs,
    })];
    let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bars"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &buffers },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let ubuf = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("u"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("u"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
    });
    let icap = 4096;
    let ibuf = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("inst"),
        size: (icap * std::mem::size_of::<Inst>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let wmodule = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("waves"),
        source: wgpu::ShaderSource::Wgsl(include_str!("waves.wgsl").into()),
    });
    let wattrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Float32, 4 => Float32];
    let wbuffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<WVert>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wattrs,
    })];
    let wpipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("waves"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &wmodule, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &wbuffers },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &wmodule,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let wubuf = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wu"),
        size: std::mem::size_of::<WUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let wbind = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("wu"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wubuf.as_entire_binding() }],
    });
    let vcap = 32768;
    let vbuf = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wverts"),
        size: (vcap * std::mem::size_of::<WVert>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Pipe { format, pipeline, ubuf, bind, ibuf, icap, wpipeline, wubuf, wbind, vbuf, vcap }
}

impl App {
    fn configure_surface(&mut self, idx: usize) {
        let s = &mut self.surfs[idx];
        let (bw, bh) = (s.width.max(1), s.height.max(1));
        let caps = s.gsurf.get_capabilities(&self.gpu.adapter);
        let Some(mut conf) = s.gsurf.get_default_config(&self.gpu.adapter, bw, bh) else {
            eprintln!("soundwav: surface on {} not supported by the adapter", s.name);
            return;
        };
        let alpha = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Inherit, wgpu::CompositeAlphaMode::PostMultiplied]
            .into_iter()
            .find(|m| caps.alpha_modes.contains(m));
        let Some(alpha) = alpha else {
            eprintln!("soundwav: no transparent alpha mode on {} ({:?}), not drawing", s.name, caps.alpha_modes);
            return;
        };
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(conf.format);
        conf.format = format;
        conf.alpha_mode = alpha;
        conf.present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        conf.desired_maximum_frame_latency = 1;
        conf.view_formats = vec![];
        s.gsurf.configure(&self.gpu.device, &conf);
        s.format = Some(format);
        s.srgb = format.is_srgb();
        s.configured = true;
        s.needs_clear = true;
        s.frame_pending = None;
        if self.gpu.pipe.as_ref().map(|p| p.format) != Some(format) {
            self.gpu.pipe = Some(make_pipe(&self.gpu, format));
        }
    }

    fn draw(&mut self, idx: usize, qh: &QueueHandle<Self>, with_bars: bool) {
        let s = &self.surfs[idx];
        if !s.configured || s.format != self.gpu.pipe.as_ref().map(|p| p.format) {
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
        let (w, h) = (s.width as f32, s.height as f32);
        self.insts.clear();
        self.verts.clear();
        self.strips.clear();
        let wavy = crate::config::wavy(&self.cfg);
        if with_bars && wavy {
            self.build_waves();
        } else if with_bars {
            let pitch = self.cfg.bar_width + self.cfg.bar_gap;
            let (left, right) = {
                let a = self.audio.lock().unwrap();
                (a.left.clone(), a.right.clone())
            };
            layout::bars(self.anim.rect, self.cfg.corner_radius, self.anim.caps, pitch, self.cfg.border, &left, &right, &mut self.bars);
            self.insts.extend(self.bars.iter().map(|b| Inst {
                origin: b.origin,
                dir: b.dir,
                len: b.len,
                cap: b.cap,
                width: self.cfg.bar_width,
                over: if b.over { 1.0 } else { 0.0 },
            }));
        }
        let lin = |c: [f32; 3]| if srgb { [to_linear(c[0]), to_linear(c[1]), to_linear(c[2]), 1.0] } else { [c[0], c[1], c[2], 1.0] };
        let u = Uniforms {
            screen: [w.max(1.0), h.max(1.0)],
            glow: self.cfg.glow,
            alpha: self.anim.alpha,
            c0: lin(self.colors[0]),
            c1: lin(self.colors[1]),
            c2: lin(self.colors[2]),
            over_alpha: self.cfg.over_alpha,
            pad: [0.0; 3],
        };
        let wu = WUniforms {
            screen: u.screen,
            glow: self.cfg.glow,
            alpha: self.anim.alpha,
            c0: lin(self.wcolors[0]),
            c1: lin(self.wcolors[1]),
            c2: lin(self.wcolors[2]),
            over_alpha: self.cfg.over_alpha,
            line: self.cfg.wave_line,
            fill: self.cfg.wave_fill,
            pad: 0.0,
        };
        let need = self.insts.len();
        let vneed = self.verts.len();
        let gpu = &mut self.gpu;
        let pipe = gpu.pipe.as_mut().unwrap();
        if vneed > pipe.vcap {
            pipe.vcap = vneed.next_power_of_two();
            pipe.vbuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("wverts"),
                size: (pipe.vcap * std::mem::size_of::<WVert>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if vneed > 0 {
            gpu.queue.write_buffer(&pipe.wubuf, 0, bytemuck::bytes_of(&wu));
            gpu.queue.write_buffer(&pipe.vbuf, 0, bytemuck::cast_slice(&self.verts));
        }
        if need > pipe.icap {
            pipe.icap = need.next_power_of_two();
            pipe.ibuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("inst"),
                size: (pipe.icap * std::mem::size_of::<Inst>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        gpu.queue.write_buffer(&pipe.ubuf, 0, bytemuck::bytes_of(&u));
        if need > 0 {
            gpu.queue.write_buffer(&pipe.ibuf, 0, bytemuck::cast_slice(&self.insts));
        }
        let view = tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("aura") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("aura"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if need > 0 {
                pass.set_pipeline(&pipe.pipeline);
                pass.set_bind_group(0, &pipe.bind, &[]);
                pass.set_vertex_buffer(0, pipe.ibuf.slice(..(need * std::mem::size_of::<Inst>()) as u64));
                pass.draw(0..6, 0..need as u32);
            }
            if vneed > 0 {
                pass.set_pipeline(&pipe.wpipeline);
                pass.set_bind_group(0, &pipe.wbind, &[]);
                pass.set_vertex_buffer(0, pipe.vbuf.slice(..(vneed * std::mem::size_of::<WVert>()) as u64));
                for (start, count) in &self.strips {
                    pass.draw(*start..start + count, 0..1);
                }
            }
        }
        gpu.queue.submit(Some(enc.finish()));
        let s = &mut self.surfs[idx];
        let ws = s.layer.wl_surface();
        ws.frame(qh, ws.clone());
        s.frame_pending = Some(Instant::now());
        self.gpu.queue.present(tex);
    }

    fn build_waves(&mut self) {
        let e = &self.env;
        let bass = |c: usize| (e.fast[c][0] + e.kick[c] * 0.6).clamp(0.0, 1.0);
        let w = WaveIn {
            env_l: [bass(0), e.fast[0][1], e.fast[0][2]],
            env_r: [bass(1), e.fast[1][1], e.fast[1][2]],
            phase: e.phase,
            floor: 2.0,
        };
        let cfg = &self.cfg;
        layout::waves(self.anim.rect, cfg.corner_radius, self.anim.caps, cfg.border, cfg.corner_blend, cfg.wave_step, &w, &mut self.samples);
        let n = self.samples.len();
        if n < 8 {
            return;
        }
        let hw = cfg.wave_line * 0.5 + cfg.glow * 2.5;
        for b in 0..3 {
            let crest = |i: usize| {
                let s = &self.samples[i % n];
                [s.base[0] + s.normal[0] * s.off[b], s.base[1] + s.normal[1] * s.off[b]]
            };
            let start = self.verts.len() as u32;
            for i in 0..=n {
                let s = &self.samples[i % n];
                let c = crest(i);
                self.verts.push(WVert { pos: s.base, v: 0.0, kind: 0.0, band: b as f32, over: s.over });
                self.verts.push(WVert { pos: c, v: 1.0, kind: 0.0, band: b as f32, over: s.over });
            }
            self.strips.push((start, self.verts.len() as u32 - start));
            let start = self.verts.len() as u32;
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
                self.verts.push(WVert { pos: [c[0] + nr[0] * outer, c[1] + nr[1] * outer], v: outer, kind: 1.0, band: b as f32, over: s.over });
                self.verts.push(WVert { pos: [c[0] - nr[0] * inner, c[1] - nr[1] * inner], v: -inner, kind: 1.0, band: b as f32, over: s.over });
            }
            self.strips.push((start, self.verts.len() as u32 - start));
        }
    }

    fn update_env(&mut self, dt: f32) {
        let (bl, br) = {
            let a = self.audio.lock().unwrap();
            (bands(&a.left), bands(&a.right))
        };
        let up = 1.0 - (-dt / 0.025).exp();
        let down = 1.0 - (-dt / 0.22).exp();
        let slow = 1.0 - (-dt / 0.6).exp();
        let kdecay = (-dt / 0.15).exp();
        for (c, src) in [bl, br].into_iter().enumerate() {
            for b in 0..3 {
                let target = (src[b] * BAND_GAIN[b]).clamp(0.0, 1.0);
                let cur = &mut self.env.fast[c][b];
                *cur += (target - *cur) * if target > *cur { up } else { down };
            }
            let f = self.env.fast[c][0];
            self.env.slow_bass[c] += (f - self.env.slow_bass[c]) * slow;
            self.env.kick[c] = (self.env.kick[c] * kdecay).max((f - self.env.slow_bass[c]) * 1.6).clamp(0.0, 1.0);
        }
        let wrap = std::f32::consts::TAU * 10.0;
        for b in 0..3 {
            let energy = 0.5 * (self.env.fast[0][b] + self.env.fast[1][b]);
            self.env.phase[b] = (self.env.phase[b] + BAND_SPEED[b] * self.cfg.wave_speed * (0.35 + energy) * dt).rem_euclid(wrap);
        }
    }

    fn tick(&mut self, qh: &QueueHandle<Self>) -> Duration {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;
        self.update_env(dt);
        let scene = self.scene.lock().unwrap().clone();
        let audio_on = self.audio.lock().unwrap().loud.is_some_and(|t| now.duration_since(t) < Duration::from_millis(1200));
        let follow = 1.0 - (-dt / (self.cfg.follow_ms.max(1.0) / 1000.0)).exp();
        let fade = 1.0 - (-dt / (self.cfg.fade_ms.max(1.0) / 1000.0)).exp();
        let mut want = false;
        if let Some(sc) = &scene {
            if self.anim.output.as_deref() != Some(sc.output.as_str()) {
                if let Some(old) = self.anim.output.take() {
                    for s in self.surfs.iter_mut().filter(|s| s.name == old) {
                        s.needs_clear = true;
                    }
                }
                self.anim.output = Some(sc.output.clone());
                self.anim.rect = sc.rect;
                self.anim.caps = sc.caps;
                self.anim.alpha = 0.0;
            } else if self.anim.alpha <= 0.0 {
                self.anim.rect = sc.rect;
                self.anim.caps = sc.caps;
            } else {
                self.anim.rect = self.anim.rect.lerp(&sc.rect, follow);
                for i in 0..4 {
                    self.anim.caps[i] += (sc.caps[i] - self.anim.caps[i]) * follow;
                }
            }
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
            if let Some(o) = &self.anim.output {
                for s in self.surfs.iter_mut().filter(|s| &s.name == o) {
                    s.needs_clear = true;
                }
            }
        }
        if std::env::var_os("SOUNDWAV_DEBUG").is_some() && now.duration_since(self.dbg_at) > Duration::from_secs(2) {
            self.dbg_at = now;
            let confs: Vec<String> = self.surfs.iter().map(|s| format!("{}:{}x{}@{} conf={} pend={}", s.name, s.width, s.height, s.scale, s.configured, s.frame_pending.is_some())).collect();
            eprintln!("soundwav: scene={scene:?} audio_on={audio_on} alpha={:.2} out={:?} surfs={confs:?}", self.anim.alpha, self.anim.output);
        }
        if let Some(sc) = &scene {
            self.sync_reserves(qh, &sc.fullscreen_outputs);
        }
        let mut busy = self.anim.alpha > 0.0;
        for i in 0..self.surfs.len() {
            let s = &mut self.surfs[i];
            if let Some(t) = s.frame_pending {
                if now.duration_since(t) > Duration::from_secs(1) {
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
                self.surfs[i].needs_clear = false;
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
            Duration::from_millis(50)
        }
    }

    fn add_reserves(&mut self, qh: &QueueHandle<Self>, output: &wl_output::WlOutput) {
        let edges = [
            (Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, (0, 1)),
            (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (1, 0)),
            (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (0, 1)),
            (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (1, 0)),
        ];
        for (i, (anchor, size)) in edges.into_iter().enumerate() {
            let z = self.cfg.reserve[i].round() as i32;
            if z <= 0 {
                continue;
            }
            let surface = self.compositor.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("soundwav-reserve"), Some(output));
            layer.set_anchor(anchor);
            layer.set_size(size.0, size.1);
            layer.set_exclusive_zone(z);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            if let Ok(r) = Region::new(&self.compositor) {
                layer.set_input_region(Some(r.wl_region()));
            }
            layer.commit();
            self.reserves.push(Reserve { layer, output: output.clone() });
        }
    }

    fn sync_reserves(&mut self, qh: &QueueHandle<Self>, fullscreen: &[String]) {
        if self.cfg.reserve.iter().all(|z| z.round() <= 0.0) {
            return;
        }
        let outs: Vec<(String, wl_output::WlOutput)> = self.surfs.iter().map(|s| (s.name.clone(), s.output.clone())).collect();
        for (name, output) in outs {
            let has = self.reserves.iter().any(|r| r.output == output);
            let off = fullscreen.contains(&name);
            if off && has {
                self.reserves.retain(|r| r.output != output);
            } else if !off && !has {
                self.add_reserves(qh, &output);
            }
        }
    }

    fn fill_reserve(&mut self, i: usize, w: u32, h: u32) {
        let (w, h) = (w.max(1) as i32, h.max(1) as i32);
        if self.pool.is_none() {
            self.pool = SlotPool::new((w * h * 4).max(4096) as usize, &self.shm).ok();
        }
        let Some(pool) = self.pool.as_mut() else { return };
        let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        canvas.fill(0);
        let layer = &self.reserves[i].layer;
        let ws = layer.wl_surface();
        ws.damage_buffer(0, 0, w, h);
        if buffer.attach_to(ws).is_ok() {
            layer.commit();
        }
    }

    fn add_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_reserves(qh, &output);
        let info = self.outputs.info(&output);
        let name = info.as_ref().and_then(|i| i.name.clone()).unwrap_or_default();
        let scale = info.as_ref().map(|i| i.scale_factor).unwrap_or(1).max(1);
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(qh, surface, layer_kind(&self.cfg.layer), Some("soundwav"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        if let Ok(r) = Region::new(&self.compositor) {
            layer.set_input_region(Some(r.wl_region()));
        }
        let _ = layer.set_buffer_scale(1);
        layer.commit();
        let gsurf = match wgpu_surface(&self.gpu.instance, &self.conn, layer.wl_surface()) {
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
        self.reserves.retain(|r| r.output != output);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.surfs.retain(|s| s.layer.wl_surface() != layer.wl_surface());
        self.reserves.retain(|r| r.layer.wl_surface() != layer.wl_surface());
    }
    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if let Some(i) = self.reserves.iter().position(|r| r.layer.wl_surface() == layer.wl_surface()) {
            self.fill_reserve(i, configure.new_size.0, configure.new_size.1);
            return;
        }
        let Some(i) = self.surfs.iter().position(|s| s.layer.wl_surface() == layer.wl_surface()) else { return };
        let (w, h) = configure.new_size;
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
        &mut self.shm
    }
}

fn wgpu_surface(instance: &wgpu::Instance, conn: &Connection, surface: &wl_surface::WlSurface) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
    let display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(NonNull::new(conn.backend().display_ptr() as *mut _).unwrap()));
    let window = RawWindowHandle::Wayland(WaylandWindowHandle::new(NonNull::new(surface.id().as_ptr() as *mut _).unwrap()));
    unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle { raw_display_handle: Some(display), raw_window_handle: window }) }
}

pub fn run(cfg: Config, audio: Arc<Mutex<Audio>>, scene: Arc<Mutex<Option<Scene>>>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
    let (globals, event_queue) = registry_queue_init(&conn).map_err(|e| format!("wayland registry: {e}"))?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("layer shell: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    let instance = wgpu::Instance::new(desc);
    let probe = compositor.create_surface(&qh);
    let adapter = {
        let psurf = wgpu_surface(&instance, &conn, &probe).map_err(|e| format!("gpu surface: {e}"))?;
        let mut found = None;
        for pref in [wgpu::PowerPreference::HighPerformance, wgpu::PowerPreference::LowPower] {
            if let Ok(a) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: pref,
                compatible_surface: Some(&psurf),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                found = Some(a);
                break;
            }
        }
        found.ok_or("no gpu adapter can present to this compositor")?
    };
    probe.destroy();
    if adapter.get_info().device_type == wgpu::DeviceType::Cpu && std::env::var_os("SOUNDWAV_ALLOW_CPU").is_none() {
        return Err(format!("only a software renderer ({}) can present here, refusing to burn cpu (set SOUNDWAV_ALLOW_CPU=1 to force)", adapter.get_info().name));
    }
    if std::env::var_os("SOUNDWAV_DEBUG").is_some() {
        eprintln!("soundwav: adapter {:?}", adapter.get_info());
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("soundwav"), ..Default::default() }))
        .map_err(|e| format!("gpu device: {e}"))?;
    let colors = palette(&cfg);
    let wcolors = wave_palette(&cfg);
    let mut app = App {
        conn: conn.clone(),
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        gpu: Gpu { instance, adapter, device, queue, pipe: None },
        shm,
        pool: None,
        reserves: Vec::new(),
        surfs: Vec::new(),
        cfg,
        colors,
        audio,
        scene,
        anim: Anim { output: None, rect: Rect::default(), caps: [0.0; 4], alpha: 0.0, cleared: true },
        last_tick: Instant::now(),
        dbg_at: Instant::now(),
        bars: Vec::new(),
        insts: Vec::new(),
        wcolors,
        env: Env::default(),
        samples: Vec::new(),
        verts: Vec::new(),
        strips: Vec::new(),
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

fn layer_kind(s: &str) -> Layer {
    match s.trim().to_ascii_lowercase().as_str() {
        "bottom" => Layer::Bottom,
        "overlay" => Layer::Overlay,
        "background" => Layer::Background,
        _ => Layer::Top,
    }
}
