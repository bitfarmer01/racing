// Shared by every scene shader: frame globals, noise, sky, fog and lighting.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    shadow_mat: mat4x4<f32>,
    // xyz camera position, w time (s)
    cam_pos: vec4<f32>,
    // xyz direction towards the sun, w shadow strength
    sun_dir: vec4<f32>,
    // rgb sun radiance, w hdr output scale
    sun_color: vec4<f32>,
    // rgb, w fog density
    sky_zenith: vec4<f32>,
    // rgb, w cloud cover
    sky_horizon: vec4<f32>,
    // rgb, w water level
    ground_bounce: vec4<f32>,
    // rgb, w snow line
    water_color: vec4<f32>,
    grass: vec4<f32>,
    grass_dry: vec4<f32>,
    rock: vec4<f32>,
    // rgb sand, w track length
    sand: vec4<f32>,
    // x start lights lit (0..5, 6 = go), y lamp strength, z lamp spacing, w lamp lateral
    params: vec4<f32>,
    // x half width, y wind, z shadow texel size, w lamp skip end (s)
    params2: vec4<f32>,
    // Per bike: ground contact xyz and heading.
    contacts: array<vec4<f32>, 8>,
    // Contact-shadow strength per bike, four to a vec4.
    contact_w: array<vec4<f32>, 2>,
};

@group(0) @binding(0) var<uniform> g: Globals;

const PI: f32 = 3.14159265;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm3(p: vec2<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 3; i = i + 1) {
        s = s + vnoise(q) * a;
        q = q * 2.03 + vec2<f32>(1.7, 9.2);
        a = a * 0.5;
    }
    return s / 0.875;
}

fn fbm5(p: vec2<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 5; i = i + 1) {
        s = s + vnoise(q) * a;
        q = q * 2.03 + vec2<f32>(1.7, 9.2);
        a = a * 0.5;
    }
    return s / 0.96875;
}

fn decode_albedo(c: vec3<f32>) -> vec3<f32> {
    return c * c * (c * 0.305 + 0.682) + c * 0.0125;
}

// Sky radiance along a view direction. `detail` adds the sun disk and clouds.
fn sky_color(dir: vec3<f32>, detail: bool) -> vec3<f32> {
    let sun = g.sun_dir.xyz;
    let up = max(dir.y, 0.0);
    let h = pow(1.0 - up, 4.0);
    var col = mix(g.sky_zenith.rgb, g.sky_horizon.rgb, h);
    // Below the horizon: fade to a hazy ground tone.
    if (dir.y < 0.0) {
        col = mix(g.sky_horizon.rgb, g.ground_bounce.rgb * 1.6, clamp(-dir.y * 3.0, 0.0, 1.0));
    }
    let mu = dot(dir, sun);
    // Mie glow around the sun, strongest near the horizon.
    let glow = pow(max(mu, 0.0), 8.0) * 0.35 + pow(max(mu, 0.0), 64.0) * 0.9;
    col = col + g.sun_color.rgb * glow * (0.12 + 0.18 * h);
    if (detail) {
        if (dir.y > 0.0) {
            // Clouds: fbm on a plane, lit from the sun side.
            let t = g.cam_pos.w;
            let uv = dir.xz / (dir.y + 0.12) * 1.6 + vec2<f32>(t * 0.004, t * 0.0015);
            let n = fbm5(uv);
            let cover = g.sky_horizon.w;
            let c = smoothstep(1.0 - cover - 0.05, 1.0 - cover + 0.35, n);
            let edge = smoothstep(0.0, 0.35, dir.y);
            let shade = fbm3(uv * 2.0 + sun.xz * 0.3);
            let lit = g.sun_color.rgb * (0.05 + 0.1 * pow(max(mu, 0.0), 3.0)) * (1.2 - shade * 0.6);
            let base = mix(g.sky_horizon.rgb * 0.9, g.sky_zenith.rgb * 1.6, 0.4);
            col = mix(col, base * 0.5 + lit, c * edge * 0.92);
        }
        // Sun disk.
        let disk = smoothstep(0.99955, 0.99985, mu);
        col = col + g.sun_color.rgb * disk * 6.0;
    }
    return col;
}

// Aerial perspective: blend towards the haze colour with distance and height.
fn apply_fog(col: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let d = world - g.cam_pos.xyz;
    let dist = length(d);
    let dir = d / max(dist, 0.001);
    let density = g.sky_zenith.w;
    // Height falloff: thicker in valleys, thinner up high.
    let hf = exp(-max(world.y - g.ground_bounce.w, 0.0) * 0.0045);
    let amount = 1.0 - exp(-dist * density * (0.35 + 0.65 * hf));
    var haze = sky_color(vec3<f32>(dir.x, max(dir.y, 0.02), dir.z), false);
    let mu = max(dot(dir, g.sun_dir.xyz), 0.0);
    haze = haze + g.sun_color.rgb * pow(mu, 6.0) * 0.08;
    return mix(col, haze, clamp(amount, 0.0, 1.0));
}

// GGX specular + Lambert diffuse for the sun, hemispheric ambient and a
// sky-reflection term. Returns HDR radiance.
fn shade(albedo: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32, metal: f32, shadow: f32, ao: f32) -> vec3<f32> {
    let l = g.sun_dir.xyz;
    let h = normalize(l + v);
    let ndl = max(dot(n, l), 0.0);
    let ndv = max(dot(n, v), 0.001);
    let ndh = max(dot(n, h), 0.0);
    let a = max(rough * rough, 0.002);
    let a2 = a * a;
    let dd = ndh * ndh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let vis = 1.0 / ((ndl * (1.0 - k) + k) * (ndv * (1.0 - k) + k) * 4.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let vh = max(dot(v, h), 0.0);
    let f = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
    let spec = d * vis * f;
    let diff = (1.0 - metal) * albedo / PI;
    var col = (diff * (1.0 - f) + spec) * g.sun_color.rgb * ndl * shadow;
    // Ambient: sky from above, warm bounce from below.
    let sky_amb = mix(g.ground_bounce.rgb, g.sky_zenith.rgb * 1.15 + g.sky_horizon.rgb * 0.25, n.y * 0.5 + 0.5);
    col = col + diff * sky_amb * PI * 0.55 * ao;
    // Environment reflection.
    let r = reflect(-v, n);
    let fe = f0 + (max(vec3<f32>(1.0 - rough), f0) - f0) * pow(1.0 - ndv, 5.0);
    let env = sky_color(vec3<f32>(r.x, abs(r.y) * 0.8 + 0.02, r.z), false);
    let env_dim = mix(1.0, 0.25, rough);
    col = col + env * fe * env_dim * ao * mix(0.6, 1.0, shadow);
    return col;
}

// Soft dark patch where each bike meets the ground: a long ellipse under
// the bike and a tight spot under each tyre. Returns 0 (none) .. 1.
// Bikes are packed first, so the loop ends at the first empty slot, and
// nothing past CONTACT_RANGE (renderer.rs) has one.
const CONTACT_RANGE: f32 = 72.0;

fn contact_shadow(wp: vec3<f32>) -> f32 {
    var occ = 0.0;
    let to_cam = wp - g.cam_pos.xyz;
    if (dot(to_cam, to_cam) > CONTACT_RANGE * CONTACT_RANGE) {
        return 0.0;
    }
    for (var i = 0; i < 8; i = i + 1) {
        let w = g.contact_w[i / 4][i % 4];
        if (w <= 0.0) {
            break;
        }
        let c = g.contacts[i];
        let d = wp - c.xyz;
        if (abs(d.y) > 0.6 || dot(d.xz, d.xz) > 4.0) {
            continue;
        }
        let f = vec2<f32>(cos(c.w), sin(c.w));
        let along = dot(d.xz, f);
        let across = d.x * -f.y + d.z * f.x;
        let body = exp(-(along * along) / 0.7 - (across * across) / 0.06);
        let fa = along - 0.72;
        let ra = along + 0.70;
        let tyres = exp(-(fa * fa * 0.6 + across * across) / 0.01) + exp(-(ra * ra * 0.6 + across * across) / 0.012);
        occ = max(occ, clamp(body * 0.7 + tyres, 0.0, 1.0) * w * (1.0 - abs(d.y) / 0.6));
    }
    return occ;
}
