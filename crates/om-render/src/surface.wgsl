// OpenMapper surface shader.
//
// One draw call per surface. The vertex stage emits the surface quad from
// four corners in canvas pixels; the fragment stage maps each pixel back to
// the unit square with the inverse homography, which gives perspective-
// correct texturing for any four-corner shape.

struct SurfaceUniforms {
    corners0: vec4<f32>,   // p0.xy, p1.xy
    corners1: vec4<f32>,   // p2.xy, p3.xy
    inv_h0: vec4<f32>,     // canvas px -> surface uv (mat3 columns)
    inv_h1: vec4<f32>,
    inv_h2: vec4<f32>,
    uv_m0: vec4<f32>,      // base uv -> texture uv (mat3 columns)
    uv_m1: vec4<f32>,
    uv_m2: vec4<f32>,
    canvas: vec4<f32>,     // width, height, local time, mapping (0 surface, 1 canvas)
    tint: vec4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    gen: vec4<f32>,        // source kind, scale, speed, angle (radians)
    misc: vec4<f32>,       // opacity, hue shift (turns), feather, mask
};

@group(0) @binding(0) var<uniform> u: SurfaceUniforms;
@group(1) @binding(0) var src_tex: texture_2d<f32>;
@group(1) @binding(1) var src_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) canvas_px: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let i = order[vi];
    var p: vec2<f32>;
    switch i {
        case 0u: { p = u.corners0.xy; }
        case 1u: { p = u.corners0.zw; }
        case 2u: { p = u.corners1.xy; }
        default: { p = u.corners1.zw; }
    }
    var out: VsOut;
    out.pos = vec4<f32>(p.x / u.canvas.x * 2.0 - 1.0, 1.0 - p.y / u.canvas.y * 2.0, 0.0, 1.0);
    out.canvas_px = p;
    return out;
}

const TAU: f32 = 6.28318530718;

fn rot2(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, s, -s, c);
}

fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 5; i++) {
        v += amp * value_noise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.2);
        amp *= 0.5;
    }
    return v;
}

// Antialiased square wave with duty 0.5; `x` in cycles.
fn aa_square(x: f32) -> f32 {
    let w = max(fwidth(x), 1e-4);
    let f = fract(x);
    return clamp((min(f, 1.0 - f) - 0.25) / w + 0.5, 0.0, 1.0);
}

// Distance (in cycles) to the nearest integer, antialiased into a line mask.
fn aa_line(x: f32, thickness: f32) -> f32 {
    let w = max(fwidth(x), 1e-4);
    let d = abs(fract(x + 0.5) - 0.5);
    return 1.0 - smoothstep(thickness * w, thickness * w + w, d);
}

fn generator(kind: u32, uv: vec2<f32>, t: f32) -> vec4<f32> {
    let scale = u.gen.y;
    let speed = u.gen.z;
    let angle = u.gen.w;
    let a = u.color_a;
    let b = u.color_b;
    let ruv = rot2(angle) * (uv - 0.5);

    switch kind {
        // Gradient: mirrored, so scrolling stays seamless.
        case 2u: {
            let x = ruv.x * max(scale * 0.25, 0.01) + 0.5 + t * speed;
            let f = 1.0 - abs(fract(x * 0.5) * 2.0 - 1.0);
            return mix(a, b, smoothstep(0.0, 1.0, f));
        }
        // Checker
        case 3u: {
            let p = ruv * scale + vec2<f32>(t * speed, 0.0);
            let m = aa_square(p.x * 0.5) ;
            let n = aa_square(p.y * 0.5);
            return mix(a, b, abs(m - n));
        }
        // Stripes
        case 4u: {
            let x = ruv.x * scale - t * speed;
            return mix(a, b, aa_square(x));
        }
        // Rings
        case 5u: {
            let d = length(uv - 0.5) * scale - t * speed;
            return mix(a, b, aa_square(d));
        }
        // Noise
        case 6u: {
            let n = fbm(ruv * scale + vec2<f32>(t * speed, t * speed * 0.37));
            return mix(a, b, smoothstep(0.25, 0.75, n));
        }
        // Plasma
        case 7u: {
            let p = ruv * scale;
            let tt = t * speed * TAU;
            var v = sin(p.x * 1.7 + tt) + sin(p.y * 1.3 - tt * 0.7)
                + sin((p.x + p.y) * 0.9 + tt * 0.5)
                + sin(length(p + vec2<f32>(sin(tt * 0.3), cos(tt * 0.4)) * 2.0) * 1.5);
            v = 0.5 + 0.5 * sin(v * 1.2);
            return mix(a, b, v);
        }
        // Test grid: background a, lines b, plus border, crosshair and circle.
        case 8u: {
            let cells = max(scale, 1.0);
            let g = max(aa_line(uv.x * cells, 1.0), aa_line(uv.y * cells, 1.0));
            let sub = max(aa_line(uv.x * cells * 4.0, 0.5), aa_line(uv.y * cells * 4.0, 0.5)) * 0.3;
            let border = max(aa_line(uv.x, 4.0), aa_line(uv.y, 4.0));
            let cross = max(aa_line(uv.x - 0.5, 2.0), aa_line(uv.y - 0.5, 2.0));
            let circle = aa_line(length(uv - 0.5) * 2.0 - 0.8, 2.0);
            let diag = max(aa_line(uv.x - uv.y, 1.0), aa_line(uv.x + uv.y - 1.0, 1.0)) * 0.5;
            let m = clamp(max(max(max(g, sub), max(border, cross)), max(circle, diag)), 0.0, 1.0);
            var c = mix(a, b, m);
            // Colored quadrant markers so orientation is obvious on the wall.
            let q = step(vec2<f32>(0.5), uv);
            let qc = vec3<f32>(1.0 - q.x, q.x * (1.0 - q.y), q.y);
            let blob = 1.0 - smoothstep(0.035, 0.04, length(fract(uv * 2.0) - 0.5) * 0.5);
            c = vec4<f32>(mix(c.rgb, qc, blob * 0.9), c.a);
            return c;
        }
        default: {
            return a;
        }
    }
}

fn hue_rotate(c: vec3<f32>, turns: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735027);
    let ang = turns * TAU;
    let ca = cos(ang);
    return c * ca + cross(k, c) * sin(ang) + k * dot(k, c) * (1.0 - ca);
}

// Signed coverage distance (in uv units, > 0 inside) for the surface mask.
fn mask_distance(suv: vec2<f32>, mask: u32) -> f32 {
    switch mask {
        case 1u: {
            return 0.5 - length(suv - 0.5);
        }
        case 2u: {
            // Apex at top-center, base along the bottom edge.
            let l = (suv.x - (0.5 - 0.5 * suv.y));
            let r = ((0.5 + 0.5 * suv.y) - suv.x);
            return min(min(l, r) * 0.894, 1.0 - suv.y);
        }
        default: {
            return min(min(suv.x, 1.0 - suv.x), min(suv.y, 1.0 - suv.y));
        }
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let inv_h = mat3x3<f32>(u.inv_h0.xyz, u.inv_h1.xyz, u.inv_h2.xyz);
    let hp = inv_h * vec3<f32>(in.canvas_px, 1.0);
    let suv = hp.xy / hp.z;

    let mapping = u.canvas.w;
    var base_uv = suv;
    if mapping > 0.5 {
        base_uv = in.canvas_px / u.canvas.xy;
    }
    let uv_m = mat3x3<f32>(u.uv_m0.xyz, u.uv_m1.xyz, u.uv_m2.xyz);
    let tuv = (uv_m * vec3<f32>(base_uv, 1.0)).xy;

    // Coverage: always at least ~1px of antialiasing, more with feather.
    let d = mask_distance(suv, u32(u.misc.w));
    let aa = max(fwidth(d), 1e-5);
    let coverage = smoothstep(0.0, max(u.misc.z, aa), d);

    let kind = u32(u.gen.x);
    let tex_color = textureSample(src_tex, src_samp, tuv);
    var color: vec4<f32>;
    if kind == 0u {
        color = tex_color;
    } else if kind == 1u {
        color = u.color_a;
    } else {
        color = generator(kind, tuv, u.canvas.z);
    }

    let rgb = clamp(hue_rotate(color.rgb, u.misc.y), vec3<f32>(0.0), vec3<f32>(1.0)) * u.tint.rgb;
    let alpha = color.a * u.tint.a * u.misc.x * coverage;
    return vec4<f32>(rgb * alpha, alpha);
}
