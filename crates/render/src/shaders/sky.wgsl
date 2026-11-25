// Full-screen sky at the far plane (reverse-Z depth 0). Prepended with common.wgsl.

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VOut {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var o: VOut;
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    o.ndc = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let p = g.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(p.xyz / p.w - g.cam_pos.xyz);
    let col = sky_color(dir, true);
    return vec4<f32>(col * g.sun_color.w, 1.0);
}
