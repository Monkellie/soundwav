use bytemuck::{Pod, Zeroable};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use std::ptr::NonNull;
use wayland_client::{protocol::wl_surface, Connection, Proxy};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub screen: [f32; 2],
    pub glow: f32,
    pub alpha: f32,
    pub c0: [f32; 4],
    pub c1: [f32; 4],
    pub c2: [f32; 4],
    pub over_alpha: f32,
    pub pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Inst {
    pub origin: [f32; 2],
    pub dir: [f32; 2],
    pub len: f32,
    pub cap: f32,
    pub width: f32,
    pub over: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct WUniforms {
    pub screen: [f32; 2],
    pub glow: f32,
    pub alpha: f32,
    pub c0: [f32; 4],
    pub c1: [f32; 4],
    pub c2: [f32; 4],
    pub over_alpha: f32,
    pub line: f32,
    pub fill: f32,
    pub pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct WVert {
    pub pos: [f32; 2],
    pub v: f32,
    pub kind: f32,
    pub band: f32,
    pub over: f32,
}

#[derive(Default)]
pub struct Batch {
    pub insts: Vec<Inst>,
    pub verts: Vec<WVert>,
    pub strips: Vec<(u32, u32)>,
}

impl Batch {
    pub fn clear(&mut self) {
        self.insts.clear();
        self.verts.clear();
        self.strips.clear();
    }
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

pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipe: Option<Pipe>,
}

pub fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn wgpu_surface(instance: &wgpu::Instance, conn: &Connection, surface: &wl_surface::WlSurface) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
    let display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(NonNull::new(conn.backend().display_ptr() as *mut _).unwrap()));
    let window = RawWindowHandle::Wayland(WaylandWindowHandle::new(NonNull::new(surface.id().as_ptr() as *mut _).unwrap()));
    unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle { raw_display_handle: Some(display), raw_window_handle: window }) }
}

fn uniform_buffer(d: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    d.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn vertex_buffer(d: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    d.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn pipeline(d: &wgpu::Device, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, label: &str, buffers: &[Option<wgpu::VertexBufferLayout>], topology: wgpu::PrimitiveTopology, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState { module, entry_point: Some("vs"), compilation_options: Default::default(), buffers },
        primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
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
    })
}

fn make_pipe(d: &wgpu::Device, format: wgpu::TextureFormat) -> Pipe {
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bars"),
        source: wgpu::ShaderSource::Wgsl(include_str!("bars.wgsl").into()),
    });
    let wmodule = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("waves"),
        source: wgpu::ShaderSource::Wgsl(include_str!("waves.wgsl").into()),
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
    let wattrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Float32, 4 => Float32];
    let wbuffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<WVert>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wattrs,
    })];
    let bind_to = |label: &str, buf: &wgpu::Buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
        })
    };
    let ubuf = uniform_buffer(d, "u", std::mem::size_of::<Uniforms>());
    let wubuf = uniform_buffer(d, "wu", std::mem::size_of::<WUniforms>());
    let (icap, vcap) = (4096, 32768);
    Pipe {
        format,
        pipeline: pipeline(d, &layout, &module, "bars", &buffers, wgpu::PrimitiveTopology::TriangleList, format),
        bind: bind_to("u", &ubuf),
        ubuf,
        ibuf: vertex_buffer(d, "inst", icap * std::mem::size_of::<Inst>()),
        icap,
        wpipeline: pipeline(d, &layout, &wmodule, "waves", &wbuffers, wgpu::PrimitiveTopology::TriangleStrip, format),
        wbind: bind_to("wu", &wubuf),
        wubuf,
        vbuf: vertex_buffer(d, "wverts", vcap * std::mem::size_of::<WVert>()),
        vcap,
    }
}

impl Gpu {
    pub fn new(conn: &Connection, probe: &wl_surface::WlSurface) -> Result<Self, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(desc);
        let adapter = {
            let psurf = wgpu_surface(&instance, conn, probe).map_err(|e| format!("gpu surface: {e}"))?;
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
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu && std::env::var_os("SOUNDWAV_ALLOW_CPU").is_none() {
            return Err(format!("only a software renderer ({}) can present here, refusing to burn cpu (set SOUNDWAV_ALLOW_CPU=1 to force)", adapter.get_info().name));
        }
        if std::env::var_os("SOUNDWAV_DEBUG").is_some() {
            eprintln!("soundwav: adapter {:?}", adapter.get_info());
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("soundwav"), ..Default::default() }))
            .map_err(|e| format!("gpu device: {e}"))?;
        Ok(Self { instance, adapter, device, queue, pipe: None })
    }

    pub fn surface(&self, conn: &Connection, surface: &wl_surface::WlSurface) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
        wgpu_surface(&self.instance, conn, surface)
    }

    pub fn ready_for(&self, format: Option<wgpu::TextureFormat>) -> bool {
        format.is_some() && format == self.pipe.as_ref().map(|p| p.format)
    }

    pub fn configure(&mut self, gsurf: &wgpu::Surface<'static>, width: u32, height: u32, name: &str) -> Option<wgpu::TextureFormat> {
        let caps = gsurf.get_capabilities(&self.adapter);
        let Some(mut conf) = gsurf.get_default_config(&self.adapter, width.max(1), height.max(1)) else {
            eprintln!("soundwav: surface on {name} not supported by the adapter");
            return None;
        };
        let alpha = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Inherit, wgpu::CompositeAlphaMode::PostMultiplied]
            .into_iter()
            .find(|m| caps.alpha_modes.contains(m));
        let Some(alpha) = alpha else {
            eprintln!("soundwav: no transparent alpha mode on {name} ({:?}), not drawing", caps.alpha_modes);
            return None;
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
        gsurf.configure(&self.device, &conf);
        if self.pipe.as_ref().map(|p| p.format) != Some(format) {
            self.pipe = Some(make_pipe(&self.device, format));
        }
        Some(format)
    }

    pub fn render(&mut self, tex: &wgpu::SurfaceTexture, u: &Uniforms, wu: &WUniforms, batch: &Batch) {
        let Some(pipe) = self.pipe.as_mut() else { return };
        let need = batch.insts.len();
        let vneed = batch.verts.len();
        if vneed > pipe.vcap {
            pipe.vcap = vneed.next_power_of_two();
            pipe.vbuf = vertex_buffer(&self.device, "wverts", pipe.vcap * std::mem::size_of::<WVert>());
        }
        if vneed > 0 {
            self.queue.write_buffer(&pipe.wubuf, 0, bytemuck::bytes_of(wu));
            self.queue.write_buffer(&pipe.vbuf, 0, bytemuck::cast_slice(&batch.verts));
        }
        if need > pipe.icap {
            pipe.icap = need.next_power_of_two();
            pipe.ibuf = vertex_buffer(&self.device, "inst", pipe.icap * std::mem::size_of::<Inst>());
        }
        self.queue.write_buffer(&pipe.ubuf, 0, bytemuck::bytes_of(u));
        if need > 0 {
            self.queue.write_buffer(&pipe.ibuf, 0, bytemuck::cast_slice(&batch.insts));
        }
        let view = tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("soundwav") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("soundwav"),
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
                for (start, count) in &batch.strips {
                    pass.draw(*start..start + count, 0..1);
                }
            }
        }
        self.queue.submit(Some(enc.finish()));
    }
}
