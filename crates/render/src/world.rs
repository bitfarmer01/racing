//! CPU-side world geometry for one circuit: terrain tiles, the road ribbon
//! with verges, kerbs and barriers, the water plane and prop instances.
//!
//! Everything static lands in one vertex buffer with absolute u32 indices
//! (WebGL2 has no base-vertex draws); a [`Chunk`] is an index range with
//! its bounds, culled per frame.

use glam::{Vec2, Vec3};

use game::noise::smoothstep;
use game::scenery::{self, Prop, PropKind};
use game::theme::TreeStyle;
use game::track::{Track, RUNOFF};

use crate::mesh::{kind, Mat, MeshBuilder};
use crate::models;

#[derive(Debug, Clone, Copy)]
pub struct Chunk {
    pub first: u32,
    pub count: u32,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// Prop instance: position + yaw, then (scale, tint, wind phase, 0).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PropInstance {
    pub pos_yaw: [f32; 4],
    pub params: [f32; 4],
}

/// One prop model and its instances bucketed by grid cell.
#[derive(Debug, Clone)]
pub struct PropModel {
    pub mesh: Chunk,
    /// Cheaper mesh drawn past `lod_distance` (and for the sun's shadow).
    pub lod: Option<Chunk>,
    pub lod_distance: f32,
    /// Model-space bounding radius.
    pub radius: f32,
    pub draw_distance: f32,
    pub casts_shadow: bool,
    pub cells: Vec<PropCell>,
}

#[derive(Debug, Clone)]
pub struct PropCell {
    pub lo: Vec3,
    pub hi: Vec3,
    pub instances: Vec<PropInstance>,
}

pub struct WorldGeometry {
    pub mesh: MeshBuilder,
    pub terrain: Vec<Chunk>,
    pub road: Vec<Chunk>,
    pub water: Option<Chunk>,
    pub props: Vec<PropModel>,
    pub bike: Chunk,
    /// Road traffic, one model per car kind.
    pub cars: [Chunk; 4],
}

const TERRAIN_STEP: f32 = 5.0;
const TERRAIN_MARGIN: f32 = 420.0;
const TERRAIN_EXTENT: f32 = 4200.0;
const TILE: usize = 40;
const ROAD_CHUNK: usize = 48;
const PROP_CELL: f32 = 120.0;

fn finish_chunk(mesh: &MeshBuilder, first: usize) -> Chunk {
    let idx = &mesh.indices[first..];
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for &i in idx {
        let p = Vec3::from(mesh.vertices[i as usize].pos);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    Chunk {
        first: first as u32,
        count: idx.len() as u32,
        lo,
        hi,
    }
}

/// Grid coordinates: fine spacing over the circuit, growing geometrically
/// out to the horizon.
fn axis(lo: f32, hi: f32, centre: f32) -> Vec<f32> {
    let a = lo - TERRAIN_MARGIN;
    let b = hi + TERRAIN_MARGIN;
    let n = ((b - a) / TERRAIN_STEP).ceil() as usize;
    let step = (b - a) / n as f32;
    let mut core: Vec<f32> = (0..=n).map(|i| a + i as f32 * step).collect();
    let mut left = Vec::new();
    let (mut x, mut s) = (a, step);
    while x > centre - TERRAIN_EXTENT {
        s *= 1.14;
        x -= s;
        left.push(x);
    }
    let mut right = Vec::new();
    let (mut x, mut s) = (b, step);
    while x < centre + TERRAIN_EXTENT {
        s *= 1.14;
        x += s;
        right.push(x);
    }
    left.reverse();
    left.append(&mut core);
    left.append(&mut right);
    left
}

impl WorldGeometry {
    pub fn build(track: &Track) -> Self {
        let mut mesh = MeshBuilder::new();
        let terrain = build_terrain(&mut mesh, track);
        let road = build_road(&mut mesh, track);
        let water = build_water(&mut mesh, track);
        let props = build_props(&mut mesh, track);
        let first = mesh.indices.len();
        let base = mesh.vertices.len() as u32;
        let bike = models::bike();
        mesh.vertices.extend_from_slice(&bike.vertices);
        mesh.indices.extend(bike.indices.iter().map(|i| i + base));
        let bike = finish_chunk(&mesh, first);
        let cars = models::cars().map(|car| {
            let first = mesh.indices.len();
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend_from_slice(&car.vertices);
            mesh.indices.extend(car.indices.iter().map(|i| i + base));
            finish_chunk(&mesh, first)
        });
        Self {
            mesh,
            terrain,
            road,
            water,
            props,
            bike,
            cars,
        }
    }
}

fn build_terrain(mesh: &mut MeshBuilder, track: &Track) -> Vec<Chunk> {
    let (lo, hi) = track.bounds();
    let c = (lo + hi) * 0.5;
    let xs = axis(lo.x, hi.x, c.x);
    let zs = axis(lo.y, hi.y, c.y);
    let (nx, nz) = (xs.len(), zs.len());
    let theme = track.def.theme;
    let base = mesh.vertices.len() as u32;
    let near_lo = lo - Vec2::splat(TERRAIN_MARGIN);
    let near_hi = hi + Vec2::splat(TERRAIN_MARGIN);
    let mat = Mat::new([1.0, 1.0, 1.0], 0.92, 0.0).kind(kind::TERRAIN);
    let mut grid = MeshBuilder::new();
    for &z in &zs {
        for &x in &xs {
            let p = Vec2::new(x, z);
            let inside = p.cmpge(near_lo).all() && p.cmple(near_hi).all();
            let (h, near, gravel) = if inside {
                match track.nearest_index(p, 70.0) {
                    Some(i) => {
                        let tp = track.locate_near(p, i, 3);
                        let edge = tp.lateral.abs() - track.half_width;
                        let k = track.frame_at(tp.s).curvature;
                        let outer = if tp.lateral * k < 0.0 { 1.0 } else { 0.35 };
                        let trap = smoothstep(0.007, 0.016, k.abs())
                            * (1.0 - smoothstep(RUNOFF - 1.0, RUNOFF + 0.5, edge))
                            * smoothstep(0.6, 2.0, edge)
                            * outer;
                        (
                            track.terrain_vertex_height(x, z),
                            (1.0 - smoothstep(0.0, 16.0, edge)).min(0.98),
                            trap,
                        )
                    }
                    None => (theme.natural_height(x, z), 0.0, 0.0),
                }
            } else {
                (theme.natural_height(x, z), 0.0, 0.0)
            };
            grid.vertex(Vec3::new(x, h, z), Vec3::Y, Vec2::new(x, z), &mat);
            // Raw channels (not colour): road proximity and gravel trap.
            let v = grid.vertices.last_mut().expect("just pushed");
            v.color = [(near * 255.0) as u8, (gravel * 255.0) as u8, 0, 255];
        }
    }
    // Indices tile by tile so each tile is one contiguous range.
    for tz in (0..nz - 1).step_by(TILE) {
        for tx in (0..nx - 1).step_by(TILE) {
            for z in tz..(tz + TILE).min(nz - 1) {
                for x in tx..(tx + TILE).min(nx - 1) {
                    let a = (z * nx + x) as u32;
                    let b = a + 1;
                    let d = a + nx as u32;
                    let cc = d + 1;
                    grid.quad_idx(a, d, cc, b);
                }
            }
        }
    }
    grid.recompute_normals();
    mesh.vertices.extend_from_slice(&grid.vertices);
    let mut chunks = Vec::new();
    let mut cursor = 0;
    for tz in (0..nz - 1).step_by(TILE) {
        for tx in (0..nx - 1).step_by(TILE) {
            let quads = ((tz + TILE).min(nz - 1) - tz) * ((tx + TILE).min(nx - 1) - tx);
            let first = mesh.indices.len();
            mesh.indices.extend(
                grid.indices[cursor..cursor + quads * 6]
                    .iter()
                    .map(|i| i + base),
            );
            cursor += quads * 6;
            chunks.push(finish_chunk(mesh, first));
        }
    }
    chunks
}

fn build_road(mesh: &mut MeshBuilder, track: &Track) -> Vec<Chunk> {
    let hw = track.half_width;
    let n = track.len();
    let mut chunks = Vec::new();
    let verge = Mat::new([1.0, 1.0, 1.0], 0.92, 0.0).kind(kind::TERRAIN);
    let asphalt = Mat::new([0.06, 0.06, 0.065], 0.62, 0.0).kind(kind::ROAD);
    let kerb = Mat::new([1.0, 1.0, 1.0], 0.5, 0.0).kind(kind::KERB);
    let steel = Mat::new([0.6, 0.62, 0.66], 0.32, 0.9).kind(kind::BARRIER);
    let post = Mat::new([0.25, 0.25, 0.27], 0.6, 0.6);
    let barrier = hw + RUNOFF;

    let mut i0 = 0;
    while i0 < n {
        let i1 = (i0 + ROAD_CHUNK).min(n);
        let first = mesh.indices.len();
        let mut b = MeshBuilder::new();
        let kerb_k = |i: usize| -> f32 {
            let k = track.sample(i % n).curvature.abs();
            smoothstep(0.0055, 0.009, k)
        };
        for i in i0..=i1 {
            let s = track.sample(i % n);
            let dist = if i == n { track.length } else { s.dist };
            let right = Vec3::new(s.right.x, 0.0, s.right.y);
            let at = |lat: f32, dy: f32| {
                let mut p = s.pos + right * lat;
                p.y = s.surface_y(lat) + dy;
                p
            };
            let ground = |lat: f32, dy: f32| {
                let p = s.pos + right * lat;
                let tp = game::track::TrackPos {
                    index: i % n,
                    s: s.dist,
                    lateral: lat,
                };
                Vec3::new(p.x, track.ground_at(p.x, p.z, &tp) + dy, p.z)
            };
            let up = Vec3::new(
                -right.x * s.bank.sin(),
                s.bank.cos(),
                -right.z * s.bank.sin(),
            )
            .normalize();
            let mut near_road = verge;
            near_road.color = [1.0, 0.0, 0.0];
            // Row layout per sample: 0..2 left verge, 2..4 asphalt, 4..6 right verge.
            let lv_out = ground(-(hw + 3.4), -0.03);
            let lv_in = at(-(hw + 0.15), -0.025);
            let rv_in = at(hw + 0.15, -0.025);
            let rv_out = ground(hw + 3.4, -0.03);
            b.vertex(lv_out, Vec3::Y, Vec2::new(-(hw + 3.4), dist), &near_road);
            b.vertex(lv_in, up, Vec2::new(-(hw + 0.15), dist), &near_road);
            b.vertex(
                at(-(hw + 0.15), 0.0),
                up,
                Vec2::new(-(hw + 0.15), dist),
                &asphalt,
            );
            b.vertex(at(hw + 0.15, 0.0), up, Vec2::new(hw + 0.15, dist), &asphalt);
            b.vertex(rv_in, up, Vec2::new(hw + 0.15, dist), &near_road);
            b.vertex(rv_out, Vec3::Y, Vec2::new(hw + 3.4, dist), &near_road);
        }
        let rows = i1 - i0;
        for r in 0..rows as u32 {
            let a = r * 6;
            let c = a + 6;
            for k in [0u32, 2, 4] {
                b.quad_idx(a + k, c + k, c + k + 1, a + k + 1);
            }
        }
        // Raw channels for the verge (proximity = 1).
        for v in &mut b.vertices {
            if v.mat[0] == kind::TERRAIN {
                v.color = [255, 0, 0, 255];
            }
        }

        // Kerbs: raised striped strips on both edges through corners.
        let mut run: Vec<usize> = Vec::new();
        let flush = |b: &mut MeshBuilder, run: &mut Vec<usize>| {
            if run.len() >= 2 {
                for side in [-1.0f32, 1.0] {
                    let start = b.vertices.len() as u32;
                    for &i in run.iter() {
                        let s = track.sample(i % n);
                        let dist = if i == n { track.length } else { s.dist };
                        let w = kerb_k(i);
                        let right = Vec3::new(s.right.x, 0.0, s.right.y);
                        let up = Vec3::new(
                            -right.x * s.bank.sin(),
                            s.bank.cos(),
                            -right.z * s.bank.sin(),
                        );
                        let lat_in = side * (hw - 0.05);
                        let lat_out = side * (hw + 1.15 * w.max(0.05));
                        let mut pin = s.pos + right * lat_in;
                        pin.y = s.surface_y(lat_in) + 0.025;
                        let mut pout = s.pos + right * lat_out;
                        pout.y = s.surface_y(lat_out) + 0.07 * w;
                        let mut m = kerb;
                        m.extra = w;
                        b.vertex(pin, up, Vec2::new(lat_in, dist), &m);
                        b.vertex(pout, up, Vec2::new(lat_out, dist), &m);
                    }
                    for k in 0..(run.len() as u32 - 1) {
                        let a = start + k * 2;
                        if side > 0.0 {
                            b.quad_idx(a, a + 2, a + 3, a + 1);
                        } else {
                            b.quad_idx(a, a + 1, a + 3, a + 2);
                        }
                    }
                }
            }
            run.clear();
        };
        for i in i0..=i1 {
            if kerb_k(i) > 0.0 {
                run.push(i);
            } else {
                if !run.is_empty() {
                    run.push(i);
                }
                flush(&mut b, &mut run);
            }
        }
        flush(&mut b, &mut run);

        // Armco barriers both sides: a steel rail on a dark backing.
        for side in [-1.0f32, 1.0] {
            let start = b.vertices.len() as u32;
            for i in i0..=i1 {
                let s = track.sample(i % n);
                let dist = if i == n { track.length } else { s.dist };
                let right = Vec3::new(s.right.x, 0.0, s.right.y);
                let lat = side * barrier;
                let p = s.pos + right * lat;
                let tp = game::track::TrackPos {
                    index: i % n,
                    s: s.dist,
                    lateral: lat,
                };
                let g = track.ground_at(p.x, p.z, &tp);
                let inward = -right * side;
                let m = steel;
                b.vertex(
                    Vec3::new(p.x, g + 0.45, p.z),
                    inward,
                    Vec2::new(0.0, dist),
                    &m,
                );
                b.vertex(
                    Vec3::new(p.x, g + 0.95, p.z),
                    inward,
                    Vec2::new(1.0, dist),
                    &m,
                );
                let back = p - inward * 0.12;
                b.vertex(
                    Vec3::new(back.x, g + 0.98, back.z),
                    Vec3::Y,
                    Vec2::new(1.0, dist),
                    &m,
                );
                b.vertex(
                    Vec3::new(back.x, g - 0.2, back.z),
                    -inward,
                    Vec2::new(0.0, dist),
                    &post,
                );
            }
            for k in 0..(i1 - i0) as u32 {
                let a = start + k * 4;
                let c = a + 4;
                if side > 0.0 {
                    b.quad_idx(a, a + 1, c + 1, c);
                    b.quad_idx(a + 1, a + 2, c + 2, c + 1);
                    b.quad_idx(a + 2, a + 3, c + 3, c + 2);
                } else {
                    b.quad_idx(a, c, c + 1, a + 1);
                    b.quad_idx(a + 1, c + 1, c + 2, a + 2);
                    b.quad_idx(a + 2, c + 2, c + 3, a + 3);
                }
            }
            // Posts every 4 m.
            for i in (i0..i1).step_by(2) {
                let s = track.sample(i % n);
                let right = Vec3::new(s.right.x, 0.0, s.right.y);
                let lat = side * (barrier + 0.14);
                let p = s.pos + right * lat;
                let g = track.ground_height(p.x, p.z);
                b.cube(
                    Vec3::new(p.x, g + 0.4, p.z),
                    Vec3::new(0.05, 0.5, 0.05),
                    &post,
                );
            }
        }

        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend_from_slice(&b.vertices);
        mesh.indices.extend(b.indices.iter().map(|i| i + base));
        chunks.push(finish_chunk(mesh, first));
        i0 = i1;
    }
    chunks
}

fn build_water(mesh: &mut MeshBuilder, track: &Track) -> Option<Chunk> {
    let theme = track.def.theme;
    let (lo, hi) = track.bounds();
    let c = (lo + hi) * 0.5;
    let first = mesh.indices.len();
    let m = Mat::new(theme.water_color, 0.05, 0.0).kind(kind::WATER);
    let n = 24;
    let ext = TERRAIN_EXTENT + 600.0;
    let mut b = MeshBuilder::new();
    for j in 0..=n {
        for i in 0..=n {
            let x = c.x - ext + 2.0 * ext * i as f32 / n as f32;
            let z = c.y - ext + 2.0 * ext * j as f32 / n as f32;
            b.vertex(
                Vec3::new(x, theme.water_level, z),
                Vec3::Y,
                Vec2::new(x, z),
                &m,
            );
        }
    }
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * (n as u32 + 1) + i;
            let d = a + n as u32 + 1;
            b.quad_idx(a, d, d + 1, a + 1);
        }
    }
    // Skip if the water never shows above ground.
    let visible = (0..400).any(|k| {
        let a = k as f32 * 0.7;
        let r = 300.0 + k as f32 * 9.0;
        theme.natural_height(c.x + a.cos() * r, c.y + a.sin() * r) < theme.water_level
    });
    if !visible {
        return None;
    }
    let base = mesh.vertices.len() as u32;
    mesh.vertices.extend_from_slice(&b.vertices);
    mesh.indices.extend(b.indices.iter().map(|i| i + base));
    Some(finish_chunk(mesh, first))
}

fn variant(kind: PropKind) -> usize {
    match kind {
        PropKind::Tree(TreeStyle::Pine) => 0,
        PropKind::Tree(TreeStyle::Broadleaf) => 1,
        PropKind::Tree(TreeStyle::Palm) => 2,
        PropKind::Tree(TreeStyle::Cactus) => 3,
        PropKind::Rock => 4,
        PropKind::Lamp => 5,
        PropKind::Grandstand => 6,
        PropKind::Gantry => 7,
        PropKind::Marshal => 8,
    }
}

/// Beyond this, trees and rocks switch to their low-detail mesh.
pub const PROP_LOD_DISTANCE: f32 = 110.0;

fn build_props(mesh: &mut MeshBuilder, track: &Track) -> Vec<PropModel> {
    let theme = track.def.theme;
    let models: [(MeshBuilder, Option<MeshBuilder>, f32, bool); 9] = [
        (models::pine(0.5), Some(models::pine_lod(0.5)), 1500.0, true),
        (
            models::broadleaf(0.5),
            Some(models::broadleaf_lod(0.5)),
            1500.0,
            true,
        ),
        (models::palm(0.5), Some(models::palm_lod(0.5)), 1400.0, true),
        (
            models::cactus(0.5),
            Some(models::cactus_lod(0.5)),
            900.0,
            true,
        ),
        (
            models::rock(0.5, theme.rock),
            Some(models::rock_lod(0.5, theme.rock)),
            700.0,
            true,
        ),
        (models::lamp(), None, 1600.0, true),
        (models::grandstand(0.3), None, 2500.0, true),
        (models::gantry(track.half_width), None, 2500.0, true),
        (models::marshal(0.5), None, 1200.0, true),
    ];
    let mut out = Vec::new();
    let append = |mesh: &mut MeshBuilder, m: &MeshBuilder| {
        let first = mesh.indices.len();
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend_from_slice(&m.vertices);
        mesh.indices.extend(m.indices.iter().map(|i| i + base));
        finish_chunk(mesh, first)
    };
    for (m, lod, dist, shadow) in &models {
        let chunk = append(mesh, m);
        let lod = lod.as_ref().map(|l| append(mesh, l));
        let radius = chunk.lo.abs().max(chunk.hi.abs()).length();
        out.push(PropModel {
            mesh: chunk,
            lod,
            lod_distance: PROP_LOD_DISTANCE,
            radius,
            draw_distance: *dist,
            casts_shadow: *shadow,
            cells: Vec::new(),
        });
    }
    let props: Vec<Prop> = scenery::generate(track);
    let mut buckets: Vec<std::collections::BTreeMap<(i32, i32), Vec<PropInstance>>> =
        vec![Default::default(); out.len()];
    for p in &props {
        let v = variant(p.kind);
        let key = (
            (p.pos.x / PROP_CELL).floor() as i32,
            (p.pos.z / PROP_CELL).floor() as i32,
        );
        let scale = if matches!(p.kind, PropKind::Gantry) {
            1.0
        } else {
            p.scale
        };
        buckets[v].entry(key).or_default().push(PropInstance {
            pos_yaw: [p.pos.x, p.pos.y, p.pos.z, p.yaw],
            params: [
                scale,
                p.tint,
                (p.pos.x * 0.37 + p.pos.z * 0.21) % std::f32::consts::TAU,
                0.0,
            ],
        });
    }
    for (model, cells) in out.iter_mut().zip(buckets) {
        for (_, instances) in cells {
            let mut lo = Vec3::splat(f32::MAX);
            let mut hi = Vec3::splat(f32::MIN);
            for inst in &instances {
                let p = Vec3::new(inst.pos_yaw[0], inst.pos_yaw[1], inst.pos_yaw[2]);
                let r = model.radius * inst.params[0];
                lo = lo.min(p - Vec3::splat(r));
                hi = hi.max(p + Vec3::splat(r));
            }
            model.cells.push(PropCell { lo, hi, instances });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use game::tracks::catalog;

    #[test]
    fn world_geometry_is_sane() {
        for def in catalog() {
            let t = Track::build(*def);
            let w = WorldGeometry::build(&t);
            let nv = w.mesh.vertices.len() as u32;
            assert!(w.mesh.indices.iter().all(|&i| i < nv));
            assert!(!w.terrain.is_empty() && !w.road.is_empty());
            let instances: usize = w
                .props
                .iter()
                .flat_map(|p| &p.cells)
                .map(|c| c.instances.len())
                .sum();
            assert!(instances > 300);
            // Road chunks cover the whole lap.
            let road_tris: u32 = w.road.iter().map(|c| c.count).sum();
            assert!(road_tris as usize > t.len() * 18);
            eprintln!(
                "{}: {} verts, {} tris, {} props, water {}",
                def.name,
                nv,
                w.mesh.indices.len() / 3,
                instances,
                w.water.is_some()
            );
        }
    }
}
