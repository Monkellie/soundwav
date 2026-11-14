use std::f32::consts::FRAC_PI_2;

pub const TOP: usize = 0;
pub const RIGHT: usize = 1;
pub const BOTTOM: usize = 2;
pub const LEFT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl From<zoo_ipc::zoolander::Area> for Rect {
    fn from(a: zoo_ipc::zoolander::Area) -> Self {
        Self::new(a.x, a.y, a.width, a.height)
    }
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn expand(&self, d: f32) -> Self {
        Self::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    pub fn lerp(&self, o: &Rect, k: f32) -> Self {
        Self::new(
            self.x + (o.x - self.x) * k,
            self.y + (o.y - self.y) * k,
            self.w + (o.w - self.w) * k,
            self.h + (o.h - self.h) * k,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub border: f32,
    pub min_open: f32,
    pub max_out: f32,
    pub max_over: f32,
    pub max_over_frac: f32,
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

pub fn free_space(target: Rect, area: Rect, obstacles: &[Rect], border: f32) -> ([f32; 4], [bool; 4]) {
    let o = target.expand(border);
    let slack = 2.0 * border + 1.0;
    let mut lim = [area.y, area.right(), area.bottom(), area.x];
    let mut hit = [false; 4];
    for ob in obstacles {
        let b = ob.expand(border);
        let ox = overlap(o.x, o.right(), b.x, b.right()) > 8.0;
        let oy = overlap(o.y, o.bottom(), b.y, b.bottom()) > 8.0;
        if ox && b.bottom() <= o.y + slack && b.bottom() >= lim[TOP] {
            lim[TOP] = b.bottom();
            hit[TOP] = true;
        }
        if ox && b.y >= o.bottom() - slack && b.y <= lim[BOTTOM] {
            lim[BOTTOM] = b.y;
            hit[BOTTOM] = true;
        }
        if oy && b.right() <= o.x + slack && b.right() >= lim[LEFT] {
            lim[LEFT] = b.right();
            hit[LEFT] = true;
        }
        if oy && b.x >= o.right() - slack && b.x <= lim[RIGHT] {
            lim[RIGHT] = b.x;
            hit[RIGHT] = true;
        }
    }
    (
        [
            (o.y - lim[TOP]).max(0.0),
            (lim[RIGHT] - o.right()).max(0.0),
            (lim[BOTTOM] - o.bottom()).max(0.0),
            (o.x - lim[LEFT]).max(0.0),
        ],
        hit,
    )
}

pub const MIN_EDGE: f32 = 6.0;

pub fn tab_strip_cap(free: [f32; 4], width: f32, offset: f32) -> Option<(usize, f32)> {
    let need = width + 8.0;
    let gap = |f: f32| ((f - width) * 0.5).clamp(4.0, offset.max(4.0));
    let side = if free[LEFT] >= need {
        LEFT
    } else if free[RIGHT] >= need {
        RIGHT
    } else {
        return None;
    };
    let room = gap(free[side]) - 2.0;
    Some((side, if room >= MIN_EDGE { room } else { 0.0 }))
}

pub fn side_caps(target: Rect, area: Rect, obstacles: &[Rect], fullscreen: bool, p: &Params) -> [f32; 4] {
    if fullscreen {
        return [0.0; 4];
    }
    let over = p.max_over.min(p.max_over_frac * target.w.min(target.h)).max(0.0);
    let (free, hit) = free_space(target, area, obstacles, p.border);
    let mut caps = [0.0; 4];
    for i in 0..4 {
        let room = free[i] - 4.0;
        caps[i] = if free[i] >= p.min_open {
            p.max_out.min(room)
        } else if hit[i] {
            -over
        } else if room >= MIN_EDGE {
            room
        } else {
            0.0
        };
    }
    caps
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub origin: [f32; 2],
    pub dir: [f32; 2],
    pub len: f32,
    pub cap: f32,
    pub over: bool,
}

pub fn sample(v: &[f32], u: f32) -> f32 {
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return v[0];
    }
    let f = u.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = f.floor() as isize;
    let t = f - i as f32;
    let g = |k: isize| v[k.clamp(0, n as isize - 1) as usize];
    let (p0, p1, p2, p3) = (g(i - 1), g(i), g(i + 1), g(i + 2));
    let r = 0.5
        * ((2.0 * p1)
            + (-p0 + p2) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t);
    r.clamp(0.0, 1.0)
}

struct PathPoint {
    pos: [f32; 2],
    normal: [f32; 2],
}

fn half_path(r: Rect, radius: f32, s: f32) -> PathPoint {
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let cx = r.x + r.w * 0.5;
    let l1 = (r.w * 0.5 - rad).max(0.0);
    let arc = rad * FRAC_PI_2;
    let l3 = (r.h - 2.0 * rad).max(0.0);
    let mut s = s;
    if s <= l1 {
        return PathPoint { pos: [cx - s, r.bottom()], normal: [0.0, 1.0] };
    }
    s -= l1;
    if s <= arc && arc > 0.0 {
        let a = s / rad;
        let n = [-a.sin(), a.cos()];
        let c = [r.x + rad, r.bottom() - rad];
        return PathPoint { pos: [c[0] + rad * n[0], c[1] + rad * n[1]], normal: n };
    }
    s -= arc;
    if s <= l3 {
        return PathPoint { pos: [r.x, r.bottom() - rad - s], normal: [-1.0, 0.0] };
    }
    s -= l3;
    if s <= arc && arc > 0.0 {
        let a = s / rad;
        let n = [-a.cos(), -a.sin()];
        let c = [r.x + rad, r.y + rad];
        return PathPoint { pos: [c[0] + rad * n[0], c[1] + rad * n[1]], normal: n };
    }
    s -= arc;
    PathPoint { pos: [r.x + rad + s.min(l1), r.y], normal: [0.0, -1.0] }
}

pub fn half_length(r: Rect, radius: f32) -> f32 {
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    2.0 * (r.w * 0.5 - rad).max(0.0) + (r.h - 2.0 * rad).max(0.0) + 2.0 * rad * FRAC_PI_2
}

pub fn ring_length(r: Rect, radius: f32) -> f32 {
    2.0 * half_length(r, radius)
}

#[derive(Clone, Copy, Debug)]
pub struct RingPt {
    pub pos: [f32; 2],
    pub normal: [f32; 2],
}

pub fn ring_point(r: Rect, radius: f32, s: f32) -> RingPt {
    let h = half_length(r, radius);
    let p = 2.0 * h;
    if p <= 0.0 {
        return RingPt { pos: [r.x, r.y], normal: [0.0, -1.0] };
    }
    let s = s.rem_euclid(p);
    if s <= h {
        let q = half_path(r, radius, s);
        RingPt { pos: q.pos, normal: q.normal }
    } else {
        let q = half_path(r, radius, p - s);
        RingPt { pos: [2.0 * r.x + r.w - q.pos[0], q.pos[1]], normal: [-q.normal[0], q.normal[1]] }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn cap_at(r: Rect, radius: f32, caps: [f32; 4], blend: f32, s: f32) -> (f32, f32) {
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let l1 = (r.w * 0.5 - rad).max(0.0);
    let arc = rad * FRAC_PI_2;
    let l3 = (r.h - 2.0 * rad).max(0.0);
    let h = 2.0 * l1 + l3 + 2.0 * arc;
    let p = 2.0 * h;
    if p <= 0.0 {
        return (0.0, 0.0);
    }
    let s = s.rem_euclid(p);
    let corners = [
        (l1 + arc * 0.5, BOTTOM, LEFT),
        (l1 + arc + l3 + arc * 0.5, LEFT, TOP),
        (h + l1 + arc * 0.5, TOP, RIGHT),
        (h + l1 + arc + l3 + arc * 0.5, RIGHT, BOTTOM),
    ];
    let b = blend.min(2.0 * l1 + arc).min(l3 + arc).max(0.0);
    let mut best = (f32::MAX, 0.0f32, BOTTOM, LEFT);
    for (c, a, z) in corners {
        let mut d = s - c;
        if d > h {
            d -= p;
        } else if d < -h {
            d += p;
        }
        if d.abs() < best.0.abs() {
            best = (d, d, a, z);
        }
    }
    let (_, d, a, z) = best;
    let t = smoothstep(-b * 0.5, b * 0.5, d);
    let mag = caps[a].abs() + (caps[z].abs() - caps[a].abs()) * t;
    let ov = |c: f32| if c < 0.0 { 1.0 } else { 0.0 };
    let over = ov(caps[a]) + (ov(caps[z]) - ov(caps[a])) * t;
    (mag, over)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WaveIn {
    pub env_l: [f32; 3],
    pub env_r: [f32; 3],
    pub phase: [f32; 3],
    pub floor: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct WaveSample {
    pub base: [f32; 2],
    pub normal: [f32; 2],
    pub over: f32,
    pub off: [f32; 3],
}

pub const WAVE_SCALE: [f32; 3] = [1.0, 0.8, 0.6];
const WAVE_K: [f32; 3] = [3.0, 7.0, 17.0];
const WAVE_LEN: [f32; 3] = [900.0, 360.0, 140.0];

pub fn wave_shape(k: f32, u: f32, phase: f32) -> f32 {
    use std::f32::consts::TAU;
    (0.5 + 0.32 * (TAU * k * u - phase).sin() + 0.18 * (TAU * (2.0 * k + 1.0) * u + 1.7 * phase).sin()).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
pub fn waves(rect: Rect, radius: f32, caps: [f32; 4], border: f32, blend: f32, step: f32, w: &WaveIn, out: &mut Vec<WaveSample>) {
    out.clear();
    if rect.w < 4.0 || rect.h < 4.0 {
        return;
    }
    let p = ring_length(rect, radius);
    let n = (p / step.max(1.0)).ceil() as usize;
    if n < 8 {
        return;
    }
    let k: [f32; 3] = std::array::from_fn(|b| WAVE_K[b].max((p / WAVE_LEN[b]).round()));
    let cx = rect.x + rect.w * 0.5;
    for i in 0..n {
        let s = i as f32 * p / n as f32;
        let u = s / p;
        let q = ring_point(rect, radius, s);
        let (mag, over) = cap_at(rect, radius, caps, blend, s);
        let wr = smoothstep(cx - 80.0, cx + 80.0, q.pos[0]);
        let off = std::array::from_fn(|b| {
            if mag <= 0.0 {
                return 0.0;
            }
            let env = (w.env_l[b] + (w.env_r[b] - w.env_l[b]) * wr).clamp(0.0, 1.0);
            let v = mag * WAVE_SCALE[b] * env * wave_shape(k[b], u, w.phase[b]);
            v.max(w.floor.min(mag)).min(mag)
        });
        out.push(WaveSample { base: [q.pos[0] + q.normal[0] * border, q.pos[1] + q.normal[1] * border], normal: q.normal, over, off });
    }
}

#[allow(clippy::too_many_arguments)]
pub fn bars(
    rect: Rect,
    radius: f32,
    caps: [f32; 4],
    pitch: f32,
    border: f32,
    left: &[f32],
    right: &[f32],
    out: &mut Vec<Bar>,
) {
    out.clear();
    if rect.w < 4.0 || rect.h < 4.0 {
        return;
    }
    let ph = half_length(rect, radius);
    let n = (ph / pitch.max(1.0)).floor() as usize;
    if n == 0 {
        return;
    }
    let cx2 = 2.0 * rect.x + rect.w;
    for i in 0..n {
        let s = (i as f32 + 0.5) * ph / n as f32;
        let u = s / ph;
        let p = half_path(rect, radius, s);
        for (mirror, spec) in [(false, left), (true, right)] {
            let (mag, ov) = cap_at(rect, radius, caps, 120.0, if mirror { 2.0 * ph - s } else { s });
            let c = if ov > 0.5 { -mag } else { mag };
            let level = sample(spec, u);
            let len = level * c.abs();
            if len < 0.75 {
                continue;
            }
            let mut n = p.normal;
            let mut pos = p.pos;
            if mirror {
                n[0] = -n[0];
                pos[0] = cx2 - pos[0];
            }
            let origin = [pos[0] + n[0] * border, pos[1] + n[1] * border];
            out.push(Bar { origin, dir: n, len, cap: c.abs(), over: c < 0.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_side_shrinks() {
        assert_eq!(tab_strip_cap([90.0, 55.0, 55.0, 55.0], 30.0, 12.0), Some((LEFT, 10.0)));
        assert_eq!(tab_strip_cap([90.0, 55.0, 55.0, 0.0], 30.0, 12.0), Some((RIGHT, 10.0)));
        assert_eq!(tab_strip_cap([90.0, 5.0, 55.0, 5.0], 30.0, 12.0), None);
        assert_eq!(tab_strip_cap([90.0, 0.0, 55.0, 40.0], 30.0, 12.0), Some((LEFT, 0.0)));
    }

    fn params() -> Params {
        Params { border: 5.0, min_open: 60.0, max_out: 220.0, max_over: 56.0, max_over_frac: 0.12 }
    }

    fn area() -> Rect {
        Rect::new(0.0, 38.0, 2560.0, 1318.0)
    }

    #[test]
    fn floating_alone_all_outward() {
        let k = Rect::new(800.0, 400.0, 900.0, 500.0);
        let c = side_caps(k, area(), &[], false, &params());
        assert!(c.iter().all(|v| *v > 0.0), "{c:?}");
        assert_eq!(c[TOP], 220.0);
    }

    #[test]
    fn master_left_with_stack_right() {
        let k = Rect::new(15.0, 86.0, 1388.0, 1255.0);
        let other = Rect::new(1408.0, 86.0, 1137.0, 1255.0);
        let c = side_caps(k, area(), &[other], false, &params());
        assert_eq!(c[RIGHT], -56.0, "{c:?}");
        assert_eq!(c[LEFT], 6.0, "left gap is 10px: {c:?}");
        assert_eq!(c[TOP], 39.0, "43px under the bar: {c:?}");
    }

    #[test]
    fn kitty_in_stack_bottom() {
        let k = Rect::new(1408.0, 720.0, 1137.0, 621.0);
        let master = Rect::new(15.0, 86.0, 1388.0, 1255.0);
        let above = Rect::new(1408.0, 86.0, 1137.0, 629.0);
        let c = side_caps(k, area(), &[master, above], false, &params());
        assert!(c[LEFT] < 0.0 && c[TOP] < 0.0, "{c:?}");
        assert!(c[RIGHT] >= 0.0 && c[BOTTOM] >= 0.0, "{c:?}");
    }

    #[test]
    fn open_space_beside_small_tile() {
        let k = Rect::new(15.0, 86.0, 1000.0, 700.0);
        let c = side_caps(k, area(), &[], false, &params());
        assert!(c[RIGHT] > 60.0 && c[BOTTOM] > 60.0, "{c:?}");
        assert!(c[LEFT] >= 0.0 && c[TOP] >= 0.0, "screen edges never go over: {c:?}");
    }

    #[test]
    fn tiny_edge_gap_draws_nothing() {
        let k = Rect::new(3.0, 86.0, 1000.0, 700.0);
        let c = side_caps(k, area(), &[], false, &params());
        assert_eq!(c[LEFT], 0.0, "{c:?}");
    }

    #[test]
    fn fullscreen_draws_nothing() {
        let k = Rect::new(0.0, 0.0, 2560.0, 1440.0);
        let c = side_caps(k, area(), &[], true, &params());
        assert!(c.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn over_cap_scales_with_small_windows() {
        let k = Rect::new(500.0, 500.0, 400.0, 300.0);
        let n = Rect::new(910.0, 500.0, 400.0, 300.0);
        let c = side_caps(k, area(), &[n], false, &params());
        assert!((c[RIGHT] + 36.0).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn far_neighbour_is_open_space() {
        let k = Rect::new(500.0, 500.0, 400.0, 300.0);
        let n = Rect::new(1200.0, 500.0, 400.0, 300.0);
        let c = side_caps(k, area(), &[n], false, &params());
        assert_eq!(c[RIGHT], 220.0, "{c:?}");
    }

    #[test]
    fn bars_always_outward() {
        let r = Rect::new(100.0, 100.0, 400.0, 300.0);
        let caps = [50.0, -30.0, 50.0, 50.0];
        let spec = vec![1.0; 16];
        let mut out = Vec::new();
        bars(r, 10.0, caps, 8.0, 5.0, &spec, &spec, &mut out);
        assert!(!out.is_empty());
        let right_side: Vec<_> = out.iter().filter(|b| b.origin[0] > 499.0 && b.dir[0].abs() > 0.99999 && b.origin[1] > 180.0 && b.origin[1] < 320.0).collect();
        assert!(!right_side.is_empty());
        assert!(right_side.iter().all(|b| b.over && b.dir[0] > 0.0 && (b.origin[0] - 505.0).abs() < 1e-3 && (b.len - 30.0).abs() < 1e-3));
        let left_side: Vec<_> = out.iter().filter(|b| b.origin[0] < 100.0 && b.dir[0].abs() > 0.99999).collect();
        assert!(!left_side.is_empty());
        assert!(left_side.iter().all(|b| !b.over && b.dir[0] < 0.0 && (b.origin[0] - 95.0).abs() < 1e-3));
        let inside = out.iter().any(|b| {
            let e = [b.origin[0] + b.dir[0] * b.len, b.origin[1] + b.dir[1] * b.len];
            e[0] > 101.0 && e[0] < 499.0 && e[1] > 101.0 && e[1] < 399.0
        });
        assert!(!inside);
    }

    #[test]
    fn ring_closes_and_normals_continuous() {
        let r = Rect::new(100.0, 100.0, 900.0, 500.0);
        let p = ring_length(r, 14.0);
        let a = ring_point(r, 14.0, 0.0);
        let b = ring_point(r, 14.0, p - 0.001);
        assert!((a.pos[0] - b.pos[0]).abs() < 0.01 && (a.pos[1] - b.pos[1]).abs() < 0.01);
        let mut prev = ring_point(r, 14.0, 0.0);
        let mut s = 3.0;
        while s < p {
            let q = ring_point(r, 14.0, s);
            let dot = prev.normal[0] * q.normal[0] + prev.normal[1] * q.normal[1];
            assert!(dot > 15f32.to_radians().cos(), "jump at {s}");
            let d = ((q.pos[0] - prev.pos[0]).powi(2) + (q.pos[1] - prev.pos[1]).powi(2)).sqrt();
            assert!(d < 3.5, "gap {d} at {s}");
            prev = q;
            s += 3.0;
        }
    }

    #[test]
    fn caps_blend_smoothly() {
        let r = Rect::new(0.0, 0.0, 900.0, 500.0);
        let caps = [10.0, 91.0, -56.0, 91.0];
        let p = ring_length(r, 14.0);
        let mut prev = cap_at(r, 14.0, caps, 120.0, 0.0);
        assert!((prev.0 - 56.0).abs() < 0.01 && prev.1 == 1.0);
        let mut s = 1.0;
        while s < p {
            let c = cap_at(r, 14.0, caps, 120.0, s);
            assert!((c.0 - prev.0).abs() < 3.0, "cap jump at {s}: {prev:?} -> {c:?}");
            prev = c;
            s += 1.0;
        }
        let top = cap_at(r, 14.0, caps, 120.0, p * 0.5);
        assert!((top.0 - 10.0).abs() < 0.01);
    }

    #[test]
    fn waves_stay_inside_caps() {
        let r = Rect::new(0.0, 0.0, 900.0, 500.0);
        let caps = [10.0, 91.0, -56.0, 91.0];
        let w = WaveIn { env_l: [1.0; 3], env_r: [1.0; 3], phase: [0.3, 1.1, 2.0], floor: 2.0 };
        let mut out = Vec::new();
        waves(r, 14.0, caps, 5.0, 120.0, 3.0, &w, &mut out);
        assert!(out.len() > 900);
        for (i, s) in out.iter().enumerate() {
            let (mag, _) = cap_at(r, 14.0, caps, 120.0, i as f32 * ring_length(r, 14.0) / out.len() as f32);
            for o in s.off {
                assert!(o <= mag + 1e-3 && o >= 0.0);
            }
        }
    }

    #[test]
    fn sample_endpoints() {
        let v = [0.0, 0.5, 1.0];
        assert!((sample(&v, 0.0) - 0.0).abs() < 1e-6);
        assert!((sample(&v, 1.0) - 1.0).abs() < 1e-6);
        assert!((sample(&v, 0.5) - 0.5).abs() < 1e-6);
    }
}
