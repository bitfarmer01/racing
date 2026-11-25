// Camera-facing particles: smoke and dust (alpha), sparks and flames
// (additive, stretched along velocity). Prepended with common.wgsl.

struct Inst {
    // xyz position, w size
    @location(1) pos: vec4<f32>,
    // linear rgb (premultiplied by alpha for the alpha pass), a alpha
    @location(2) color: vec4<f32>,
    // xyz velocity, w stretch seconds (0 = round billboard)
    @location(3) vel: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) seed: f32,
};

@vertex
fn vs_main(@location(0) corner: vec2<f32>, inst: Inst) -> VOut {
    let p = inst.pos.xyz;
    let to_cam = normalize(g.cam_pos.xyz - p);
    var right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), to_cam));
    var up = cross(to_cam, right);
    var world = p;
    if (inst.vel.w > 0.0) {
        let streak = inst.vel.xyz * inst.vel.w;
        let len = length(streak);
        let axis = streak / max(len, 0.0001);
        let side = normalize(cross(axis, to_cam));
        world = p + side * corner.x * inst.pos.w + axis * (corner.y * 0.5 - 0.5) * (len + inst.pos.w);
    } else {
        let a = inst.vel.x * 0.0 + p.x * 3.7 + p.z * 1.3;
        let c = cos(a);
        let s = sin(a);
        let r2 = right * c + up * s;
        let u2 = up * c - right * s;
        world = p + (r2 * corner.x + u2 * corner.y) * inst.pos.w;
    }
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(world, 1.0);
    o.uv = corner;
    o.color = inst.color;
    o.world = world;
    o.seed = fract(p.x * 0.13 + p.z * 0.71);
    return o;
}

@fragment
fn fs_alpha(in: VOut) -> @location(0) vec4<f32> {
    let r = length(in.uv);
    let n = vnoise(in.uv * 2.3 + vec2<f32>(in.seed * 17.0, g.cam_pos.w * 0.4));
    let a = in.color.a * (1.0 - smoothstep(0.35, 1.0, r + (n - 0.5) * 0.45));
    if (a <= 0.002) {
        discard;
    }
    // Smoke is lit by the sun and sky.
    let lit = g.sun_color.rgb * 0.06 * max(g.sun_dir.y + 0.3, 0.1) + g.sky_zenith.rgb * 0.6;
    var col = in.color.rgb * lit;
    col = apply_fog(col, in.world);
    return vec4<f32>(col * a * g.sun_color.w, a);
}

@fragment
fn fs_add(in: VOut) -> @location(0) vec4<f32> {
    let r = length(in.uv);
    let a = in.color.a * (1.0 - smoothstep(0.0, 1.0, r));
    return vec4<f32>(in.color.rgb * a * g.sun_color.w, 0.0);
}
