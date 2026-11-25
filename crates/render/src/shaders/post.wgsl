// Post chain: bloom prefilter/down/up, tonemap + grade, FXAA.

struct Post {
    // 1/w, 1/h of the source texture, w/h of the target
    texel: vec4<f32>,
    // exposure, bloom strength, vignette, boost (0..1)
    params: vec4<f32>,
    // rgb grade, w: 1 = encode for an sRGB surface (output linear)
    grade: vec4<f32>,
    // threshold, knee, time, hdr scale (inverse)
    extra: vec4<f32>,
    // ink rgb (linear), w: outline strength
    ink: vec4<f32>,
    // impact, speed lines, grain, camera near
    fx: vec4<f32>,
    shadow_tint: vec4<f32>,
    high_tint: vec4<f32>,
    // impact-frame tones
    accent: vec4<f32>,
    paper: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src2: texture_2d<f32>;
@group(0) @binding(3) var<uniform> post: Post;
// Composite only: scene depth (reversed-Z, 0 = sky) for the ink outlines.
@group(0) @binding(4) var depth_tex: texture_2d<f32>;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: VOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

fn tap(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(t, samp, uv, 0.0).rgb;
}

// 13-tap downsample (Jimenez 2014).
fn down13(t: texture_2d<f32>, uv: vec2<f32>, d: vec2<f32>) -> vec3<f32> {
    let a = tap(t, uv + d * vec2<f32>(-2.0, -2.0));
    let b = tap(t, uv + d * vec2<f32>(0.0, -2.0));
    let c = tap(t, uv + d * vec2<f32>(2.0, -2.0));
    let dd = tap(t, uv + d * vec2<f32>(-2.0, 0.0));
    let e = tap(t, uv);
    let f = tap(t, uv + d * vec2<f32>(2.0, 0.0));
    let gg = tap(t, uv + d * vec2<f32>(-2.0, 2.0));
    let h = tap(t, uv + d * vec2<f32>(0.0, 2.0));
    let i = tap(t, uv + d * vec2<f32>(2.0, 2.0));
    let j = tap(t, uv + d * vec2<f32>(-1.0, -1.0));
    let k = tap(t, uv + d * vec2<f32>(1.0, -1.0));
    let l = tap(t, uv + d * vec2<f32>(-1.0, 1.0));
    let m = tap(t, uv + d * vec2<f32>(1.0, 1.0));
    return e * 0.125 + (a + c + gg + i) * 0.03125 + (b + dd + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

@fragment
fn fs_prefilter(in: VOut) -> @location(0) vec4<f32> {
    var c = down13(src, in.uv, post.texel.xy) * post.extra.w;
    // Clamp fireflies, then a soft-knee threshold.
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = c / (1.0 + luma * 0.02);
    let br = max(c.r, max(c.g, c.b));
    let th = post.extra.x;
    let knee = post.extra.y;
    var soft = clamp(br - th + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 0.0001);
    let w = max(soft, br - th) / max(br, 0.0001);
    return vec4<f32>(c * w / post.extra.w, 1.0);
}

@fragment
fn fs_down(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(down13(src, in.uv, post.texel.xy), 1.0);
}

@fragment
fn fs_up(in: VOut) -> @location(0) vec4<f32> {
    let d = post.texel.xy;
    var s = tap(src, in.uv) * 4.0;
    s = s + (tap(src, in.uv + vec2<f32>(-d.x, 0.0)) + tap(src, in.uv + vec2<f32>(d.x, 0.0))
        + tap(src, in.uv + vec2<f32>(0.0, -d.y)) + tap(src, in.uv + vec2<f32>(0.0, d.y))) * 2.0;
    s = s + tap(src, in.uv + vec2<f32>(-d.x, -d.y)) + tap(src, in.uv + vec2<f32>(d.x, -d.y))
        + tap(src, in.uv + vec2<f32>(-d.x, d.y)) + tap(src, in.uv + vec2<f32>(d.x, d.y));
    let low = s / 16.0;
    return vec4<f32>(low + tap(src2, in.uv), 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn depth_at(p: vec2<i32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(depth_tex));
    return textureLoad(depth_tex, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0).r;
}

// Ink: the geometry is rendered properly first, then lines are drawn over it
// where the depth buffer creases or breaks. Reversed-Z depth is linear in
// screen space across any flat surface, so its second difference is ~0 on
// planes (roads, walls, slopes) and spikes only at silhouettes and creases.
fn ink_line(uv: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(depth_tex));
    let p = vec2<i32>(uv * dims);
    let c = depth_at(p);
    let l = depth_at(p + vec2<i32>(-1, 0));
    let r = depth_at(p + vec2<i32>(1, 0));
    let u = depth_at(p + vec2<i32>(0, -1));
    let d = depth_at(p + vec2<i32>(0, 1));
    let near_d = max(max(l, r), max(max(u, d), c));
    if (near_d <= 0.0) {
        return 0.0; // all sky
    }
    // Draw on the nearer side of a break so lines hug the object.
    let lap = abs(l + r - 2.0 * c) + abs(u + d - 2.0 * c);
    let e = lap / max(near_d, 1e-7);
    let line = smoothstep(0.025, 0.09, e);
    // Lines thin out with distance like aerial perspective; none on sky.
    let dist = post.fx.w / max(near_d, 1e-7);
    let fade = 1.0 - smoothstep(60.0, 420.0, dist);
    return line * fade * select(1.0, 0.6, c <= 0.0);
}

// Manga speed lines while boosting: a few thin radial streaks at the frame
// edge, re-drawn a dozen times a second like hand-inked frames.
fn speed_lines(uv: vec2<f32>, amount: f32) -> f32 {
    if (amount <= 0.0) {
        return 0.0;
    }
    let aspect = post.texel.z / max(post.texel.w, 1.0);
    let q = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let r = length(q);
    let lanes = 220.0;
    let a = (atan2(q.y, q.x) / 6.2831853 + 0.5) * lanes;
    let lane = floor(a);
    let beat = floor(post.extra.z * 12.0);
    let h = fract(sin(lane * 91.7 + beat * 13.1) * 43758.547);
    let h2 = fract(sin(lane * 17.3 + beat * 7.7) * 24634.633);
    let on = step(0.9, h);
    // Each streak starts at its own radius and tapers toward the centre.
    let start = 0.48 + h2 * 0.22;
    let edge = smoothstep(start, start + 0.25, r);
    // Width in lanes shrinks toward the centre so streaks taper.
    let w = 0.18 * edge;
    let thin = 1.0 - smoothstep(w * 0.5, w, abs(fract(a) - 0.5));
    return on * edge * thin * amount;
}

@fragment
fn fs_composite(in: VOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let boost = post.params.w;
    let centred = uv - vec2<f32>(0.5);
    let r = length(centred);
    // Speed: chromatic fringe and a short radial smear at the edges.
    let ca = centred * (0.0015 + boost * 0.006) * r;
    var hdr = vec3<f32>(
        tap(src, uv - ca).r,
        tap(src, uv).g,
        tap(src, uv + ca).b,
    );
    if (boost > 0.01) {
        var acc = hdr;
        for (var i = 1; i <= 4; i = i + 1) {
            acc = acc + tap(src, uv - centred * f32(i) * 0.012 * boost * r);
        }
        hdr = mix(hdr, acc / 5.0, smoothstep(0.15, 0.6, r));
    }
    hdr = hdr * post.extra.w;
    let bloom = tap(src2, uv) * post.extra.w / 5.0;
    var c = hdr + bloom * post.params.y;
    // Ink over the lit scene, before exposure, so it reads as line work
    // rather than shadow.
    if (post.ink.w > 0.0) {
        let k = ink_line(uv) * post.ink.w;
        c = mix(c, post.ink.rgb * 0.25, k);
    }
    c = c * post.params.x;
    c = aces(c);
    // Grade: tint, gentle contrast, saturation.
    c = c * post.grade.rgb;
    var l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = mix(vec3<f32>(l), c, 1.12);
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    c = c * c * (3.0 - 2.0 * c) * 0.25 + c * 0.75;
    // Split tone: cool or warm the shadows and highlights per track.
    l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = c * mix(post.shadow_tint.rgb, post.high_tint.rgb, smoothstep(0.1, 0.8, l));
    // Speed lines in the paper tone.
    let sl = speed_lines(uv, post.fx.y);
    c = mix(c, post.paper.rgb, sl * 0.6);
    // Vignette.
    let vig = 1.0 - post.params.z * smoothstep(0.35, 0.85, r * (1.0 + boost * 0.25));
    c = c * vig;
    // Impact frame: a held, three-tone inverted drawing (ink, hit colour,
    // paper) for a few frames when something big lands.
    let impact = post.fx.x;
    if (impact > 0.0) {
        let li = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        var tone = post.paper.rgb;
        if (li > 0.62) {
            tone = post.ink.rgb;
        } else if (li > 0.3) {
            tone = post.accent.rgb;
        }
        c = mix(c, tone, step(0.5, impact) * 0.92);
    }
    // Film grain, seeded per frame, strongest in the mid-tones.
    let g = fract(sin(dot(in.clip.xy + post.extra.z * 517.0, vec2<f32>(12.9898, 78.233))) * 43758.547) - 0.5;
    l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = c + g * post.fx.z * (1.0 - abs(l * 2.0 - 1.0));
    // To display gamma, luma in alpha for FXAA. Dither against banding.
    let n = fract(sin(dot(in.clip.xy, vec2<f32>(12.9898, 78.233)) + post.extra.z) * 43758.547);
    var o = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)) + (n - 0.5) / 255.0;
    return vec4<f32>(o, dot(o, vec3<f32>(0.299, 0.587, 0.114)));
}

@fragment
fn fs_fxaa(in: VOut) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let uv = in.uv;
    let nw = textureSampleLevel(src, samp, uv + vec2<f32>(-1.0, -1.0) * t, 0.0).a;
    let ne = textureSampleLevel(src, samp, uv + vec2<f32>(1.0, -1.0) * t, 0.0).a;
    let sw = textureSampleLevel(src, samp, uv + vec2<f32>(-1.0, 1.0) * t, 0.0).a;
    let se = textureSampleLevel(src, samp, uv + vec2<f32>(1.0, 1.0) * t, 0.0).a;
    let mid = textureSampleLevel(src, samp, uv, 0.0);
    let m = mid.a;
    let lmin = min(m, min(min(nw, ne), min(sw, se)));
    let lmax = max(m, max(max(nw, ne), max(sw, se)));
    var dir = vec2<f32>(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.03125, 1.0 / 128.0);
    let rcp = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp, vec2<f32>(-8.0), vec2<f32>(8.0)) * t;
    let a = 0.5 * (textureSampleLevel(src, samp, uv + dir * (1.0 / 3.0 - 0.5), 0.0).rgb
        + textureSampleLevel(src, samp, uv + dir * (2.0 / 3.0 - 0.5), 0.0).rgb);
    let b = a * 0.5 + 0.25 * (textureSampleLevel(src, samp, uv - dir * 0.5, 0.0).rgb
        + textureSampleLevel(src, samp, uv + dir * 0.5, 0.0).rgb);
    let lb = dot(b, vec3<f32>(0.299, 0.587, 0.114));
    var c = select(b, a, lb < lmin || lb > lmax);
    if (lmax - lmin < 0.03) {
        c = mid.rgb;
    }
    if (post.grade.w > 0.5) {
        c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
    }
    return vec4<f32>(c, 1.0);
}
