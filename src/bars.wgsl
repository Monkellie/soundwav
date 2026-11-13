struct U {
    screen: vec2<f32>,
    glow: f32,
    alpha: f32,
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    over_alpha: f32,
    p0: f32,
    p1: f32,
    p2: f32,
};

@group(0) @binding(0) var<uniform> u: U;

struct Inst {
    @location(0) origin: vec2<f32>,
    @location(1) dir: vec2<f32>,
    @location(2) len: f32,
    @location(3) cap: f32,
    @location(4) width: f32,
    @location(5) over: f32,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) len: f32,
    @location(2) cap: f32,
    @location(3) hw: f32,
    @location(4) over: f32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let k = corners[vi];
    let g = u.glow * 2.0;
    let hw = i.width * 0.5;
    let a = mix(-g, i.len + g, k.x);
    let c = mix(-(hw + g), hw + g, k.y);
    let perp = vec2<f32>(-i.dir.y, i.dir.x);
    let p = i.origin + i.dir * a + perp * c;
    var o: VOut;
    o.pos = vec4<f32>(p.x / u.screen.x * 2.0 - 1.0, 1.0 - p.y / u.screen.y * 2.0, 0.0, 1.0);
    o.local = vec2<f32>(a, c);
    o.len = i.len;
    o.cap = i.cap;
    o.hw = hw;
    o.over = i.over;
    return o;
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
    let along = clamp(v.local.x, v.hw, max(v.len - v.hw, v.hw));
    let d = length(vec2<f32>(v.local.x - along, v.local.y)) - v.hw;
    let core = clamp(0.5 - d, 0.0, 1.0);
    let glow = select(0.0, exp(-max(d, 0.0) / max(u.glow, 0.001)) * 0.35, d > 0.0);
    let t = clamp(v.local.x / max(v.cap, 1.0), 0.0, 1.0);
    var col = mix(u.c0.rgb, u.c1.rgb, clamp(t * 2.0, 0.0, 1.0));
    col = mix(col, u.c2.rgb, clamp(t * 2.0 - 1.0, 0.0, 1.0));
    let a = max(core, glow) * u.alpha * mix(1.0, u.over_alpha, v.over);
    return vec4<f32>(col * a, a);
}
