//! Trackside dressing: forests, rocks, street lamps, grandstands and the
//! start gantry. Deterministic per circuit; the renderer instances meshes
//! from this list.

use glam::{Vec2, Vec3};
use sim::rng::XorShift64;

use crate::noise::{fbm, smoothstep};
use crate::theme::TreeStyle;
use crate::track::{Track, RUNOFF};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropKind {
    Tree(TreeStyle),
    Rock,
    Lamp,
    Grandstand,
    Gantry,
    Marshal,
}

#[derive(Debug, Clone, Copy)]
pub struct Prop {
    pub kind: PropKind,
    pub pos: Vec3,
    /// Yaw: the prop's local +x points along (cos yaw, sin yaw).
    pub yaw: f32,
    pub scale: f32,
    /// Free per-prop variation in [0, 1).
    pub tint: f32,
}

pub fn generate(track: &Track) -> Vec<Prop> {
    let theme = track.def.theme;
    let mut rng = XorShift64::new(theme.seed as u64 ^ 0x5ce4e);
    let mut props = Vec::with_capacity(8_000);
    let hw = track.half_width;
    let barrier = hw + RUNOFF;

    // Start/finish gantry spans the road at s = 0.
    let f0 = track.frame_at(0.0);
    props.push(Prop {
        kind: PropKind::Gantry,
        pos: f0.pos,
        yaw: f0.heading(),
        scale: hw,
        tint: 0.0,
    });
    // Grandstands along the start straight, outside the barrier.
    for k in 0..5 {
        let s = track.wrap_s(-90.0 + k as f32 * 36.0);
        let side = if theme.landform == crate::theme::Landform::Coast {
            -1.0
        } else {
            1.0
        };
        let lat = side * (barrier + 7.0);
        let f = track.frame_at(s);
        let p = track.point_at(s, lat);
        let y = track.ground_height(p.x, p.z).max(f.pos.y - 1.0);
        props.push(Prop {
            kind: PropKind::Grandstand,
            pos: Vec3::new(p.x, y, p.z),
            yaw: f.heading()
                + if side > 0.0 {
                    0.0
                } else {
                    std::f32::consts::PI
                },
            scale: 1.0,
            tint: rng.next_f32(),
        });
    }

    // Lamps every 40 m, alternating sides, just outside the barrier.
    if theme.lamps {
        let count = (track.length / 40.0) as usize;
        for k in 0..count {
            let s = k as f32 * track.length / count as f32;
            if s < 60.0 || s > track.length - 120.0 {
                continue; // the grandstands light the straight
            }
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let f = track.frame_at(s);
            let p = track.point_at(s, side * (barrier + 1.2));
            props.push(Prop {
                kind: PropKind::Lamp,
                pos: Vec3::new(p.x, track.ground_height(p.x, p.z), p.z),
                yaw: f.heading()
                    + if side > 0.0 {
                        std::f32::consts::PI
                    } else {
                        0.0
                    },
                scale: 1.0,
                tint: 0.0,
            });
        }
    }

    // Marshals' posts at the sharpest corners.
    let mut last = -1000.0;
    for i in 0..track.len() {
        let sm = track.sample(i);
        if sm.curvature.abs() > 0.012 && sm.dist - last > 150.0 {
            last = sm.dist;
            let side = -sm.curvature.signum();
            let p = track.point_at(sm.dist, side * (barrier + 3.0));
            props.push(Prop {
                kind: PropKind::Marshal,
                pos: Vec3::new(p.x, track.ground_height(p.x, p.z), p.z),
                yaw: sm.heading(),
                scale: 1.0,
                tint: rng.next_f32(),
            });
        }
    }

    // Vegetation and rocks on a jittered grid around the circuit.
    let (lo, hi) = track.bounds();
    let margin = 520.0;
    let cell = 8.5;
    let nx = ((hi.x - lo.x + 2.0 * margin) / cell) as i32;
    let nz = ((hi.y - lo.y + 2.0 * margin) / cell) as i32;
    let seed = theme.seed;
    for gz in 0..nz {
        for gx in 0..nx {
            let x = lo.x - margin + (gx as f32 + rng.next_f32()) * cell;
            let z = lo.y - margin + (gz as f32 + rng.next_f32()) * cell;
            let p = Vec2::new(x, z);
            let near = track.nearest_index(p, 70.0);
            let edge = match near {
                Some(i) => {
                    let s = track.sample(i);
                    (p - Vec2::new(s.pos.x, s.pos.z)).length() - hw
                }
                None => 70.0,
            };
            if edge < RUNOFF + 3.5 {
                continue;
            }
            let h = track.ground_height(x, z);
            if h < theme.water_level + 1.2 || h > theme.snow_line - 10.0 {
                continue;
            }
            // Forest patches, denser near the road to sell speed, thinning far out.
            let forest = fbm(x * 0.006, z * 0.006, 3, seed ^ 0x7ee) * 0.5 + 0.5;
            let near_boost = 1.0 - smoothstep(20.0, 120.0, edge);
            let far_fade = 1.0 - smoothstep(260.0, 520.0, edge);
            let density = (smoothstep(0.42, 0.7, forest) * 0.75 + near_boost * 0.22)
                * far_fade
                * theme.tree_density;
            let roll = rng.next_f32();
            if roll < density * 0.75 {
                // Slope check: skip cliff faces.
                let hx = track.ground_height(x + 2.0, z);
                let hz = track.ground_height(x, z + 2.0);
                if ((hx - h).abs() + (hz - h).abs()) > 2.6 {
                    continue;
                }
                let style = if rng.next_f32() < 0.72 {
                    theme.trees[0]
                } else {
                    theme.trees[1]
                };
                // Palms only near the shore.
                let style = if style == TreeStyle::Palm && h > theme.water_level + 22.0 {
                    theme.trees[0]
                } else {
                    style
                };
                props.push(Prop {
                    kind: PropKind::Tree(style),
                    pos: Vec3::new(x, h - 0.2, z),
                    yaw: rng.next_f32() * std::f32::consts::TAU,
                    scale: rng.next_f32_in(0.75, 1.35),
                    tint: rng.next_f32(),
                });
            } else if roll > 0.988 {
                props.push(Prop {
                    kind: PropKind::Rock,
                    pos: Vec3::new(x, h - 0.4, z),
                    yaw: rng.next_f32() * std::f32::consts::TAU,
                    scale: rng.next_f32_in(0.7, 3.2),
                    tint: rng.next_f32(),
                });
            }
        }
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::catalog;

    #[test]
    fn scenery_keeps_off_the_track_and_is_deterministic() {
        for def in catalog() {
            let t = Track::build(*def);
            let a = generate(&t);
            let b = generate(&t);
            assert_eq!(a.len(), b.len());
            assert!(a.len() > 300, "{} has only {} props", def.name, a.len());
            for p in a
                .iter()
                .filter(|p| matches!(p.kind, PropKind::Tree(_) | PropKind::Rock))
            {
                let tp = t.locate(Vec2::new(p.pos.x, p.pos.z));
                assert!(tp.lateral.abs() > t.half_width + RUNOFF + 2.0);
            }
        }
    }
}
