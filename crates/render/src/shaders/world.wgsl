// Static world (terrain, road, water) and instanced props.
// Prepended with common.wgsl.

@group(0) @binding(1) var shadow_tex: texture_depth_2d;
@group(0) @binding(2) var shadow_samp: sampler_comparison;

const K_PLAIN: u32 = 0u;
const K_TERRAIN: u32 = 1u;
const K_ROAD: u32 = 2u;
const K_KERB: u32 = 3u;
const K_FOLIAGE: u32 = 4u;
const K_WATER: u32 = 5u;
const K_EMISSIVE: u32 = 6u;
const K_CROWD: u32 = 7u;
const K_START_LIGHT: u32 = 8u;
const K_CHECKER: u32 = 9u;
const K_BARRIER: u32 = 10u;
const K_LAMP_GLOW: u32 = 11u;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) mat: vec4<f32>,
};

struct Inst {
    @location(5) pos_yaw: vec4<f32>,
    @location(6) params: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) @interpolate(flat) mat: vec4<f32>,
    @location(5) @interpolate(flat) tint: f32,
};

fn kind_of(m: vec4<f32>) -> u32 {
    return u32(round(m.x * 255.0));
}

fn place(v: VIn, inst: Inst) -> VOut {
    let yaw = inst.pos_yaw.w;
    let sc = inst.params.x;
    let ax = vec3<f32>(cos(yaw), 0.0, sin(yaw));
    let az = vec3<f32>(-sin(yaw), 0.0, cos(yaw));
    var lp = v.pos;
    if (kind_of(v.mat) == K_FOLIAGE) {
        // Wind sway grows with height.
        let w = v.uv.y * v.uv.y;
        let t = g.cam_pos.w * 1.6 + inst.params.z;
        let gust = 0.6 + 0.4 * sin(g.cam_pos.w * 0.37 + inst.params.z * 0.5);
        lp = lp + vec3<f32>(sin(t), 0.0, cos(t * 0.83)) * w * 0.22 * g.params2.y * gust;
    }
    let world = inst.pos_yaw.xyz + (ax * lp.x + vec3<f32>(0.0, lp.y, 0.0) + az * lp.z) * sc;
    let n = ax * v.nrm.x + vec3<f32>(0.0, v.nrm.y, 0.0) + az * v.nrm.z;
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(world, 1.0);
    o.world = world;
    o.nrm = n;
    o.uv = v.uv;
    o.color = v.color;
    o.mat = v.mat;
    o.tint = inst.params.y;
    return o;
}

@vertex
fn vs_static(v: VIn) -> VOut {
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    o.nrm = v.nrm;
    o.uv = v.uv;
    o.color = v.color;
    o.mat = v.mat;
    o.tint = 0.5;
    return o;
}

@vertex
fn vs_prop(v: VIn, inst: Inst) -> VOut {
    return place(v, inst);
}

@vertex
fn vs_static_shadow(v: VIn) -> @builtin(position) vec4<f32> {
    return g.view_proj * vec4<f32>(v.pos, 1.0);
}

@vertex
fn vs_prop_shadow(v: VIn, inst: Inst) -> @builtin(position) vec4<f32> {
    return place(v, inst).clip;
}

@fragment
fn fs_shadow() {
}

fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let ndl = dot(n, g.sun_dir.xyz);
    let offset = n * (0.035 + 0.07 * (1.0 - abs(ndl)));
    let p = g.shadow_mat * vec4<f32>(world + offset, 1.0);
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let depth = p.z - 0.0002;
    let t = g.params2.z;
    var s = 0.0;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            s = s + textureSampleCompareLevel(shadow_tex, shadow_samp, uv + vec2<f32>(f32(i), f32(j)) * t * 1.2, depth);
        }
    }
    s = s / 9.0;
    let edge = max(abs(uv.x - 0.5), abs(uv.y - 0.5)) * 2.0;
    let fade = smoothstep(0.82, 0.98, edge);
    let outside = select(0.0, 1.0, p.z > 1.0 || p.z < 0.0);
    return mix(s, 1.0, max(fade, outside));
}

// Warm pools of light under the street lamps (road and verges only).
// `wide` is how much wider than its base the road is here (the lamps
// stand back on the wide straight).
fn lamp_light(s: f32, u: f32, wide: f32) -> vec3<f32> {
    let strength = g.params.y;
    if (strength <= 0.0) {
        return vec3<f32>(0.0);
    }
    let sp = g.params.z;
    var acc = 0.0;
    let k0 = floor(s / sp);
    for (var i = 0; i < 2; i = i + 1) {
        let k = k0 + f32(i);
        let ls = k * sp;
        let on = select(0.0, 1.0, ls >= 60.0 && ls <= g.params2.w);
        let side = select(-1.0, 1.0, (i32(k) % 2) == 0);
        let d = vec2<f32>(s - ls, u - side * (g.params.w + wide));
        let r2 = dot(d, d) + 8.2 * 8.2;
        acc = acc + on * 8.2 / (r2 * sqrt(r2)) * 320.0;
    }
    return vec3<f32>(1.0, 0.68, 0.38) * acc * strength;
}

fn terrain_albedo(wp: vec3<f32>, n: vec3<f32>, near: f32, gravel: f32) -> vec4<f32> {
    let slope = 1.0 - n.y;
    let water = g.ground_bounce.w;
    let nz = fbm3(wp.xz * 0.03);
    let nz2 = vnoise(wp.xz * 0.45);
    let nz3 = vnoise(wp.xz * 2.1);
    var c = mix(g.grass.rgb, g.grass_dry.rgb, smoothstep(0.38, 0.72, nz + (wp.y - water) * 0.0015));
    c = c * (0.78 + 0.3 * nz2 + 0.12 * nz3);
    // Mown verge: greener and a touch darker close to the circuit.
    c = mix(c, g.grass.rgb * vec3<f32>(0.85, 1.05, 0.8) * (0.9 + 0.2 * nz3), near * 0.55);
    let rockf = smoothstep(0.22, 0.42, slope + (nz - 0.5) * 0.25);
    c = mix(c, g.rock.rgb * (0.75 + 0.35 * nz2 + 0.15 * nz3), rockf);
    let beach = 1.0 - smoothstep(1.2, 4.5, wp.y - water + (nz - 0.5) * 3.0);
    c = mix(c, g.sand.rgb * (0.9 + 0.2 * nz3), beach * (1.0 - rockf) * (1.0 - near));
    // Wet sand at the waterline.
    c = c * mix(0.55, 1.0, max(smoothstep(0.0, 0.9, wp.y - water), near));
    let snow = smoothstep(g.water_color.w - 20.0, g.water_color.w + 10.0, wp.y + (nz - 0.5) * 40.0)
        * (1.0 - smoothstep(0.3, 0.55, slope));
    c = mix(c, vec3<f32>(0.86, 0.89, 0.94), snow);
    c = mix(c, g.sand.rgb * 0.8 * (0.8 + 0.35 * nz3), gravel);
    let rough = mix(0.93, 0.7, snow);
    return vec4<f32>(c, rough);
}

fn water_height(p: vec2<f32>) -> f32 {
    let t = g.cam_pos.w;
    return vnoise(p * 0.05 + vec2<f32>(t * 0.02, t * 0.013)) * 1.2
        + vnoise(p * 0.21 - vec2<f32>(t * 0.09, -t * 0.05)) * 0.35
        + vnoise(p * 0.9 + vec2<f32>(t * 0.3, t * 0.2)) * 0.08;
}

@fragment
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let kind = kind_of(in.mat);
    var n = normalize(in.nrm);
    if (!front) {
        n = -n;
    }
    let wp = in.world;
    let to_cam = g.cam_pos.xyz - wp;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 0.001);
    var albedo = decode_albedo(in.color.rgb);
    var rough = in.mat.y;
    var metal = in.mat.z;
    let extra = in.mat.w;
    var emissive = vec3<f32>(0.0);
    var ao = 1.0;
    var lamps = vec3<f32>(0.0);

    if (kind == K_WATER) {
        let e = 0.7;
        let fade = 1.0 - smoothstep(150.0, 1400.0, dist);
        let h0 = water_height(wp.xz);
        let hx = water_height(wp.xz + vec2<f32>(e, 0.0));
        let hz = water_height(wp.xz + vec2<f32>(0.0, e));
        let wn = normalize(vec3<f32>((h0 - hx) * fade, e * 1.6, (h0 - hz) * fade));
        let ndv = max(dot(wn, v), 0.0);
        let fres = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
        let r = reflect(-v, wn);
        let refl = sky_color(vec3<f32>(r.x, abs(r.y), r.z), true);
        let shadow = sun_shadow(wp, vec3<f32>(0.0, 1.0, 0.0));
        let deep = g.water_color.rgb * (g.sky_zenith.rgb * 1.4 + g.sun_color.rgb * 0.04 * max(g.sun_dir.y, 0.0));
        var col = mix(deep, refl * mix(0.35, 1.0, shadow), fres);
        // Tight sun glint.
        let hv = normalize(g.sun_dir.xyz + v);
        col = col + g.sun_color.rgb * pow(max(dot(wn, hv), 0.0), 600.0) * 4.0 * shadow;
        col = apply_fog(col, wp);
        return vec4<f32>(col * g.sun_color.w, 1.0);
    }

    if (kind == K_TERRAIN) {
        // Micro-relief from noise so close terrain is not glassy.
        let nx = vnoise(wp.xz * 0.9 + vec2<f32>(0.37, 0.0)) - vnoise(wp.xz * 0.9 - vec2<f32>(0.37, 0.0));
        let nzz = vnoise(wp.xz * 0.9 + vec2<f32>(0.0, 0.37)) - vnoise(wp.xz * 0.9 - vec2<f32>(0.0, 0.37));
        let relief = 0.35 * (1.0 - smoothstep(30.0, 200.0, dist));
        n = normalize(n + vec3<f32>(nx, 0.0, nzz) * relief);
        let t = terrain_albedo(wp, n, in.color.r, in.color.g);
        albedo = t.rgb;
        rough = t.a;
        metal = 0.0;
        if (in.color.r > 0.99) {
            lamps = lamp_light(in.uv.y, in.uv.x, 0.0);
        }
    } else if (kind == K_ROAD) {
        let u = in.uv.x;
        let s = in.uv.y;
        // Half width varies (the wide straight), so it rides in the vertex.
        let hw = in.color.r * 16.0;
        let grain = vnoise(wp.xz * 3.1) * 0.5 + vnoise(wp.xz * 13.0) * 0.5;
        let patches = fbm3(vec2<f32>(s * 0.045, u * 0.25));
        var c = vec3<f32>(0.05, 0.05, 0.055) * (0.72 + 0.45 * grain) * (0.82 + 0.36 * patches);
        rough = 0.5 + grain * 0.22;
        // Rubbered racing line.
        let lane = (u - sin(s * 0.011) * hw * 0.35) / (hw * 0.4);
        let line = exp(-lane * lane);
        c = c * (1.0 - 0.32 * line);
        rough = rough - 0.12 * line;
        // White edge lines.
        let e = abs(u) - (hw - 0.55);
        let lw = smoothstep(-0.03, 0.03, e) * (1.0 - smoothstep(0.27, 0.33, e));
        c = mix(c, vec3<f32>(0.75, 0.75, 0.72) * (0.85 + 0.15 * grain), lw);
        rough = mix(rough, 0.45, lw);
        // Lane paint: a dashed centreline and dashed lane dividers, since
        // traffic runs both ways. Widths are antialiased by screen footprint.
        let aa = max(fwidth(u), 0.01);
        let dash = step(fract(s / 9.0), 0.45);
        let cl = (1.0 - smoothstep(0.07 - aa, 0.07 + aa, abs(u))) * dash;
        let ld = (1.0 - smoothstep(0.06 - aa, 0.06 + aa, abs(abs(u) - 3.15)))
            * step(fract(s / 12.0 + 0.3), 0.3);
        let paint = clamp(cl + ld, 0.0, 1.0) * (0.8 + 0.2 * grain);
        c = mix(c, vec3<f32>(0.72, 0.66, 0.42), cl * (0.8 + 0.2 * grain));
        c = mix(c, vec3<f32>(0.72, 0.72, 0.7), ld * (0.8 + 0.2 * grain));
        rough = mix(rough, 0.5, paint);
        // Start / finish checker band.
        if (s < 3.0) {
            let ch = (i32(floor((u + 64.0) / 0.75)) + i32(floor(s / 0.75))) % 2;
            c = select(vec3<f32>(0.02), vec3<f32>(0.8), ch == 0);
        }
        // Grid slots behind the line.
        let back = g.sand.w - s;
        if (back > 8.0 && back < 104.0) {
            let row = floor((back - 8.0) / 8.0);
            let side = select(-1.0, 1.0, (i32(row) % 2) == 0);
            let lu = u * side - hw * 0.35;
            let lv = fract((back - 8.0) / 8.0) * 8.0;
            let slot = (1.0 - step(0.2, lv)) * step(abs(lu), 1.2) + step(abs(abs(lu) - 1.2), 0.1) * (1.0 - step(2.2, lv));
            c = mix(c, vec3<f32>(0.75), clamp(slot, 0.0, 1.0));
        }
        albedo = c;
        lamps = lamp_light(s, u, hw - g.params2.x);
    } else if (kind == K_KERB) {
        let st = step(0.5, fract(in.uv.y / 2.4));
        albedo = mix(vec3<f32>(0.72, 0.03, 0.02), vec3<f32>(0.78, 0.78, 0.76), st);
        rough = 0.45;
        lamps = lamp_light(in.uv.y, in.uv.x, 0.0);
    } else if (kind == K_FOLIAGE) {
        albedo = albedo * mix(0.78, 1.22, in.tint);
        ao = mix(0.55, 1.0, in.uv.y);
    } else if (kind == K_EMISSIVE) {
        emissive = albedo * extra * 14.0 * (0.4 + 0.6 * g.params.y);
    } else if (kind == K_CROWD) {
        let cell = floor(vec2<f32>(in.uv.x * 140.0, in.uv.y * 5.0));
        let h = hash21(cell + vec2<f32>(in.tint * 31.0, extra * 17.0));
        let h2 = hash21(cell * 1.7 + 3.1);
        var shirt = vec3<f32>(fract(h * 3.1), fract(h * 7.3), fract(h * 13.7));
        shirt = mix(vec3<f32>(dot(shirt, vec3<f32>(0.33))), shirt, 0.55) * 0.55;
        let mask = step(0.3, fract(in.uv.x * 140.0)) * step(h2, 0.8);
        albedo = mix(vec3<f32>(0.035), shirt * shirt + vec3<f32>(0.03), mask);
        rough = 0.9;
    } else if (kind == K_START_LIGHT) {
        let idx = extra * 8.0;
        let lit = g.params.x;
        var lc = vec3<f32>(0.0);
        if (lit >= 5.5) {
            lc = vec3<f32>(0.1, 1.0, 0.25);
        } else if (idx < lit - 0.5) {
            lc = vec3<f32>(1.0, 0.04, 0.02);
        }
        albedo = vec3<f32>(0.02);
        emissive = lc * 40.0;
    } else if (kind == K_CHECKER) {
        let ch = (i32(floor(in.uv.x * 22.0)) + i32(floor(in.uv.y * 2.0))) % 2;
        albedo = select(vec3<f32>(0.02), vec3<f32>(0.8), ch == 0);
        rough = 0.6;
    } else if (kind == K_BARRIER) {
        let band = smoothstep(0.42, 0.5, in.uv.x) * (1.0 - smoothstep(0.5, 0.58, in.uv.x));
        ao = 1.0 - band * 0.5;
    } else if (kind == K_LAMP_GLOW) {
        emissive = vec3<f32>(1.0, 0.72, 0.42) * (2.0 + g.params.y * 60.0);
    } else if (kind == K_PLAIN) {
        albedo = albedo * mix(0.88, 1.12, in.tint);
    }

    // Contact shadows ground the bikes: they darken both sun and sky light.
    let contact = contact_shadow(wp);
    let shadow = mix(1.0, sun_shadow(wp, n), g.sun_dir.w) * (1.0 - contact);
    // The patch blocks sky light and reflections too, so it reads even
    // when the bike rides into a low sun.
    var col = shade(albedo, n, v, rough, metal, shadow, ao) * (1.0 - contact * 0.82);
    if (kind == K_FOLIAGE) {
        // Light through the leaves when backlit.
        let back = max(dot(-n, g.sun_dir.xyz), 0.0) * max(dot(-v, g.sun_dir.xyz), 0.0);
        col = col + albedo * g.sun_color.rgb * back * 0.35 * shadow;
    }
    col = col + albedo * lamps + emissive;
    col = apply_fog(col, wp);
    return vec4<f32>(col * g.sun_color.w, 1.0);
}
