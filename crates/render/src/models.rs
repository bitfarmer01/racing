//! Procedural models: the sport bike and rider, trees, rocks and the
//! trackside furniture. Built once at load into [`MeshBuilder`]s.
//!
//! Bike space: +x right, +y up, -z forward, ground at y = 0.

use glam::{Mat4, Quat, Vec2, Vec3};

use crate::mesh::{kind, Mat, MeshBuilder};

/// Bike part ids (vertex `mat.r` in the bike pipeline).
pub mod part {
    pub const BODY: u8 = 0;
    pub const FRONT_WHEEL: u8 = 1;
    pub const REAR_WHEEL: u8 = 2;
    pub const RIDER: u8 = 10;
    pub const RIGHT_ARM: u8 = 11;
    pub const LEFT_ARM: u8 = 12;
}

/// Bike vertex flags (vertex `mat.a`).
pub mod flag {
    pub const PAINT: f32 = 1.0 / 255.0;
    pub const PAINT2: f32 = 2.0 / 255.0;
    pub const HEADLIGHT: f32 = 4.0 / 255.0;
    pub const TAILLIGHT: f32 = 8.0 / 255.0;
}

pub const WHEEL_R: f32 = 0.325;
pub const FRONT_AXLE: Vec3 = Vec3::new(0.0, WHEEL_R, -0.72);
pub const REAR_AXLE: Vec3 = Vec3::new(0.0, WHEEL_R, 0.70);
pub const RIGHT_SHOULDER: Vec3 = Vec3::new(0.2, 1.30, -0.18);
pub const LEFT_SHOULDER: Vec3 = Vec3::new(-0.2, 1.30, -0.18);

fn bike_mat(color: [f32; 3], rough: f32, metal: f32, part: u8, flags: f32) -> Mat {
    Mat::new(color, rough, metal).kind(part).extra(flags)
}

/// The full bike + rider as one mesh; parts animate in the vertex shader.
pub fn bike() -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let paint = bike_mat([0.8, 0.8, 0.8], 0.22, 0.35, part::BODY, flag::PAINT);
    let carbon = bike_mat([0.025, 0.025, 0.03], 0.35, 0.2, part::BODY, 0.0);
    let metal_dark = bike_mat([0.08, 0.08, 0.09], 0.45, 0.8, part::BODY, 0.0);
    let chrome = bike_mat([0.85, 0.85, 0.88], 0.12, 1.0, part::BODY, 0.0);
    let gold = bike_mat([0.85, 0.55, 0.15], 0.25, 1.0, part::BODY, 0.0);
    let glass = bike_mat([0.05, 0.06, 0.08], 0.05, 0.0, part::BODY, 0.0);
    let seat = bike_mat([0.03, 0.03, 0.03], 0.7, 0.0, part::BODY, 0.0);
    let head = bike_mat([1.0, 0.95, 0.85], 0.2, 0.0, part::BODY, flag::HEADLIGHT);
    let tail = bike_mat([1.0, 0.05, 0.03], 0.2, 0.0, part::BODY, flag::TAILLIGHT);

    // Wheels: tyre, rim disc and spokes so the spin reads.
    for (axle, p) in [
        (FRONT_AXLE, part::FRONT_WHEEL),
        (REAR_AXLE, part::REAR_WHEEL),
    ] {
        let tyre = bike_mat([0.02, 0.02, 0.022], 0.85, 0.0, p, 0.0);
        let rim = bike_mat([0.06, 0.06, 0.065], 0.3, 0.9, p, 0.0);
        let rim_lip = bike_mat([0.9, 0.6, 0.12], 0.25, 1.0, p, 0.0);
        let width = if p == part::REAR_WHEEL { 0.085 } else { 0.07 };
        b.torus_x(axle, WHEEL_R - width, width, 20, 6, &tyre);
        b.torus_x(axle, WHEEL_R - width * 2.0 - 0.01, 0.012, 16, 4, &rim_lip);
        let saved = b.transform;
        b.transform = saved
            * Mat4::from_rotation_translation(
                Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                axle,
            );
        b.cylinder(Vec3::ZERO, -0.05, 0.05, 0.06, 0.06, 10, true, &rim);
        b.transform = saved;
        for k in 0..5 {
            let a = k as f32 / 5.0 * std::f32::consts::TAU;
            let dir = Vec3::new(0.0, a.cos(), a.sin());
            b.beam(
                axle + dir * 0.05,
                axle + dir * (WHEEL_R - width * 2.0),
                0.018,
                &rim,
            );
        }
        if p == part::FRONT_WHEEL {
            // Brake discs.
            for x in [-0.085f32, 0.085] {
                let saved = b.transform;
                b.transform = saved
                    * Mat4::from_rotation_translation(
                        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                        axle + Vec3::X * x,
                    );
                b.cylinder(Vec3::ZERO, -0.006, 0.006, 0.16, 0.16, 14, true, &chrome);
                b.transform = saved;
            }
        }
    }

    // Front forks and clip-ons.
    for x in [-0.1f32, 0.1] {
        b.beam(
            Vec3::new(x, WHEEL_R, -0.72),
            Vec3::new(x, 0.98, -0.5),
            0.028,
            &gold,
        );
    }
    b.beam(
        Vec3::new(-0.33, 1.02, -0.46),
        Vec3::new(0.33, 1.02, -0.46),
        0.018,
        &metal_dark,
    );
    // Swingarm and rear shock.
    for x in [-0.11f32, 0.11] {
        b.beam(
            Vec3::new(x, WHEEL_R, 0.70),
            Vec3::new(x, 0.45, 0.05),
            0.035,
            &metal_dark,
        );
    }
    b.beam(
        Vec3::new(0.0, 0.5, 0.25),
        Vec3::new(0.0, 0.85, 0.05),
        0.03,
        &gold,
    );
    // Engine and frame spars.
    b.cuboid(
        Vec3::new(0.0, 0.48, -0.08),
        Vec3::new(0.18, 0.18, 0.26),
        Vec2::new(0.9, 0.85),
        &metal_dark,
    );
    for x in [-0.16f32, 0.16] {
        b.beam(
            Vec3::new(x, 0.88, -0.48),
            Vec3::new(x, 0.62, 0.12),
            0.03,
            &chrome,
        );
    }
    // Exhaust: header under the engine, can up by the tail.
    b.beam(
        Vec3::new(0.08, 0.28, -0.25),
        Vec3::new(0.16, 0.36, 0.25),
        0.04,
        &chrome,
    );
    b.beam(
        Vec3::new(0.16, 0.36, 0.25),
        Vec3::new(0.2, 0.62, 0.72),
        0.06,
        &carbon,
    );
    b.cylinder(
        Vec3::new(0.2, 0.62, 0.72),
        -0.01,
        0.0,
        0.05,
        0.05,
        8,
        false,
        &chrome,
    );

    // Bodywork.
    b.cuboid(
        Vec3::new(0.0, 0.76, -0.58),
        Vec3::new(0.21, 0.2, 0.26),
        Vec2::new(0.62, 0.8),
        &paint,
    );
    b.cuboid(
        Vec3::new(0.0, 0.62, -0.22),
        Vec3::new(0.23, 0.2, 0.25),
        Vec2::new(0.88, 1.0),
        &paint,
    );
    b.cuboid(
        Vec3::new(0.0, 0.93, -0.16),
        Vec3::new(0.17, 0.11, 0.22),
        Vec2::new(0.78, 0.86),
        &paint,
    );
    b.cuboid(
        Vec3::new(0.0, 0.97, 0.46),
        Vec3::new(0.13, 0.08, 0.22),
        Vec2::new(0.55, 1.0),
        &paint,
    );
    b.cuboid(
        Vec3::new(0.0, 0.99, -0.62),
        Vec3::new(0.13, 0.08, 0.1),
        Vec2::new(0.6, 0.6),
        &glass,
    );
    b.cuboid(
        Vec3::new(0.0, 0.9, 0.2),
        Vec3::new(0.14, 0.04, 0.16),
        Vec2::new(0.9, 1.0),
        &seat,
    );
    // Lower fairing belly pan.
    b.cuboid(
        Vec3::new(0.0, 0.36, -0.2),
        Vec3::new(0.16, 0.06, 0.28),
        Vec2::new(1.2, 1.0),
        &carbon,
    );
    // Lights.
    for x in [-0.075f32, 0.075] {
        b.cube(
            Vec3::new(x, 0.77, -0.845),
            Vec3::new(0.045, 0.028, 0.01),
            &head,
        );
    }
    b.cube(
        Vec3::new(0.0, 0.98, 0.685),
        Vec3::new(0.08, 0.025, 0.01),
        &tail,
    );

    rider(&mut b);
    b
}

fn rider(b: &mut MeshBuilder) {
    let leather = bike_mat([0.5, 0.5, 0.5], 0.55, 0.0, part::RIDER, flag::PAINT2);
    let stripe = bike_mat([0.8, 0.8, 0.8], 0.4, 0.1, part::RIDER, flag::PAINT);
    let helmet = bike_mat([0.8, 0.8, 0.8], 0.15, 0.2, part::RIDER, flag::PAINT);
    let visor = bike_mat([0.02, 0.02, 0.03], 0.05, 0.6, part::RIDER, 0.0);
    let boots = bike_mat([0.03, 0.03, 0.03], 0.6, 0.0, part::RIDER, 0.0);
    let gloves = bike_mat([0.04, 0.04, 0.04], 0.6, 0.0, part::RIDER, 0.0);

    // Legs: hips -> knees (tucked against the tank) -> pegs.
    for x in [-1.0f32, 1.0] {
        let hip = Vec3::new(0.14 * x, 0.99, 0.16);
        let knee = Vec3::new(0.25 * x, 0.82, -0.22);
        let foot = Vec3::new(0.2 * x, 0.46, 0.18);
        b.beam(hip, knee, 0.085, &leather);
        b.beam(knee, foot, 0.07, &leather);
        b.cube(
            foot + Vec3::new(0.0, -0.02, -0.06),
            Vec3::new(0.05, 0.05, 0.11),
            &boots,
        );
    }
    // Torso in a racing tuck, with a stripe down the back.
    let hips = Vec3::new(0.0, 1.03, 0.16);
    let shoulders = Vec3::new(0.0, 1.31, -0.2);
    b.beam(hips, shoulders, 0.17, &leather);
    b.beam(
        hips + Vec3::new(0.0, 0.13, 0.03),
        shoulders + Vec3::new(0.0, 0.14, 0.04),
        0.06,
        &stripe,
    );
    b.cuboid(
        Vec3::new(0.0, 1.33, -0.16),
        Vec3::new(0.22, 0.08, 0.1),
        Vec2::new(0.9, 1.0),
        &leather,
    );
    // Helmet and visor.
    b.sphere(
        Vec3::new(0.0, 1.47, -0.33),
        Vec3::new(0.145, 0.15, 0.17),
        7,
        10,
        &helmet,
    );
    b.sphere(
        Vec3::new(0.0, 1.455, -0.42),
        Vec3::new(0.115, 0.065, 0.09),
        5,
        8,
        &visor,
    );
    // Arms to the clip-ons.
    for (x, p) in [(1.0f32, part::RIGHT_ARM), (-1.0, part::LEFT_ARM)] {
        let arm = bike_mat([0.5, 0.5, 0.5], 0.55, 0.0, p, flag::PAINT2);
        let glove = Mat { kind: p, ..gloves };
        let shoulder = if x > 0.0 {
            RIGHT_SHOULDER
        } else {
            LEFT_SHOULDER
        };
        let elbow = Vec3::new(0.29 * x, 1.12, -0.32);
        let hand = Vec3::new(0.3 * x, 1.03, -0.46);
        b.beam(shoulder, elbow, 0.06, &arm);
        b.beam(elbow, hand, 0.05, &arm);
        b.cube(hand, Vec3::new(0.045, 0.04, 0.05), &glove);
    }
}

fn foliage(color: [f32; 3]) -> Mat {
    Mat::new(color, 0.8, 0.0).kind(kind::FOLIAGE)
}

/// Writes each vertex's height fraction into uv.y (wind sway weight).
fn sway_weights(b: &mut MeshBuilder, height: f32) {
    for v in &mut b.vertices {
        v.uv[1] = (v.pos[1] / height).clamp(0.0, 1.0);
    }
}

pub fn pine(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let bark = Mat::new([0.16, 0.1, 0.06], 0.9, 0.0).kind(kind::FOLIAGE);
    b.cylinder(Vec3::ZERO, -0.3, 2.6, 0.2, 0.1, 6, false, &bark);
    let g = [0.035 + tint * 0.02, 0.085 + tint * 0.03, 0.04];
    let leaves = foliage(g);
    let dark = foliage([g[0] * 0.7, g[1] * 0.7, g[2] * 0.7]);
    b.cylinder(Vec3::ZERO, 1.4, 5.2, 1.75, 0.0, 8, true, &dark);
    b.cylinder(Vec3::ZERO, 3.2, 6.9, 1.3, 0.0, 8, true, &leaves);
    b.cylinder(Vec3::ZERO, 4.9, 8.6, 0.85, 0.0, 7, true, &leaves);
    sway_weights(&mut b, 8.6);
    b
}

pub fn broadleaf(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let bark = Mat::new([0.2, 0.14, 0.09], 0.9, 0.0).kind(kind::FOLIAGE);
    b.cylinder(Vec3::ZERO, -0.3, 3.2, 0.26, 0.16, 6, false, &bark);
    let g = [0.07 + tint * 0.05, 0.13 + tint * 0.03, 0.035];
    let leaves = foliage(g);
    for (c, s) in [
        (Vec3::new(0.0, 4.6, 0.0), Vec3::new(2.3, 1.9, 2.3)),
        (Vec3::new(1.1, 3.9, 0.5), Vec3::new(1.5, 1.3, 1.5)),
        (Vec3::new(-0.9, 4.1, -0.7), Vec3::new(1.6, 1.4, 1.5)),
        (Vec3::new(0.2, 5.7, -0.3), Vec3::new(1.4, 1.2, 1.4)),
    ] {
        b.sphere(c, s, 5, 8, &leaves);
    }
    sway_weights(&mut b, 7.0);
    b
}

pub fn palm(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let bark = Mat::new([0.3, 0.22, 0.14], 0.9, 0.0).kind(kind::FOLIAGE);
    let mut prev = Vec3::ZERO;
    let lean = 0.06 + tint * 0.08;
    for k in 1..=6 {
        let y = k as f32 * 1.35;
        let p = Vec3::new(lean * y * y * 0.12, y, 0.0);
        b.beam(prev, p, 0.17 - k as f32 * 0.012, &bark);
        prev = p;
    }
    let top = prev;
    let leaf = foliage([0.06 + tint * 0.03, 0.16, 0.04]);
    for k in 0..7 {
        let a = k as f32 / 7.0 * std::f32::consts::TAU + tint;
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        let side = Vec3::new(-dir.z, 0.0, dir.x) * 0.45;
        let mid = top + dir * 1.9 + Vec3::Y * 0.5;
        let tip = top + dir * 3.6 - Vec3::Y * 1.1;
        b.quad([top, mid + side, tip, mid - side], &leaf);
        b.quad([top, mid - side, tip, mid + side], &leaf);
    }
    sway_weights(&mut b, 8.5);
    b
}

pub fn cactus(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let skin = Mat::new([0.13, 0.2 + tint * 0.05, 0.08], 0.75, 0.0).kind(kind::FOLIAGE);
    b.cylinder(Vec3::ZERO, -0.3, 5.0, 0.38, 0.3, 8, true, &skin);
    for (side, h) in [(1.0f32, 2.0), (-1.0, 2.8)] {
        let base = Vec3::new(0.0, h, 0.0);
        let elbow = Vec3::new(side * 1.0, h + 0.2, 0.0);
        b.beam(base, elbow, 0.2, &skin);
        b.cylinder(elbow, 0.0, 1.6, 0.24, 0.2, 7, true, &skin);
    }
    sway_weights(&mut b, 40.0);
    b
}

pub fn rock(tint: f32, rock_color: [f32; 3]) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let c = [
        rock_color[0] * (0.8 + tint * 0.4),
        rock_color[1] * (0.8 + tint * 0.4),
        rock_color[2] * (0.8 + tint * 0.4),
    ];
    b.sphere(
        Vec3::new(0.0, 0.3, 0.0),
        Vec3::new(1.2, 0.8, 1.0),
        4,
        7,
        &Mat::new(c, 0.9, 0.0),
    );
    // Lumpy: push vertices in and out deterministically.
    for (i, v) in b.vertices.iter_mut().enumerate() {
        let h = ((i as f32 * 12.9898 + tint * 78.233).sin() * 43_758.547).fract();
        let k = 0.75 + h * 0.5;
        v.pos = [v.pos[0] * k, (v.pos[1] - 0.3) * k + 0.3, v.pos[2] * k];
    }
    b.recompute_normals();
    b
}

pub fn lamp() -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let pole = Mat::new([0.08, 0.08, 0.09], 0.4, 0.8);
    b.cylinder(Vec3::ZERO, 0.0, 8.0, 0.12, 0.08, 8, false, &pole);
    b.beam(
        Vec3::new(0.0, 7.9, 0.0),
        Vec3::new(0.0, 8.3, 2.2),
        0.05,
        &pole,
    );
    b.cube(
        Vec3::new(0.0, 8.25, 2.4),
        Vec3::new(0.18, 0.08, 0.35),
        &pole,
    );
    let glow = Mat::new([1.0, 0.75, 0.42], 0.3, 0.0).kind(kind::LAMP_GLOW);
    b.quad(
        [
            Vec3::new(-0.15, 8.16, 2.1),
            Vec3::new(0.15, 8.16, 2.1),
            Vec3::new(0.15, 8.16, 2.7),
            Vec3::new(-0.15, 8.16, 2.7),
        ],
        &glow,
    );
    b
}

/// Grandstand facing local -z, 32 m long along x.
pub fn grandstand(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let concrete = Mat::new([0.42, 0.41, 0.4], 0.85, 0.0);
    let crowd = Mat::new([0.5, 0.5, 0.5], 0.9, 0.0)
        .kind(kind::CROWD)
        .extra(tint);
    let roof = Mat::new([0.75, 0.76, 0.8], 0.4, 0.6);
    let half = 16.0;
    for k in 0..5 {
        let z = k as f32 * 1.6;
        let y = k as f32 * 0.9;
        b.cube(
            Vec3::new(0.0, y * 0.5 + 0.25, z),
            Vec3::new(half, y * 0.5 + 0.25, 0.8),
            &concrete,
        );
        // Spectators on each step, facing the track.
        b.quad(
            [
                Vec3::new(-half, y + 0.5, z - 0.6),
                Vec3::new(half, y + 0.5, z - 0.6),
                Vec3::new(half, y + 1.4, z + 0.1),
                Vec3::new(-half, y + 1.4, z + 0.1),
            ]
            .map(|p| p)
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
            &crowd,
        );
    }
    b.cube(
        Vec3::new(0.0, 4.8, 7.6),
        Vec3::new(half, 4.8, 0.3),
        &concrete,
    );
    for x in [-half + 0.5, 0.0, half - 0.5] {
        b.beam(Vec3::new(x, 0.0, 8.0), Vec3::new(x, 9.2, 8.0), 0.15, &roof);
    }
    b.cuboid(
        Vec3::new(0.0, 9.3, 3.5),
        Vec3::new(half + 0.5, 0.12, 4.8),
        Vec2::ONE,
        &roof,
    );
    // Sponsor boards along the front.
    let colors = [
        [0.9, 0.12, 0.1],
        [0.05, 0.25, 0.8],
        [0.95, 0.75, 0.05],
        [0.1, 0.6, 0.3],
    ];
    for k in 0..4 {
        let x0 = -half + k as f32 * 8.0;
        let m = Mat::new(colors[(k + (tint * 4.0) as usize) % 4], 0.5, 0.0)
            .kind(kind::EMISSIVE)
            .extra(0.25);
        b.quad(
            [
                Vec3::new(x0 + 0.2, 0.2, -1.0),
                Vec3::new(x0 + 7.8, 0.2, -1.0),
                Vec3::new(x0 + 7.8, 1.2, -1.0),
                Vec3::new(x0 + 0.2, 1.2, -1.0),
            ]
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
            &m,
        );
    }
    b
}

/// Start gantry: local x along the track, spans local z in [-w, w].
pub fn gantry(half_width: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let steel = Mat::new([0.12, 0.12, 0.14], 0.35, 0.9);
    let w = half_width + 1.6;
    for z in [-w, w] {
        b.cube(Vec3::new(0.0, 3.8, z), Vec3::new(0.35, 3.8, 0.35), &steel);
    }
    b.cube(
        Vec3::new(0.0, 7.4, 0.0),
        Vec3::new(0.5, 0.6, w + 0.35),
        &steel,
    );
    // Checkered banners on both faces.
    let checker = Mat::new([1.0, 1.0, 1.0], 0.5, 0.0).kind(kind::CHECKER);
    for (x, flip) in [(-0.52f32, false), (0.52, true)] {
        let mut q = [
            Vec3::new(x, 6.85, w),
            Vec3::new(x, 6.85, -w),
            Vec3::new(x, 7.95, -w),
            Vec3::new(x, 7.95, w),
        ];
        if flip {
            q = [q[1], q[0], q[3], q[2]];
        }
        b.quad(q, &checker);
    }
    // Five start-light pods facing the grid (local -x).
    for k in 0..5 {
        let z = (k as f32 - 2.0) * 1.3;
        let pod = Mat::new([0.02, 0.02, 0.02], 0.4, 0.2);
        b.cube(Vec3::new(-0.65, 6.3, z), Vec3::new(0.15, 0.35, 0.45), &pod);
        let light = Mat::new([1.0, 1.0, 1.0], 0.2, 0.0)
            .kind(kind::START_LIGHT)
            .extra(k as f32 / 8.0);
        for y in [6.1f32, 6.5] {
            b.quad(
                [
                    Vec3::new(-0.81, y - 0.14, z + 0.14),
                    Vec3::new(-0.81, y - 0.14, z - 0.14),
                    Vec3::new(-0.81, y + 0.14, z - 0.14),
                    Vec3::new(-0.81, y + 0.14, z + 0.14),
                ],
                &light,
            );
        }
    }
    b
}

pub fn marshal(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let wall = Mat::new([0.85, 0.85, 0.82], 0.7, 0.0);
    let orange = Mat::new([1.0, 0.35, 0.02], 0.6, 0.0);
    b.cube(Vec3::new(0.0, 1.2, 0.0), Vec3::new(1.4, 1.2, 1.0), &wall);
    b.cube(Vec3::new(0.0, 2.5, 0.0), Vec3::new(1.6, 0.1, 1.2), &orange);
    let pole = Mat::new([0.6, 0.6, 0.6], 0.3, 0.9);
    b.beam(
        Vec3::new(1.2, 0.0, -1.2),
        Vec3::new(1.2, 5.0, -1.2),
        0.04,
        &pole,
    );
    let flag = Mat::new([1.0, 0.8 * tint + 0.2, 0.05], 0.7, 0.0).kind(kind::FOLIAGE);
    let q = [
        Vec3::new(1.2, 4.0, -1.2),
        Vec3::new(2.6, 4.1, -1.2),
        Vec3::new(2.6, 4.9, -1.2),
        Vec3::new(1.2, 4.9, -1.2),
    ];
    b.quad(q, &flag);
    b.quad([q[1], q[0], q[3], q[2]], &flag);
    sway_weights(&mut b, 5.0);
    for v in &mut b.vertices {
        // Only the flag sways.
        if v.mat[0] != kind::FOLIAGE {
            v.uv[1] = 0.0;
        }
    }
    b
}

/// Low-detail stand-ins drawn beyond the LOD distance: same silhouette and
/// colours, a fraction of the triangles.
pub fn pine_lod(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let bark = Mat::new([0.16, 0.1, 0.06], 0.9, 0.0).kind(kind::FOLIAGE);
    b.cylinder(Vec3::ZERO, -0.3, 2.0, 0.2, 0.12, 3, false, &bark);
    let g = [0.035 + tint * 0.02, 0.085 + tint * 0.03, 0.04];
    let dark = foliage([g[0] * 0.7, g[1] * 0.7, g[2] * 0.7]);
    b.cylinder(Vec3::ZERO, 1.4, 6.0, 1.7, 0.0, 5, false, &dark);
    b.cylinder(Vec3::ZERO, 4.0, 8.6, 1.1, 0.0, 5, false, &foliage(g));
    sway_weights(&mut b, 8.6);
    b
}

pub fn broadleaf_lod(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let bark = Mat::new([0.2, 0.14, 0.09], 0.9, 0.0).kind(kind::FOLIAGE);
    b.cylinder(Vec3::ZERO, -0.3, 3.2, 0.26, 0.16, 3, false, &bark);
    let g = [0.07 + tint * 0.05, 0.13 + tint * 0.03, 0.035];
    b.sphere(
        Vec3::new(0.1, 4.7, -0.1),
        Vec3::new(2.6, 2.3, 2.5),
        3,
        6,
        &foliage(g),
    );
    sway_weights(&mut b, 7.0);
    b
}

pub fn palm_lod(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let bark = Mat::new([0.3, 0.22, 0.14], 0.9, 0.0).kind(kind::FOLIAGE);
    let lean = 0.06 + tint * 0.08;
    let mid = Vec3::new(lean * 16.4 * 0.12, 4.05, 0.0);
    let top = Vec3::new(lean * 65.6 * 0.12, 8.1, 0.0);
    b.beam(Vec3::ZERO, mid, 0.15, &bark);
    b.beam(mid, top, 0.11, &bark);
    let leaf = foliage([0.06 + tint * 0.03, 0.16, 0.04]);
    for k in 0..5 {
        let a = k as f32 / 5.0 * std::f32::consts::TAU + tint;
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        let side = Vec3::new(-dir.z, 0.0, dir.x) * 0.5;
        let m = top + dir * 1.9 + Vec3::Y * 0.5;
        let tip = top + dir * 3.6 - Vec3::Y * 1.1;
        b.quad([top, m + side, tip, m - side], &leaf);
    }
    sway_weights(&mut b, 8.5);
    b
}

pub fn cactus_lod(tint: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let skin = Mat::new([0.13, 0.2 + tint * 0.05, 0.08], 0.75, 0.0).kind(kind::FOLIAGE);
    b.cylinder(Vec3::ZERO, -0.3, 5.0, 0.38, 0.3, 4, false, &skin);
    for (side, h) in [(1.0f32, 2.0), (-1.0, 2.8)] {
        let elbow = Vec3::new(side * 1.0, h + 0.2, 0.0);
        b.beam(Vec3::new(0.0, h, 0.0), elbow, 0.2, &skin);
        b.cylinder(elbow, 0.0, 1.6, 0.24, 0.2, 3, false, &skin);
    }
    sway_weights(&mut b, 40.0);
    b
}

pub fn rock_lod(tint: f32, rock_color: [f32; 3]) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let k = 0.8 + tint * 0.4;
    let c = [rock_color[0] * k, rock_color[1] * k, rock_color[2] * k];
    b.sphere(
        Vec3::new(0.0, 0.3, 0.0),
        Vec3::new(1.15, 0.75, 0.95),
        2,
        5,
        &Mat::new(c, 0.9, 0.0),
    );
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lods_are_much_lighter() {
        let pairs = [
            (pine(0.5), pine_lod(0.5)),
            (broadleaf(0.5), broadleaf_lod(0.5)),
            (palm(0.5), palm_lod(0.5)),
            (cactus(0.5), cactus_lod(0.5)),
            (rock(0.5, [0.5; 3]), rock_lod(0.5, [0.5; 3])),
        ];
        for (hi, lo) in pairs {
            let (h, l) = (hi.indices.len() / 3, lo.indices.len() / 3);
            println!("{h} -> {l}");
            assert!(l * 2 <= h, "{h} -> {l}");
        }
    }
}
