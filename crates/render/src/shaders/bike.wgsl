// Bikes and riders, instanced. Prepended with common.wgsl.
// Vertex mat: (part id, roughness, metalness, flags).

@group(0) @binding(1) var shadow_tex: texture_depth_2d;
@group(0) @binding(2) var shadow_samp: sampler_comparison;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) mat: vec4<f32>,
};

struct Inst {
    @location(5) m0: vec4<f32>,
    @location(6) m1: vec4<f32>,
    @location(7) m2: vec4<f32>,
    @location(8) m3: vec4<f32>,
    // rgb paint, w ghost
    @location(9) paint: vec4<f32>,
    // rgb leathers, w brake light
    @location(10) paint2: vec4<f32>,
    // front spin, rear spin, steer, punch (-1 left .. 1 right)
    @location(11) anim: vec4<f32>,
    // rider offset (bike space) xyz, w rider tumble angle
    @location(12) rider: vec4<f32>,
    // hang-off (-1 left .. 1 right), tuck 0..1, front and rear suspension
    // compression (m)
    @location(13) pose: vec4<f32>,
    // kick (-1 left .. 1 right), block 0..1
    @location(14) act: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) mat: vec4<f32>,
    @location(4) @interpolate(flat) paint: vec4<f32>,
    @location(5) @interpolate(flat) paint2: vec4<f32>,
    // Height above the ground in bike space, for a little occlusion.
    @location(6) local_y: f32,
};

fn rot_x(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, c, s), vec3<f32>(0.0, -s, c));
}

fn rot_z(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(c, s, 0.0), vec3<f32>(-s, c, 0.0), vec3<f32>(0.0, 0.0, 1.0));
}

// Rotation by `a` about unit axis `k` (right-handed).
fn rot_axis(k: vec3<f32>, a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    let t = 1.0 - c;
    return mat3x3<f32>(
        vec3<f32>(t * k.x * k.x + c, t * k.x * k.y + s * k.z, t * k.x * k.z - s * k.y),
        vec3<f32>(t * k.x * k.y - s * k.z, t * k.y * k.y + c, t * k.y * k.z + s * k.x),
        vec3<f32>(t * k.x * k.z + s * k.y, t * k.y * k.z - s * k.x, t * k.z * k.z + c),
    );
}

struct Posed {
    pos: vec3<f32>,
    nrm: vec3<f32>,
};

// Rig points; these match the constants in models.rs.
const FRONT_AXLE: vec3<f32> = vec3<f32>(0.0, 0.325, -0.72);
const REAR_AXLE: vec3<f32> = vec3<f32>(0.0, 0.325, 0.70);
const STEER_HEAD: vec3<f32> = vec3<f32>(0.0, 0.98, -0.5);
const PIVOT_Z: f32 = 0.06;
const R_SHOULDER: vec3<f32> = vec3<f32>(0.2, 1.30, -0.16);
const L_SHOULDER: vec3<f32> = vec3<f32>(-0.2, 1.30, -0.16);
const R_HIP: vec3<f32> = vec3<f32>(0.13, 1.0, 0.2);
const L_HIP: vec3<f32> = vec3<f32>(-0.13, 1.0, 0.2);
const R_HAND: vec3<f32> = vec3<f32>(0.31, 0.97, -0.43);
const L_HAND: vec3<f32> = vec3<f32>(-0.31, 0.97, -0.43);
const SEAT: vec3<f32> = vec3<f32>(0.0, 0.92, 0.18);
const HIPS: vec3<f32> = vec3<f32>(0.0, 1.0, 0.22);

// How far the sprung mass has sunk at bike-space z: the fork at the front,
// the shock at the back, pitching in between.
fn sag_at(z: f32, inst: Inst) -> f32 {
    let t = clamp((z - FRONT_AXLE.z) / (REAR_AXLE.z - FRONT_AXLE.z), 0.0, 1.0);
    return mix(inst.pose.z, inst.pose.w, t);
}

fn pose(v: VIn, inst: Inst) -> Posed {
    let part = u32(round(v.mat.x * 255.0));
    var p = v.pos;
    var n = v.nrm;
    let rake = normalize(STEER_HEAD - FRONT_AXLE);
    let steer = rot_axis(rake, -inst.anim.z);
    let sprung = vec3<f32>(0.0, -sag_at(v.pos.z, inst), 0.0);
    if (part == 1u) {
        let r = steer * rot_x(-inst.anim.x);
        p = r * (p - FRONT_AXLE) + FRONT_AXLE;
        n = r * n;
    } else if (part == 2u) {
        let r = rot_x(-inst.anim.y);
        p = r * (p - REAR_AXLE) + REAR_AXLE;
        n = r * n;
    } else if (part == 3u) {
        p = steer * (p - FRONT_AXLE) + FRONT_AXLE;
        n = steer * n;
    } else if (part == 4u) {
        // The pivot rides with the frame; the axle stays on the ground.
        let w = clamp((REAR_AXLE.z - v.pos.z) / (REAR_AXLE.z - PIVOT_Z), 0.0, 1.0);
        p.y = p.y - sag_at(PIVOT_Z, inst) * w;
    } else if (part == 5u) {
        // Stanchions slide down along the rake into the sliders.
        p = p - rake * inst.pose.z;
        p = steer * (p - FRONT_AXLE) + FRONT_AXLE;
        n = steer * n;
    } else if (part >= 10u) {
        let hang = inst.pose.x;
        let tuck = inst.pose.y;
        let punch = inst.anim.w;
        let kick = inst.act.x;
        let block = inst.act.y;
        if (part == 13u || part == 14u) {
            let side = select(-1.0, 1.0, part == 13u);
            let hip = select(L_HIP, R_HIP, part == 13u);
            // Inside knee out when hanging off; a kick swings the whole leg
            // out sideways and straightens it.
            let knee = max(hang * side, 0.0) * 0.5;
            let k = max(kick * side, 0.0);
            let r = rot_z(side * (knee + k * 1.25)) * rot_x(-k * 0.35);
            p = r * (p - hip) + hip;
            n = r * n;
        } else if (part == 11u || part == 12u) {
            let side = select(-1.0, 1.0, part == 11u);
            let sh = select(L_SHOULDER, R_SHOULDER, part == 11u);
            let hand = select(L_HAND, R_HAND, part == 11u);
            // Tucked: elbows drop, pivoting about the grip.
            let rt = rot_x(tuck * 0.15);
            p = rt * (p - hand) + hand;
            n = rt * n;
            // Guard up: forearms come up in front of the helmet.
            if (block > 0.0) {
                let r = rot_z(-side * 0.35 * block) * rot_x(block);
                p = r * (p - sh) + sh;
                n = r * n;
            }
            let ps = max(punch * side, 0.0);
            if (ps > 0.0) {
                let r = rot_z(side * ps * 1.5) * rot_x(ps * 0.5);
                p = r * (p - sh) + sh;
                n = r * n;
            }
        } else {
            // Torso and head fold down towards the tank.
            let r = rot_x(-tuck * 0.13);
            p = r * (p - HIPS) + HIPS + vec3<f32>(0.0, -0.025 * tuck, 0.0);
            n = r * n;
        }
        // Hang off the inside of the bike in a corner.
        let rh = rot_z(-hang * 0.32);
        p = rh * (p - SEAT) + SEAT + vec3<f32>(hang * 0.11, -abs(hang) * 0.03, 0.0);
        n = rh * n;
        // Thrown rider: tumble about the hips, then offset.
        if (inst.rider.w != 0.0 || dot(inst.rider.xyz, inst.rider.xyz) > 0.0) {
            let hip = vec3<f32>(0.0, 1.0, 0.1);
            let r = rot_x(inst.rider.w) * rot_z(inst.rider.w * 0.37);
            p = r * (p - hip) + hip + inst.rider.xyz;
            n = r * n;
        }
        p = p + sprung;
    }
    if (part == 0u) {
        p = p + sprung;
    }
    let m = mat4x4<f32>(inst.m0, inst.m1, inst.m2, inst.m3);
    var o: Posed;
    o.pos = (m * vec4<f32>(p, 1.0)).xyz;
    o.nrm = normalize((m * vec4<f32>(n, 0.0)).xyz);
    return o;
}

@vertex
fn vs_main(v: VIn, inst: Inst) -> VOut {
    let ps = pose(v, inst);
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(ps.pos, 1.0);
    o.world = ps.pos;
    o.nrm = ps.nrm;
    o.color = v.color;
    o.mat = v.mat;
    o.paint = inst.paint;
    o.paint2 = inst.paint2;
    o.local_y = v.pos.y;
    return o;
}

@vertex
fn vs_shadow(v: VIn, inst: Inst) -> @builtin(position) vec4<f32> {
    let ps = pose(v, inst);
    return g.view_proj * vec4<f32>(ps.pos, 1.0);
}

@fragment
fn fs_shadow() {
}

fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let p = g.shadow_mat * vec4<f32>(world + n * 0.02, 1.0);
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let depth = p.z - 0.00015;
    let t = g.params2.z;
    var s = 0.0;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            s = s + textureSampleCompareLevel(shadow_tex, shadow_samp, uv + vec2<f32>(f32(i), f32(j)) * t, depth);
        }
    }
    let inside = select(0.0, 1.0, all(uv > vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)) && p.z < 1.0);
    return mix(1.0, s / 9.0, inside);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let frag = in.clip;
    // Ghosted after a respawn: screen-door dither.
    if (in.paint.w > 0.0) {
        let h = hash21(floor(frag.xy) + vec2<f32>(fract(g.cam_pos.w * 7.0) * 13.0, 0.0));
        if (h < in.paint.w * 0.7) {
            discard;
        }
    }
    let flags = u32(round(in.mat.w * 255.0));
    var albedo = decode_albedo(in.color.rgb);
    var rough = in.mat.y;
    let metal = in.mat.z;
    var emissive = vec3<f32>(0.0);
    var coat = 0.0;
    if ((flags & 1u) != 0u) {
        albedo = in.paint.rgb * (albedo.r * 1.25);
        coat = 1.0;
    } else if ((flags & 2u) != 0u) {
        albedo = in.paint2.rgb * (albedo.r * 1.25);
    }
    if ((flags & 4u) != 0u) {
        emissive = vec3<f32>(1.0, 0.95, 0.85) * 30.0;
    }
    if ((flags & 8u) != 0u) {
        emissive = vec3<f32>(1.0, 0.04, 0.02) * (2.5 + in.paint2.w * 30.0);
    }
    let n = normalize(in.nrm);
    let v = normalize(g.cam_pos.xyz - in.world);
    let shadow = mix(1.0, sun_shadow(in.world, n), g.sun_dir.w);
    // Ground occlusion: the underside and wheels sit in the bike's own shade.
    let ao = mix(0.5, 1.0, smoothstep(0.02, 0.5, in.local_y));
    var col = shade(albedo, n, v, rough, metal, shadow, ao);
    if (coat > 0.0) {
        // Clear coat: a second, sharp specular lobe and a sky reflection.
        let h = normalize(g.sun_dir.xyz + v);
        let ndv = max(dot(n, v), 0.0);
        let fc = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
        col = col + g.sun_color.rgb * pow(max(dot(n, h), 0.0), 300.0) * 2.5 * shadow;
        let r = reflect(-v, n);
        col = col + sky_color(vec3<f32>(r.x, abs(r.y), r.z), false) * fc * 0.6;
    }
    // Rim light so bikes pop against the road.
    let rim = pow(1.0 - max(dot(n, v), 0.0), 3.0);
    col = col + g.sky_horizon.rgb * rim * 0.25;
    col = col + emissive;
    col = apply_fog(col, in.world);
    return vec4<f32>(col * g.sun_color.w, 1.0);
}
