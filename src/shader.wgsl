// Derivatives sit in branches on per-instance values, uniform within each
// primitive; Chrome's compiler cannot prove that and rejects the shader.
diagnostic(off, derivative_uniformity);

struct Globals {
    viewport: vec2<f32>,
}

@group(0) @binding(0) var<uniform> g: Globals;

// Outline glyphs: each curve is two vec4s, (p0, p1) and (p2, _), em units, y up.
@group(1) @binding(0) var<storage, read> curves: array<vec4<f32>>;
// Per glyph: [start, count] per horizontal band, then per vertical band; then curve indices.
@group(1) @binding(1) var<storage, read> bands: array<u32>;

// Bitmap sampled by image instances (premultiplied); a 1x1 dummy otherwise.
@group(2) @binding(0) var image: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    // shape: radius, border width, blur, kind 0
    // glyph: band header, horizontal bands, vertical bands, kind 1
    // image: radius, _, _, kind 2
    // uv: glyph em-space quad corners; shadow: hole (x0, y0, x1, y1) left undrawn;
    // image: texture coordinates of the rect's corners
    @location(4) params: vec4<f32>,
    @location(5) clip: vec4<f32>,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world: vec2<f32>,
    @location(3) @interpolate(flat) rect: vec4<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
    @location(5) @interpolate(flat) border_color: vec4<f32>,
    @location(6) @interpolate(flat) params: vec4<f32>,
    @location(7) @interpolate(flat) clip: vec4<f32>,
    @location(8) @interpolate(flat) uv_rect: vec4<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let c = corners[vi % 6u];
    let strip = vi / 6u;
    // Shapes grow by blur + 1px and glyphs by 1px so AA isn't cut off.
    var pad = 1.0;
    if inst.params.w < 0.5 {
        pad = inst.params.z + 1.0;
    }
    var lo = inst.rect.xy - pad;
    var hi = inst.rect.xy + inst.rect.zw + pad;
    // 24 vertices: one quad, or for a shadow with a hole four strips around
    // it. Unused strips collapse to nothing.
    let hole = inst.uv;
    if inst.params.w < 0.5 && hole.z > hole.x && hole.w > hole.y {
        let h0 = clamp(hole.xy, lo, hi);
        let h1 = clamp(hole.zw, lo, hi);
        switch strip {
            case 0u: { hi.y = h0.y; }
            case 1u: { lo.y = h1.y; }
            case 2u: { lo.y = h0.y; hi.y = h1.y; hi.x = h0.x; }
            default: { lo.y = h0.y; hi.y = h1.y; lo.x = h1.x; }
        }
    } else if strip > 0u {
        hi = lo;
    }
    let world = mix(lo, hi, c);

    var out: VsOut;
    out.pos = vec4(world / g.viewport * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    out.local = world - (inst.rect.xy + inst.rect.zw * 0.5);
    // Extrapolates past the uv rect into the padding.
    out.uv = inst.uv.xy + (world - inst.rect.xy) / inst.rect.zw * (inst.uv.zw - inst.uv.xy);
    out.uv_rect = inst.uv;
    out.world = world;
    out.rect = inst.rect;
    out.color = inst.color;
    out.border_color = inst.border_color;
    out.params = inst.params;
    out.clip = inst.clip;
    return out;
}

fn sd_round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + r;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Antialiased winding along a ray toward +x from `p`, using the band containing
// `p`. Swapped axes give the vertical ray. Curves are monotonic, so a curve
// crosses the ray's line iff its endpoints straddle it (half-open, so shared
// endpoints count once). Returns (coverage, weight); weight is how close the
// nearest crossing is, i.e. how well this ray resolves the edge at `p`.
fn ray_coverage(p_in: vec2<f32>, header: u32, band_count: f32, lo: f32, hi: f32, px: f32, swap: bool) -> vec2<f32> {
    var p = p_in;
    if swap {
        p = p.yx;
    }
    let band = u32(clamp(floor((p.y - lo) / (hi - lo) * band_count), 0.0, band_count - 1.0));
    let start = bands[header + 2u * band];
    let count = bands[header + 2u * band + 1u];
    var sum = 0.0;
    var weight = 0.0;
    for (var i = 0u; i < count; i++) {
        let ci = bands[start + i];
        let a = curves[2u * ci];
        let b = curves[2u * ci + 1u];
        var p0 = a.xy;
        var p1 = a.zw;
        var p2 = b.xy;
        if swap {
            p0 = p0.yx;
            p1 = p1.yx;
            p2 = p2.yx;
        }
        // Sorted by reach along the ray: everything from here on is behind the pixel.
        if max(p0.x, p2.x) < p.x - 0.5 * px {
            break;
        }
        if (p0.y <= p.y) == (p2.y <= p.y) {
            continue;
        }
        let qa = p0.y - 2.0 * p1.y + p2.y;
        let qb = 2.0 * (p1.y - p0.y);
        let qc = p0.y - p.y;
        // Stable roots q / qa and qc / q: the second stays accurate as the
        // curve approaches a line (qa -> 0), with no threshold to tune.
        let d = sqrt(max(qb * qb - 4.0 * qa * qc, 0.0));
        let q = -0.5 * (qb + select(-d, d, qb >= 0.0));
        let t2 = qc / q;
        let t = select(q / qa, t2, t2 >= -1e-4 && t2 <= 1.0 + 1e-4);
        let x = mix(mix(p0.x, p1.x, t), mix(p1.x, p2.x, t), t);
        let r = (x - p.x) / px;
        sum += sign(p2.y - p0.y) * clamp(r + 0.5, 0.0, 1.0);
        weight = max(weight, clamp(1.0 - abs(r) * 2.0, 0.0, 1.0));
    }
    return vec2(min(abs(sum), 1.0), weight);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    if in.world.x < in.clip.x || in.world.y < in.clip.y
        || in.world.x > in.clip.x + in.clip.z || in.world.y > in.clip.y + in.clip.w {
        discard;
    }

    if in.params.w > 1.5 {
        // Explicit level: no mips, and sampling isn't in uniform control flow.
        let s = textureSampleLevel(image, image_sampler, in.uv, 0.0);
        let radius = min(in.params.x, min(in.rect.z, in.rect.w) * 0.5);
        let d = sd_round_box(in.local, in.rect.zw * 0.5, radius);
        let aa = max(length(vec2(dpdx(d), dpdy(d))), 1e-4);
        let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
        return s * vec4(in.color.rgb * in.color.a, in.color.a) * coverage;
    }
    var color: vec4<f32>;
    if in.params.w > 0.5 {
        let header = u32(in.params.x);
        let px = fwidth(in.uv);
        let r = in.uv_rect;
        // uv_rect is (x0, y1, x1, y0): em bbox with y up.
        let h = ray_coverage(in.uv, header, in.params.y, r.w, r.y, max(px.x, 1e-6), false);
        let v = ray_coverage(in.uv, header + 2u * u32(in.params.y), in.params.z, r.x, r.z, max(px.y, 1e-6), true);
        // Trust the ray that crosses an edge near the pixel; the other one only
        // sees a hard inside/outside step there. Same weighting as Slug.
        let cov = max((h.x * h.y + v.x * v.y) / max(h.y + v.y, 1e-5), min(h.x, v.x));
        color = vec4(in.color.rgb, in.color.a * cov);
    } else {
        let radius = min(in.params.x, min(in.rect.z, in.rect.w) * 0.5);
        let d = sd_round_box(in.local, in.rect.zw * 0.5, radius);
        let blur = in.params.z;
        if blur > 0.0 {
            color = vec4(in.color.rgb, in.color.a * (1.0 - smoothstep(-blur, blur, d)));
        } else {
            let aa = max(length(vec2(dpdx(d), dpdy(d))), 1e-4);
            let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
            let bw = in.params.y;
            var fill = in.color;
            if bw > 0.0 {
                let inner = clamp(0.5 - (d + bw) / aa, 0.0, 1.0);
                fill = mix(in.border_color, in.color, inner);
            }
            color = vec4(fill.rgb, fill.a * coverage);
        }
    }
    return vec4(color.rgb * color.a, color.a);
}
