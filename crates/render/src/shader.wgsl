// One shader for every draw the renderer makes.
//
// A draw is a quad given in a *local frame* (element frame, text frame, screen frame) plus an
// affine transform from that frame to target pixels. The fragment stage computes
//
//     out = paint(p) * coverage(p) * opacity
//
// where `coverage` comes from an analytic shape (rounded rectangle / ellipse signed distance,
// with feather), a coverage mask texture, or a separable gaussian blur of a mask texture, and
// `paint` is a solid colour, a texture (premultiplied linear, or straight sRGB that is
// premultiplied here) or a gradient. Everything this shader writes into the Rgba16Float
// intermediate targets is **linear light, premultiplied alpha**.
//
// `fs_final` converts the linear premultiplied composite into the output frame: flatten over
// the screen background, encode to sRGB, premultiply in the encoded domain (RGBA8).
// `fs_adjust` does the same with the screen's corner pin and colour correction.

struct U {
    xf0: vec4<f32>,          // target.x = xf0.x * x + xf0.y * y + xf0.z
    xf1: vec4<f32>,          // target.y = xf1.x * x + xf1.y * y + xf1.z
    quad: vec4<f32>,         // drawn quad in the local frame: x0, y0, x1, y1
    target_info: vec4<f32>,  // target width, target height, opacity, unused
    shape: vec4<f32>,        // analytic shape rect in the local frame: x0, y0, x1, y1
    shape_p: vec4<f32>,      // corner radius, stroke width, feather width, feather style
    mask_xf: vec4<f32>,      // mask uv = (p - mask_xf.xy) / mask_xf.zw
    paint_xf: vec4<f32>,     // paint uv = (p - paint_xf.xy) / paint_xf.zw (before uv_m / crop)
    uv_m: vec4<f32>,         // 2x2 matrix about the uv centre (flip, quarter turns)
    uv_crop: vec4<f32>,      // final uv = uv_crop.xy + uv * uv_crop.zw
    color: vec4<f32>,        // solid paint, or multiplier of texture / gradient paint (premul linear)
    modes: vec4<u32>,        // coverage mode, paint mode, mask channel (0 = r, 3 = a), flags
    blur: vec4<f32>,         // direction (uv per tap), sigma (taps), radius (taps)
    grad: vec4<f32>,         // kind (0 linear, 1 radial, 2 angle), angle (rad), length, stop count
    grad_geo: vec4<f32>,     // gradient frame in the local frame: x, y, w, h
    stop_c: array<vec4<f32>, 8>, // stop colours, straight sRGB
    stop_p: array<vec4<f32>, 2>, // stop positions (8 packed)
    stop_m: array<vec4<f32>, 2>, // stop midpoints (8 packed)
};

@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var mask_tex: texture_2d<f32>;
@group(1) @binding(1) var paint_tex: texture_2d<f32>;
@group(1) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let local = mix(u.quad.xy, u.quad.zw, c);
    let px = vec2<f32>(
        u.xf0.x * local.x + u.xf0.y * local.y + u.xf0.z,
        u.xf1.x * local.x + u.xf1.y * local.y + u.xf1.z,
    );
    let ndc = vec2<f32>(px.x / u.target_info.x * 2.0 - 1.0, 1.0 - px.y / u.target_info.y * 2.0);
    var o: VOut;
    o.pos = vec4<f32>(ndc, 0.0, 1.0);
    o.local = local;
    return o;
}

// ---- coverage --------------------------------------------------------------------------------

fn sdf_rrect(p: vec2<f32>) -> f32 {
    let c = (u.shape.xy + u.shape.zw) * 0.5;
    let h = max((u.shape.zw - u.shape.xy) * 0.5, vec2<f32>(0.0));
    let r = clamp(u.shape_p.x, 0.0, min(h.x, h.y));
    let q = abs(p - c) - h + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sdf_ellipse(p: vec2<f32>) -> f32 {
    let c = (u.shape.xy + u.shape.zw) * 0.5;
    let r = max((u.shape.zw - u.shape.xy) * 0.5, vec2<f32>(1e-3));
    let k0 = length((p - c) / r);
    let k1 = length((p - c) / (r * r));
    if (k1 < 1e-6) {
        return -min(r.x, r.y);
    }
    return k0 * (k0 - 1.0) / k1;
}

// Fill coverage from a signed distance (negative inside), with optional feather.
fn fill_cov(d: f32) -> f32 {
    let fw = u.shape_p.z;
    let hard = clamp(0.5 - d, 0.0, 1.0);
    if (fw <= 0.0) {
        return hard;
    }
    let style = u.shape_p.w;
    if (style < 0.5) {
        // inside: fades in over `fw` from the outline inwards
        return hard * smoothstep(0.0, fw, -d);
    } else if (style < 1.5) {
        // centred on the outline
        return smoothstep(-fw * 0.5, fw * 0.5, -d);
    }
    // outside: full inside, fades out over `fw` beyond the outline
    return smoothstep(-fw, 0.0, -d);
}

// Coverage of a stroke of width `shape_p.y` centred on the outline.
fn stroke_cov(d: f32) -> f32 {
    let w = u.shape_p.y;
    let half = w * 0.5;
    return clamp(half + 0.5 - abs(d), 0.0, 1.0) * min(w, 1.0);
}

fn channel(s: vec4<f32>) -> f32 {
    if (u.modes.z == 0u) {
        return s.r;
    }
    return s.a;
}

fn inside01(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.y >= 0.0 && uv.x <= 1.0 && uv.y <= 1.0;
}

fn mask_at(p: vec2<f32>) -> f32 {
    let uv = (p - u.mask_xf.xy) / u.mask_xf.zw;
    let s = textureSampleLevel(mask_tex, samp, uv, 0.0);
    return select(0.0, channel(s), inside01(uv));
}

fn blur_at(p: vec2<f32>) -> f32 {
    let uv0 = (p - u.mask_xf.xy) / u.mask_xf.zw;
    let dir = u.blur.xy;
    let sigma = max(u.blur.z, 1e-3);
    let r = i32(u.blur.w);
    var acc = 0.0;
    var wsum = 0.0;
    for (var i = -r; i <= r; i = i + 1) {
        let f = f32(i);
        let w = exp(-0.5 * f * f / (sigma * sigma));
        let uv = uv0 + dir * f;
        let s = textureSampleLevel(mask_tex, samp, uv, 0.0);
        acc = acc + w * select(0.0, channel(s), inside01(uv));
        wsum = wsum + w;
    }
    return acc / max(wsum, 1e-6);
}

// ---- paint -----------------------------------------------------------------------------------

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn stop_pos(i: i32) -> f32 {
    return u.stop_p[i / 4][i % 4];
}

fn stop_mid(i: i32) -> f32 {
    return u.stop_m[i / 4][i % 4];
}

fn premul_texel(p: vec2<i32>, hi: vec2<i32>) -> vec4<f32> {
    let s = textureLoad(paint_tex, clamp(p, vec2<i32>(0), hi), 0);
    return vec4<f32>(s.rgb * s.a, s.a);
}

// Bilinear sample (level 0, clamp to edge) of a straight-alpha texture, premultiplying each
// texel before filtering: the hardware filter would blend a transparent texel's black into
// the colour of its neighbours (a dark fringe on rotated or scaled text).
fn sample_straight(uv: vec2<f32>) -> vec4<f32> {
    let dim = vec2<i32>(textureDimensions(paint_tex, 0));
    let hi = dim - vec2<i32>(1);
    let t = uv * vec2<f32>(dim) - vec2<f32>(0.5);
    let i = vec2<i32>(floor(t));
    let f = t - floor(t);
    let a = premul_texel(i, hi);
    let b = premul_texel(i + vec2<i32>(1, 0), hi);
    let c = premul_texel(i + vec2<i32>(0, 1), hi);
    let d = premul_texel(i + vec2<i32>(1, 1), hi);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn gradient(p: vec2<f32>) -> vec4<f32> {
    let g = u.grad_geo;
    let ctr = g.xy + g.zw * 0.5;
    let kind = u.grad.x;
    let a = u.grad.y;
    let len = max(u.grad.z, 1e-4);
    var t = 0.0;
    if (kind < 0.5) {
        let d = vec2<f32>(cos(a), sin(a));
        let span = max(abs(g.z * d.x) + abs(g.w * d.y), 1e-3);
        t = dot(p - ctr, d) / (span * len) + 0.5;
    } else if (kind < 1.5) {
        let radius = max(length(g.zw) * 0.5 * len, 1e-3);
        t = length(p - ctr) / radius;
    } else {
        let v = p - ctr;
        t = fract((atan2(v.y, v.x) - a) / 6.2831853);
    }
    t = clamp(t, 0.0, 1.0);
    let n = i32(u.grad.w);
    var col = u.stop_c[0];
    if (n > 1) {
        if (t >= stop_pos(n - 1)) {
            col = u.stop_c[n - 1];
        } else if (t > stop_pos(0)) {
            for (var i = 0; i < n - 1; i = i + 1) {
                let p0 = stop_pos(i);
                let p1 = stop_pos(i + 1);
                if (t >= p0 && t <= p1) {
                    var f = select((t - p0) / (p1 - p0), 0.0, p1 - p0 < 1e-6);
                    let mid = clamp(stop_mid(i), 0.01, 0.99);
                    f = pow(f, log(0.5) / log(mid));
                    col = mix(u.stop_c[i], u.stop_c[i + 1], f);
                }
            }
        }
    }
    let lin = srgb_to_linear(clamp(col.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(lin * col.a, col.a);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let p = in.local;
    var cov = 1.0;
    let cm = u.modes.x;
    if (cm == 1u) {
        cov = mask_at(p);
    } else if (cm == 2u) {
        cov = fill_cov(sdf_rrect(p));
    } else if (cm == 3u) {
        cov = stroke_cov(sdf_rrect(p));
    } else if (cm == 4u) {
        cov = fill_cov(sdf_ellipse(p));
    } else if (cm == 5u) {
        cov = stroke_cov(sdf_ellipse(p));
    } else if (cm == 6u) {
        cov = blur_at(p);
    }

    var col = u.color;
    let pm = u.modes.y;
    if (pm == 1u || pm == 2u) {
        let base = (p - u.paint_xf.xy) / u.paint_xf.zw;
        let c = base - vec2<f32>(0.5);
        let r = vec2<f32>(dot(u.uv_m.xy, c), dot(u.uv_m.zw, c)) + vec2<f32>(0.5);
        let uv = u.uv_crop.xy + r * u.uv_crop.zw;
        var s = vec4<f32>(0.0);
        if (pm == 2u) {
            s = sample_straight(uv);
        } else {
            s = textureSample(paint_tex, samp, uv);
        }
        let clip = (u.modes.w & 1u) != 0u;
        if (clip && !inside01(base)) {
            s = vec4<f32>(0.0);
        }
        col = s * u.color;
    } else if (pm == 3u) {
        col = gradient(p) * u.color.a;
    }
    return col * (cov * u.target_info.z);
}

// Linear premultiplied composite -> RGBA8, sRGB-encoded, premultiplied in the encoded domain.
// modes.x: 1 = opaque (flatten over the background, then over black; alpha forced to 1).
@fragment
fn fs_final(in: VOut) -> @location(0) vec4<f32> {
    let c0 = textureLoad(paint_tex, vec2<i32>(in.pos.xy), 0);
    var c = c0 + u.color * (1.0 - c0.a);
    if (u.modes.x == 1u) {
        c = vec4<f32>(c.rgb, 1.0);
    }
    let a = clamp(c.a, 0.0, 1.0);
    var straight = vec3<f32>(0.0);
    if (a > 1e-6) {
        straight = clamp(c.rgb / a, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let enc = linear_to_srgb(straight);
    return vec4<f32>(enc * a, a);
}

// The output pass with the screen's adjustments (see `adjust.rs`); the same conversion as
// `fs_final`, plus:
// - modes.y & 1: corner pin. stop_c[0..3] = rows of the homography from output pixels to the
//   picture's unit square; the composite is sampled there (bilinear), with an anti-aliased
//   edge; outside the quad the pixel is transparent (black once flattened on opaque screens).
// - modes.y & 2: colour correction of the straight, sRGB-encoded colour. stop_c[3] = (red,
//   green, blue gains, saturation), stop_c[4] = (brightness, contrast, gamma, black level).
@fragment
fn fs_adjust(in: VOut) -> @location(0) vec4<f32> {
    var c0 = vec4<f32>(0.0);
    var cov = 1.0;
    if ((u.modes.y & 1u) != 0u) {
        let q = vec3<f32>(in.pos.xy, 1.0);
        let w = dot(u.stop_c[2].xyz, q);
        if (w <= 1e-9) {
            cov = 0.0;
        } else {
            let uv = vec2<f32>(dot(u.stop_c[0].xyz, q), dot(u.stop_c[1].xyz, q)) / w;
            let dim = vec2<f32>(textureDimensions(paint_tex, 0));
            let s = uv * dim;
            let edge = min(min(s.x, dim.x - s.x), min(s.y, dim.y - s.y));
            cov = clamp(edge + 0.5, 0.0, 1.0);
            c0 = textureSampleLevel(paint_tex, samp, uv, 0.0);
        }
    } else {
        c0 = textureLoad(paint_tex, vec2<i32>(in.pos.xy), 0);
    }
    var c = c0 + u.color * (1.0 - c0.a);
    if (u.modes.x == 1u) {
        c = vec4<f32>(c.rgb, 1.0);
    }
    let a = clamp(c.a, 0.0, 1.0);
    var straight = vec3<f32>(0.0);
    if (a > 1e-6) {
        straight = clamp(c.rgb / a, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    var enc = linear_to_srgb(straight);
    if ((u.modes.y & 2u) != 0u) {
        let gain = u.stop_c[3];
        let k = u.stop_c[4];
        var g = enc * gain.rgb;
        let luma = dot(g, vec3<f32>(0.2126, 0.7152, 0.0722));
        g = mix(vec3<f32>(luma), g, gain.a);
        g = (g - vec3<f32>(0.5)) * k.y + vec3<f32>(0.5 + k.x);
        g = vec3<f32>(k.w) + g * (1.0 - k.w);
        g = clamp(g, vec3<f32>(0.0), vec3<f32>(1.0));
        enc = pow(g, vec3<f32>(1.0 / k.z));
    }
    if (u.modes.x == 1u) {
        // Opaque: outside the quad is black (premultiplied, then over black).
        return vec4<f32>(enc * cov, 1.0);
    }
    return vec4<f32>(enc * a, a) * cov;
}

// ---- video ------------------------------------------------------------------------------------
// YCbCr planes → linear RGB, one output pixel per luma sample. mask_tex = Y plane, paint_tex =
// interleaved CbCr plane at half resolution. modes.x: 0 = NV12 (8-bit bytes), 1 = P010 (16-bit
// little-endian words in two / four 8-bit channels, 10 significant bits MSB-aligned).
// stop_c[0..3]: rows of the matrix from raw code values (Y, Cb, Cr, 1) to R'G'B'.

fn chroma_codes(p: vec2<i32>, dim: vec2<i32>) -> vec2<f32> {
    let q = clamp(p, vec2<i32>(0), dim - vec2<i32>(1));
    let s = textureLoad(paint_tex, q, 0);
    if (u.modes.x == 1u) {
        return vec2<f32>(s.r + s.g * 256.0, s.b + s.a * 256.0) * (255.0 / 64.0);
    }
    return s.rg * 255.0;
}

@fragment
fn fs_yuv(in: VOut) -> @location(0) vec4<f32> {
    // The upright output pixel → the stored picture's pixel (modes.y: counter-clockwise
    // quarter turns that show it upright).
    let o = vec2<i32>(in.pos.xy);
    let sd = vec2<i32>(textureDimensions(mask_tex));
    var px = o;
    if (u.modes.y == 1u) {
        px = vec2<i32>(sd.x - 1 - o.y, o.x);
    } else if (u.modes.y == 2u) {
        px = vec2<i32>(sd.x - 1 - o.x, sd.y - 1 - o.y);
    } else if (u.modes.y == 3u) {
        px = vec2<i32>(o.y, sd.y - 1 - o.x);
    }
    let ys = textureLoad(mask_tex, px, 0);
    var y = ys.r * 255.0;
    if (u.modes.x == 1u) {
        y = (ys.r + ys.g * 256.0) * (255.0 / 64.0);
    }
    // Modes 2–4: Y and alpha packed as pairs (straight, premultiplied, ignored).
    var a = 1.0;
    if (u.modes.x == 2u || u.modes.x == 3u) {
        a = ys.g;
    }
    // Chroma siting: co-sited with the left luma sample, centred vertically (4:2:0 default).
    let dim = vec2<i32>(textureDimensions(paint_tex));
    let cx = f32(px.x) * 0.5;
    let cy = (f32(px.y) + 0.5) * 0.5 - 0.5;
    let x0 = i32(floor(cx));
    let y0 = i32(floor(cy));
    let fx = cx - floor(cx);
    let fy = cy - floor(cy);
    let c00 = chroma_codes(vec2<i32>(x0, y0), dim);
    let c10 = chroma_codes(vec2<i32>(x0 + 1, y0), dim);
    let c01 = chroma_codes(vec2<i32>(x0, y0 + 1), dim);
    let c11 = chroma_codes(vec2<i32>(x0 + 1, y0 + 1), dim);
    let c = mix(mix(c00, c10, fx), mix(c01, c11, fx), fy);
    let v = vec4<f32>(y, c.x, c.y, 1.0);
    var rgb = clamp(
        vec3<f32>(dot(u.stop_c[0], v), dot(u.stop_c[1], v), dot(u.stop_c[2], v)),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    if (u.modes.x == 3u) {
        // Premultiplied R'G'B': un-premultiply before linearising.
        rgb = select(vec3<f32>(0.0), clamp(rgb / max(a, 1.0 / 255.0), vec3<f32>(0.0), vec3<f32>(1.0)), a > 0.0);
    }
    // Premultiplied linear output (the converted texture is drawn as TEXTURE_PREMUL).
    return vec4<f32>(srgb_to_linear(rgb) * a, a);
}
