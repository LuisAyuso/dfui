// Copies the canvas to the screen. The canvas holds sRGB-encoded BT.709 where
// 1.0 is SDR white; HDR content goes above 1 and wide-gamut colors below 0.
// SDR screens get it as is (clamped); HDR10 screens get BT.2020 in PQ.

override pq: bool = false;
// The target encodes to sRGB itself, so it gets linear values.
override srgb: bool = false;
// Un-premultiplied output.
override straight: bool = false;
// Nits that canvas 1.0 means on an HDR screen (ITU-R BT.2408 reference white).
override white: f32 = 203.0;

// The DFUI_SHOW_DAMAGE flash: a rect (physical px) and a color over it.
struct Tint {
    rect: vec4<f32>,
    color: vec4<f32>,
}

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var<uniform> tint: Tint;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}

// sRGB decoding that keeps the sign, for colors outside the BT.709 gamut.
fn linear(c: vec3<f32>) -> vec3<f32> {
    let a = abs(c);
    let l = select(pow((a + 0.055) / 1.055, vec3(2.4)), a / 12.92, a <= vec3(0.04045));
    return sign(c) * l;
}

// SMPTE ST 2084 encoding of absolute luminance.
fn pq_encode(nits: vec3<f32>) -> vec3<f32> {
    let y = pow(clamp(nits / 10000.0, vec3(0.0), vec3(1.0)), vec3(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3(78.84375));
}

// Linear BT.709 to linear BT.2020 (ITU-R BT.2087), column by column.
const BT709_TO_BT2020 = mat3x3<f32>(
    vec3(0.6274, 0.0691, 0.0164),
    vec3(0.3293, 0.9195, 0.0880),
    vec3(0.0433, 0.0114, 0.8956),
);

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    var c = textureLoad(canvas, vec2<i32>(p.xy), 0);
    let r = tint.rect;
    if p.x >= r.x && p.y >= r.y && p.x < r.x + r.z && p.y < r.y + r.w {
        c = vec4(mix(c.rgb, tint.color.rgb, tint.color.a), c.a);
    }
    if pq {
        let nits = BT709_TO_BT2020 * linear(c.rgb) * white;
        return vec4(pq_encode(nits), 1.0);
    }
    let rgb = clamp(c.rgb, vec3(0.0), vec3(1.0));
    if !(srgb || straight) {
        return vec4(rgb, c.a);
    }
    // Premultiplied: decode the color, not the coverage.
    var u = rgb / max(c.a, 1e-6);
    if srgb {
        u = linear(u);
    }
    if straight {
        return vec4(u, c.a);
    }
    return vec4(u * c.a, c.a);
}
