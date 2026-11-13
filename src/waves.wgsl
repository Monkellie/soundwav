struct WU {
    screen: vec2<f32>,
    glow: f32,
    alpha: f32,
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    over_alpha: f32,
    line: f32,
    fill: f32,
    pad: f32,
};

@group(0) @binding(0) var<uniform> u: WU;

struct VIn {
    @location(0) pos: vec2<f32>,
    @location(1) v: f32,
    @location(2) kind: f32,
    @location(3) band: f32,
    @location(4) over: f32,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) v: f32,
    @location(1) kind: f32,
    @location(2) band: f32,
    @location(3) over: f32,
};

@vertex
fn vs(i: VIn) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(i.pos.x / u.screen.x * 2.0 - 1.0, 1.0 - i.pos.y / u.screen.y * 2.0, 0.0, 1.0);
    o.v = i.v;
    o.kind = i.kind;
    o.band = i.band;
    o.over = i.over;
    return o;
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
    var col = u.c0.rgb;
    if (v.band > 1.5) {
        col = u.c2.rgb;
    } else if (v.band > 0.5) {
        col = u.c1.rgb;
    }
    var a = 0.0;
    if (v.kind < 0.5) {
        let t = clamp(v.v, 0.0, 1.0);
        a = u.fill * mix(0.08, 1.0, t * t);
    } else {
        let d = abs(v.v) - u.line * 0.5;
        let core = clamp(0.5 - d, 0.0, 1.0);
        let glow = exp(-max(d, 0.0) / max(u.glow, 0.001)) * 0.55;
        a = max(core, glow);
        col = mix(col, vec3<f32>(1.0, 1.0, 1.0), core * 0.25);
    }
    a = a * u.alpha * mix(1.0, u.over_alpha, v.over);
    return vec4<f32>(col * a, a);
}
