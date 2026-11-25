//! CPU-side mesh building: one vertex format for everything static, plus
//! helpers for the primitive shapes the procedural models are made of.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec2, Vec3};

/// 40-byte vertex shared by the world and bike pipelines.
///
/// `mat` is interpreted per pipeline: for the world it is
/// (material kind, roughness, metalness, extra); for bikes it is
/// (part id, roughness, metalness, flags).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub uv: [f32; 2],
    pub color: [u8; 4],
    pub mat: [u8; 4],
}

impl Vertex {
    pub const ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x2,
        3 => Unorm8x4,
        4 => Unorm8x4,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

/// World material kinds (the shader switches on these).
pub mod kind {
    pub const PLAIN: u8 = 0;
    pub const TERRAIN: u8 = 1;
    pub const ROAD: u8 = 2;
    pub const KERB: u8 = 3;
    pub const FOLIAGE: u8 = 4;
    pub const WATER: u8 = 5;
    pub const EMISSIVE: u8 = 6;
    pub const CROWD: u8 = 7;
    pub const START_LIGHT: u8 = 8;
    pub const CHECKER: u8 = 9;
    pub const BARRIER: u8 = 10;
    pub const LAMP_GLOW: u8 = 11;
}

/// Material for a builder primitive.
#[derive(Debug, Clone, Copy)]
pub struct Mat {
    pub color: [f32; 3],
    pub kind: u8,
    pub rough: f32,
    pub metal: f32,
    pub extra: f32,
}

impl Mat {
    pub const fn new(color: [f32; 3], rough: f32, metal: f32) -> Self {
        Self {
            color,
            kind: kind::PLAIN,
            rough,
            metal,
            extra: 0.0,
        }
    }

    pub const fn kind(mut self, k: u8) -> Self {
        self.kind = k;
        self
    }

    pub const fn extra(mut self, e: f32) -> Self {
        self.extra = e;
        self
    }

    fn packed(&self) -> ([u8; 4], [u8; 4]) {
        // Albedo is stored gamma-encoded for precision; the shader decodes.
        let c = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8;
        let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        (
            [c(self.color[0]), c(self.color[1]), c(self.color[2]), 255],
            [self.kind, u(self.rough), u(self.metal), u(self.extra)],
        )
    }
}

/// One cross-section of a [`MeshBuilder::loft`].
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub c: Vec3,
    pub w: f32,
    pub h: f32,
    pub square: f32,
}

impl Section {
    pub const fn new(c: Vec3, w: f32, h: f32, square: f32) -> Self {
        Self { c, w, h, square }
    }

    pub const fn round(c: Vec3, r: f32) -> Self {
        Self::new(c, r, r, 2.0)
    }
}

#[derive(Debug, Default, Clone)]
pub struct MeshBuilder {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Applied to every primitive added (model space -> parent space).
    pub transform: Mat4,
}

impl MeshBuilder {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            transform: Mat4::IDENTITY,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn vertex(&mut self, pos: Vec3, nrm: Vec3, uv: Vec2, m: &Mat) -> u32 {
        let p = self.transform.transform_point3(pos);
        let n = self.transform.transform_vector3(nrm).normalize_or_zero();
        let (color, mat) = m.packed();
        self.vertices.push(Vertex {
            pos: p.to_array(),
            nrm: n.to_array(),
            uv: uv.to_array(),
            color,
            mat,
        });
        (self.vertices.len() - 1) as u32
    }

    pub fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Quad a-b-c-d counter-clockwise when seen from the front.
    pub fn quad_idx(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.indices.extend_from_slice(&[a, b, c, a, c, d]);
    }

    /// Flat quad from four corners (CCW from the front), uv 0..1.
    pub fn quad(&mut self, p: [Vec3; 4], m: &Mat) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let uv = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let i: Vec<u32> = (0..4).map(|k| self.vertex(p[k], n, uv[k], m)).collect();
        self.quad_idx(i[0], i[1], i[2], i[3]);
    }

    /// Axis-aligned box centred at `c` with half extents `h` (in the current
    /// transform's space), optionally tapered: the top face scales by `taper`.
    pub fn cuboid(&mut self, c: Vec3, h: Vec3, taper: Vec2, m: &Mat) {
        let corner = |sx: f32, sy: f32, sz: f32| {
            let t = if sy > 0.0 { taper } else { Vec2::ONE };
            c + Vec3::new(sx * h.x * t.x, sy * h.y, sz * h.z * t.y)
        };
        let faces: [[(f32, f32, f32); 4]; 6] = [
            [(-1., -1., 1.), (1., -1., 1.), (1., 1., 1.), (-1., 1., 1.)],
            [
                (1., -1., -1.),
                (-1., -1., -1.),
                (-1., 1., -1.),
                (1., 1., -1.),
            ],
            [(1., -1., 1.), (1., -1., -1.), (1., 1., -1.), (1., 1., 1.)],
            [
                (-1., -1., -1.),
                (-1., -1., 1.),
                (-1., 1., 1.),
                (-1., 1., -1.),
            ],
            [(-1., 1., 1.), (1., 1., 1.), (1., 1., -1.), (-1., 1., -1.)],
            [
                (-1., -1., -1.),
                (1., -1., -1.),
                (1., -1., 1.),
                (-1., -1., 1.),
            ],
        ];
        for f in faces {
            let p = f.map(|(x, y, z)| corner(x, y, z));
            self.quad(p, m);
        }
    }

    pub fn cube(&mut self, c: Vec3, h: Vec3, m: &Mat) {
        self.cuboid(c, h, Vec2::ONE, m);
    }

    /// Box between two points with a square cross-section of half size `r`.
    pub fn beam(&mut self, a: Vec3, b: Vec3, r: f32, m: &Mat) {
        let d = b - a;
        let len = d.length();
        if len < 1e-5 {
            return;
        }
        let rot = Quat::from_rotation_arc(Vec3::Y, d / len);
        let saved = self.transform;
        self.transform = saved * Mat4::from_rotation_translation(rot, (a + b) * 0.5);
        self.cube(Vec3::ZERO, Vec3::new(r, len * 0.5, r), m);
        self.transform = saved;
    }

    /// Cylinder (or cone/frustum) along local +y from y0 to y1.
    #[allow(clippy::too_many_arguments)]
    pub fn cylinder(
        &mut self,
        base: Vec3,
        y0: f32,
        y1: f32,
        r0: f32,
        r1: f32,
        seg: u32,
        caps: bool,
        m: &Mat,
    ) {
        let slope = (r0 - r1) / (y1 - y0).max(1e-4);
        let start = self.vertices.len() as u32;
        for i in 0..=seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            let n = Vec3::new(c, slope, s).normalize();
            let u = i as f32 / seg as f32;
            self.vertex(
                base + Vec3::new(c * r0, y0, s * r0),
                n,
                Vec2::new(u, 0.0),
                m,
            );
            self.vertex(
                base + Vec3::new(c * r1, y1, s * r1),
                n,
                Vec2::new(u, 1.0),
                m,
            );
        }
        for i in 0..seg {
            let k = start + i * 2;
            self.quad_idx(k, k + 1, k + 3, k + 2);
        }
        if caps {
            for (y, r, up) in [(y0, r0, -1.0f32), (y1, r1, 1.0)] {
                if r < 1e-4 {
                    continue;
                }
                let centre = self.vertex(
                    base + Vec3::new(0.0, y, 0.0),
                    Vec3::Y * up,
                    Vec2::splat(0.5),
                    m,
                );
                let ring = self.vertices.len() as u32;
                for i in 0..seg {
                    let a = i as f32 / seg as f32 * std::f32::consts::TAU;
                    let (s, c) = a.sin_cos();
                    self.vertex(
                        base + Vec3::new(c * r, y, s * r),
                        Vec3::Y * up,
                        Vec2::new(c, s),
                        m,
                    );
                }
                for i in 0..seg {
                    let (a, b) = (ring + i, ring + (i + 1) % seg);
                    if up > 0.0 {
                        self.tri(centre, b, a);
                    } else {
                        self.tri(centre, a, b);
                    }
                }
            }
        }
    }

    /// Low-poly sphere (lat/long), squashable via `scale`.
    pub fn sphere(&mut self, c: Vec3, scale: Vec3, rings: u32, seg: u32, m: &Mat) {
        let start = self.vertices.len() as u32;
        for r in 0..=rings {
            let v = r as f32 / rings as f32;
            let phi = v * std::f32::consts::PI;
            for i in 0..=seg {
                let u = i as f32 / seg as f32;
                let th = u * std::f32::consts::TAU;
                let n = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
                let nn = (n / scale).normalize();
                self.vertex(c + n * scale, nn, Vec2::new(u, v), m);
            }
        }
        let w = seg + 1;
        for r in 0..rings {
            for i in 0..seg {
                let a = start + r * w + i;
                self.quad_idx(a, a + 1, a + w + 1, a + w);
            }
        }
    }

    /// Torus around local x axis (a tyre), centred at `c`.
    pub fn torus_x(&mut self, c: Vec3, major: f32, minor: f32, seg: u32, side: u32, m: &Mat) {
        let start = self.vertices.len() as u32;
        for i in 0..=seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            let ring_dir = Vec3::new(0.0, a.cos(), a.sin());
            for j in 0..=side {
                let b = j as f32 / side as f32 * std::f32::consts::TAU;
                let n = ring_dir * b.cos() + Vec3::X * b.sin();
                let p = c + ring_dir * major + n * minor;
                self.vertex(
                    p,
                    n,
                    Vec2::new(i as f32 / seg as f32, j as f32 / side as f32),
                    m,
                );
            }
        }
        let w = side + 1;
        for i in 0..seg {
            for j in 0..side {
                let a = start + i * w + j;
                self.quad_idx(a, a + w, a + w + 1, a + 1);
            }
        }
    }

    /// Smooth body through a run of cross-sections: each is a centre, a
    /// half width along `side`, a half height, and a squareness (2 = round,
    /// higher = boxier). Rings sit across the run's direction; normals come
    /// from the surface itself, so tapers and bends shade smoothly. Ends are
    /// capped.
    pub fn loft(&mut self, sections: &[Section], side: Vec3, seg: u32, m: &Mat) {
        let n = sections.len();
        if n < 2 {
            return;
        }
        let side = side.normalize();
        let ring = |k: usize| -> Vec<Vec3> {
            let a = sections[k.saturating_sub(1)].c;
            let b = sections[(k + 1).min(n - 1)].c;
            let t = (b - a).normalize_or_zero();
            let right = (side - t * side.dot(t)).normalize_or_zero();
            let up = t.cross(right).normalize_or_zero();
            let s = sections[k];
            (0..seg)
                .map(|i| {
                    let th = i as f32 / seg as f32 * std::f32::consts::TAU;
                    let (sn, cs) = th.sin_cos();
                    let e = 2.0 / s.square.max(1.0);
                    let x = cs.signum() * cs.abs().powf(e) * s.w;
                    let y = sn.signum() * sn.abs().powf(e) * s.h;
                    s.c + right * x + up * y
                })
                .collect()
        };
        let grid: Vec<Vec<Vec3>> = (0..n).map(ring).collect();
        let seg_u = seg as usize;
        let start = self.vertices.len() as u32;
        for k in 0..n {
            for i in 0..=seg_u {
                let ii = i % seg_u;
                let du = grid[k][(ii + 1) % seg_u] - grid[k][(ii + seg_u - 1) % seg_u];
                let dv = grid[(k + 1).min(n - 1)][ii] - grid[k.saturating_sub(1)][ii];
                let mut nrm = dv.cross(du).normalize_or_zero();
                // Point the normal away from the section centre.
                if nrm.dot(grid[k][ii] - sections[k].c) < 0.0 {
                    nrm = -nrm;
                }
                let uv = Vec2::new(i as f32 / seg as f32, k as f32 / (n - 1) as f32);
                self.vertex(grid[k][ii], nrm, uv, m);
            }
        }
        let w = seg + 1;
        for k in 0..(n as u32 - 1) {
            for i in 0..seg {
                let a = start + k * w + i;
                self.quad_idx(a, a + 1, a + w + 1, a + w);
            }
        }
        for (k, out) in [(0usize, -1.0f32), (n - 1, 1.0)] {
            let a = sections[k.saturating_sub(1)].c;
            let b = sections[(k + 1).min(n - 1)].c;
            let t = (b - a).normalize_or_zero() * out;
            let centre = self.vertex(sections[k].c, t, Vec2::splat(0.5), m);
            let first = self.vertices.len() as u32;
            for p in &grid[k] {
                self.vertex(*p, t, Vec2::ZERO, m);
            }
            for i in 0..seg {
                self.tri(centre, first + i, first + (i + 1) % seg);
            }
        }
    }

    /// Round limb from `a` to `b`, radius `ra` to `rb`, with a ball joint at
    /// each end so bent limbs stay closed.
    pub fn limb(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, m: &Mat) {
        let d = (b - a).normalize_or_zero();
        let side = if d.x.abs() > 0.8 { Vec3::Z } else { Vec3::X };
        self.loft(
            &[
                Section::round(a, ra),
                Section::round(a.lerp(b, 0.5), (ra + rb) * 0.5 * 1.04),
                Section::round(b, rb),
            ],
            side,
            8,
            m,
        );
        self.sphere(a, Vec3::splat(ra), 4, 8, m);
        self.sphere(b, Vec3::splat(rb), 4, 8, m);
    }

    pub fn append(&mut self, other: &MeshBuilder) {
        let base = self.vertices.len() as u32;
        for v in &other.vertices {
            let mut v = *v;
            let p = self.transform.transform_point3(Vec3::from(v.pos));
            let n = self
                .transform
                .transform_vector3(Vec3::from(v.nrm))
                .normalize_or_zero();
            v.pos = p.to_array();
            v.nrm = n.to_array();
            self.vertices.push(v);
        }
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for v in &self.vertices {
            let p = Vec3::from(v.pos);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (lo, hi)
    }

    /// Recomputes smooth normals from the triangles (for terrain-like meshes).
    pub fn recompute_normals(&mut self) {
        let mut acc = vec![Vec3::ZERO; self.vertices.len()];
        for t in self.indices.chunks_exact(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let pa = Vec3::from(self.vertices[a].pos);
            let n = (Vec3::from(self.vertices[b].pos) - pa)
                .cross(Vec3::from(self.vertices[c].pos) - pa);
            acc[a] += n;
            acc[b] += n;
            acc[c] += n;
        }
        for (v, n) in self.vertices.iter_mut().zip(acc) {
            v.nrm = n.normalize_or(Vec3::Y).to_array();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_faces_point_outward() {
        let mut b = MeshBuilder::new();
        b.cube(Vec3::ZERO, Vec3::ONE, &Mat::new([1.0; 3], 0.5, 0.0));
        assert_eq!(b.indices.len(), 36);
        for t in b.indices.chunks_exact(3) {
            let p: Vec<Vec3> = t
                .iter()
                .map(|&i| Vec3::from(b.vertices[i as usize].pos))
                .collect();
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            let centre = (p[0] + p[1] + p[2]) / 3.0;
            assert!(n.dot(centre) > 0.0, "triangle winds inward");
        }
    }

    #[test]
    fn sphere_and_torus_wind_outward() {
        let mut b = MeshBuilder::new();
        b.sphere(Vec3::ZERO, Vec3::ONE, 6, 8, &Mat::new([1.0; 3], 0.5, 0.0));
        let mut t = MeshBuilder::new();
        t.torus_x(Vec3::ZERO, 1.0, 0.3, 12, 6, &Mat::new([1.0; 3], 0.5, 0.0));
        for (mesh, check) in [(&b, 0), (&t, 1)] {
            let mut outward = 0;
            let mut total = 0;
            for tri in mesh.indices.chunks_exact(3) {
                let p: Vec<Vec3> = tri
                    .iter()
                    .map(|&i| Vec3::from(mesh.vertices[i as usize].pos))
                    .collect();
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                if n.length() < 1e-6 {
                    continue;
                }
                let vn = Vec3::from(mesh.vertices[tri[0] as usize].nrm);
                total += 1;
                if n.dot(vn) > 0.0 {
                    outward += 1;
                }
            }
            assert!(outward * 10 >= total * 9, "mesh {check}: {outward}/{total}");
        }
    }
}
