//! Procedural models: the sport bike and rider, trees, rocks and the
//! trackside furniture. Built once at load into [`MeshBuilder`]s.
//!
//! Bike space: +x right, +y up, -z forward, ground at y = 0.

use glam::{Mat4, Quat, Vec2, Vec3};

use crate::mesh::{kind, Mat, MeshBuilder, Section};

/// Bike part ids (vertex `mat.r` in the bike pipeline). The vertex shader
/// poses each part: wheels spin and steer, the fork and swingarm follow the
/// suspension, and the rider's limbs move.
pub mod part {
    /// Sprung mass: frame, engine, bodywork. Rides on the suspension.
    pub const BODY: u8 = 0;
    pub const FRONT_WHEEL: u8 = 1;
    pub const REAR_WHEEL: u8 = 2;
    /// Fork sliders, calipers and fender: steer, but stay with the wheel.
    pub const FORK_LOWER: u8 = 3;
    /// Pivots so the rear axle stays on the ground as the shock works.
    pub const SWINGARM: u8 = 4;
    /// Fork tubes, yokes and bars: steer and ride on the suspension.
    pub const STEER: u8 = 5;
    pub const RIDER: u8 = 10;
    pub const RIGHT_ARM: u8 = 11;
    pub const LEFT_ARM: u8 = 12;
    pub const RIGHT_LEG: u8 = 13;
    pub const LEFT_LEG: u8 = 14;
    /// Weapons, held in one hand; hidden unless the rider carries that one.
    pub const RIGHT_CLUB: u8 = 15;
    pub const LEFT_CLUB: u8 = 16;
    pub const RIGHT_CHAIN: u8 = 17;
    pub const LEFT_CHAIN: u8 = 18;
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
/// Top of the steering head, where the fork tubes meet the top yoke.
pub const STEER_HEAD: Vec3 = Vec3::new(0.0, 0.98, -0.5);
pub const SWINGARM_PIVOT: Vec3 = Vec3::new(0.0, 0.47, 0.06);
pub const RIGHT_SHOULDER: Vec3 = Vec3::new(0.2, 1.30, -0.16);
pub const LEFT_SHOULDER: Vec3 = Vec3::new(-0.2, 1.30, -0.16);
pub const RIGHT_HIP: Vec3 = Vec3::new(0.13, 1.0, 0.2);
pub const LEFT_HIP: Vec3 = Vec3::new(-0.13, 1.0, 0.2);

fn bike_mat(color: [f32; 3], rough: f32, metal: f32, part: u8, flags: f32) -> Mat {
    Mat::new(color, rough, metal).kind(part).extra(flags)
}

fn sec(z: f32, y: f32, w: f32, h: f32, square: f32) -> Section {
    Section::new(Vec3::new(0.0, y, z), w, h, square)
}

fn sec_x(x: f32, y: f32, z: f32, w: f32, h: f32, square: f32) -> Section {
    Section::new(Vec3::new(x, y, z), w, h, square)
}

/// A disc lying across the x axis (brake rotor, hub, can tip).
fn disc_x(b: &mut MeshBuilder, c: Vec3, half: f32, r: f32, seg: u32, m: &Mat) {
    let saved = b.transform;
    b.transform = saved
        * Mat4::from_rotation_translation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), c);
    b.cylinder(Vec3::ZERO, -half, half, r, r, seg, true, m);
    b.transform = saved;
}

/// The full bike + rider as one mesh; parts animate in the vertex shader.
pub fn bike() -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let paint = bike_mat([0.8, 0.8, 0.8], 0.2, 0.3, part::BODY, flag::PAINT);
    let carbon = bike_mat([0.03, 0.03, 0.035], 0.32, 0.25, part::BODY, 0.0);
    let metal_dark = bike_mat([0.09, 0.09, 0.1], 0.42, 0.85, part::BODY, 0.0);
    let alloy = bike_mat([0.55, 0.56, 0.6], 0.3, 1.0, part::BODY, 0.0);
    let chrome = bike_mat([0.9, 0.9, 0.92], 0.1, 1.0, part::BODY, 0.0);
    let titanium = bike_mat([0.45, 0.42, 0.5], 0.22, 1.0, part::BODY, 0.0);
    let glass = bike_mat([0.06, 0.07, 0.09], 0.04, 0.0, part::BODY, 0.0);
    let seat = bike_mat([0.035, 0.035, 0.035], 0.75, 0.0, part::BODY, 0.0);
    let head = bike_mat([1.0, 0.95, 0.85], 0.2, 0.0, part::BODY, flag::HEADLIGHT);
    let tail = bike_mat([1.0, 0.05, 0.03], 0.2, 0.0, part::BODY, flag::TAILLIGHT);

    wheels(&mut b);
    front_end(&mut b);

    // Frame: twin alloy spars from the steering head round the engine to
    // the swingarm pivot.
    for x in [-1.0f32, 1.0] {
        b.loft(
            &[
                sec_x(0.07 * x, 0.9, -0.46, 0.025, 0.06, 4.0),
                sec_x(0.15 * x, 0.8, -0.24, 0.025, 0.07, 4.0),
                sec_x(0.15 * x, 0.66, -0.02, 0.025, 0.07, 4.0),
                sec_x(0.13 * x, 0.5, 0.06, 0.03, 0.06, 4.0),
            ],
            Vec3::X,
            8,
            &alloy,
        );
    }
    // Engine: crankcase, a canted cylinder block with fins, side covers.
    b.loft(
        &[
            sec(-0.34, 0.38, 0.13, 0.1, 4.0),
            sec(-0.2, 0.36, 0.16, 0.13, 4.0),
            sec(0.04, 0.4, 0.15, 0.12, 4.0),
            sec(0.1, 0.44, 0.1, 0.08, 4.0),
        ],
        Vec3::X,
        12,
        &metal_dark,
    );
    b.cuboid(
        Vec3::new(0.0, 0.6, -0.24),
        Vec3::new(0.14, 0.12, 0.09),
        Vec2::new(0.95, 0.85),
        &metal_dark,
    );
    for k in 0..4 {
        b.cube(
            Vec3::new(0.0, 0.52 + k as f32 * 0.05, -0.33),
            Vec3::new(0.145, 0.008, 0.012),
            &alloy,
        );
    }
    for x in [-0.165f32, 0.165] {
        disc_x(&mut b, Vec3::new(x, 0.37, -0.14), 0.012, 0.085, 12, &alloy);
    }
    // Exhaust: headers sweep under the engine to a short titanium can.
    for x in [-0.05f32, 0.05] {
        b.loft(
            &[
                Section::round(Vec3::new(x, 0.56, -0.38), 0.022),
                Section::round(Vec3::new(x, 0.34, -0.42), 0.024),
                Section::round(Vec3::new(x * 1.6, 0.22, -0.2), 0.026),
                Section::round(Vec3::new(0.12, 0.26, 0.1), 0.03),
            ],
            Vec3::X,
            8,
            &chrome,
        );
    }
    b.loft(
        &[
            Section::round(Vec3::new(0.14, 0.3, 0.14), 0.04),
            Section::new(Vec3::new(0.19, 0.42, 0.34), 0.07, 0.06, 2.6),
            Section::new(Vec3::new(0.21, 0.52, 0.56), 0.065, 0.055, 2.6),
            Section::round(Vec3::new(0.215, 0.545, 0.62), 0.045),
        ],
        Vec3::X,
        12,
        &titanium,
    );
    b.sphere(
        Vec3::new(0.215, 0.548, 0.63),
        Vec3::splat(0.03),
        3,
        8,
        &carbon,
    );

    // Fairing: a sharp nose above the front wheel, then side panels that
    // drop down round the engine once they're clear of the tyre.
    b.loft(
        &[
            sec(-0.96, 0.8, 0.025, 0.035, 2.2),
            sec(-0.91, 0.79, 0.11, 0.085, 2.8),
            sec(-0.8, 0.8, 0.16, 0.125, 3.4),
            sec(-0.66, 0.8, 0.19, 0.14, 3.8),
            sec(-0.5, 0.74, 0.205, 0.19, 4.2),
            sec(-0.36, 0.61, 0.205, 0.3, 4.4),
            sec(-0.2, 0.56, 0.19, 0.27, 4.2),
            sec(-0.06, 0.53, 0.15, 0.2, 3.6),
            sec(0.0, 0.53, 0.09, 0.12, 2.8),
        ],
        Vec3::X,
        24,
        &paint,
    );
    // Air intake under the nose.
    b.loft(
        &[
            sec(-0.92, 0.73, 0.04, 0.02, 3.0),
            sec(-0.86, 0.72, 0.05, 0.026, 3.0),
        ],
        Vec3::X,
        10,
        &carbon,
    );
    // Screen.
    b.loft(
        &[
            sec(-0.84, 0.92, 0.07, 0.01, 2.4),
            sec(-0.74, 0.97, 0.11, 0.012, 2.4),
            sec(-0.62, 1.0, 0.1, 0.012, 2.4),
            sec(-0.56, 0.99, 0.06, 0.008, 2.4),
        ],
        Vec3::X,
        12,
        &glass,
    );
    // Tank, seat and a tail that kicks up.
    b.loft(
        &[
            sec(-0.5, 0.93, 0.1, 0.06, 2.6),
            sec(-0.4, 0.97, 0.17, 0.1, 2.8),
            sec(-0.2, 0.98, 0.19, 0.11, 3.0),
            sec(-0.02, 0.94, 0.16, 0.09, 3.0),
            sec(0.06, 0.9, 0.1, 0.05, 2.6),
        ],
        Vec3::X,
        16,
        &paint,
    );
    b.loft(
        &[
            sec(0.02, 0.9, 0.11, 0.03, 3.5),
            sec(0.2, 0.91, 0.13, 0.035, 3.5),
            sec(0.34, 0.94, 0.11, 0.03, 3.5),
        ],
        Vec3::X,
        12,
        &seat,
    );
    b.loft(
        &[
            sec(0.12, 0.84, 0.14, 0.05, 3.0),
            sec(0.34, 0.9, 0.15, 0.06, 3.0),
            sec(0.56, 0.98, 0.1, 0.06, 3.0),
            sec(0.72, 1.03, 0.05, 0.035, 2.6),
            sec(0.76, 1.03, 0.02, 0.02, 2.0),
        ],
        Vec3::X,
        14,
        &paint,
    );
    b.cube(
        Vec3::new(0.0, 1.0, 0.71),
        Vec3::new(0.035, 0.014, 0.012),
        &tail,
    );
    for x in [-0.07f32, 0.07] {
        b.sphere(
            Vec3::new(x, 0.79, -0.89),
            Vec3::new(0.04, 0.022, 0.03),
            3,
            8,
            &head,
        );
    }
    // Rear hugger and number board under the tail.
    b.loft(
        &[
            sec(0.48, 0.66, 0.06, 0.015, 3.0),
            sec(0.66, 0.62, 0.09, 0.015, 3.0),
            sec(0.8, 0.5, 0.08, 0.012, 3.0),
        ],
        Vec3::X,
        10,
        &carbon,
    );
    // Rear shock and the pegs.
    b.limb(
        Vec3::new(0.0, 0.48, 0.24),
        Vec3::new(0.0, 0.84, 0.06),
        0.03,
        0.025,
        &chrome,
    );
    for x in [-1.0f32, 1.0] {
        b.beam(
            Vec3::new(0.13 * x, 0.5, 0.18),
            Vec3::new(0.21 * x, 0.46, 0.2),
            0.012,
            &alloy,
        );
    }

    swingarm(&mut b);
    rider(&mut b);
    b
}

fn wheels(b: &mut MeshBuilder) {
    for (axle, p) in [
        (FRONT_AXLE, part::FRONT_WHEEL),
        (REAR_AXLE, part::REAR_WHEEL),
    ] {
        let tyre = bike_mat([0.022, 0.022, 0.024], 0.82, 0.0, p, 0.0);
        let wall = bike_mat([0.04, 0.04, 0.045], 0.6, 0.0, p, 0.0);
        let rim = bike_mat([0.05, 0.05, 0.055], 0.25, 0.9, p, 0.0);
        let lip = bike_mat([0.92, 0.62, 0.12], 0.25, 1.0, p, 0.0);
        let rotor = bike_mat([0.75, 0.75, 0.78], 0.18, 1.0, p, 0.0);
        let width = if p == part::REAR_WHEEL { 0.095 } else { 0.07 };
        // A round racing profile: the tread is the outer half of a torus.
        b.torus_x(axle, WHEEL_R - width, width, 24, 8, &tyre);
        b.torus_x(axle, WHEEL_R - width * 1.9, width * 0.45, 24, 4, &wall);
        b.torus_x(axle, WHEEL_R - width * 2.35, 0.012, 24, 3, &lip);
        disc_x(b, axle, width * 0.7, 0.055, 12, &rim);
        // Three split spokes.
        let inner = 0.05;
        let outer = WHEEL_R - width * 2.3;
        for k in 0..3 {
            let a = k as f32 / 3.0 * std::f32::consts::TAU;
            for da in [-0.16f32, 0.16] {
                let d0 = Vec3::new(0.0, a.cos(), a.sin());
                let d1 = Vec3::new(0.0, (a + da).cos(), (a + da).sin());
                b.beam(axle + d0 * inner, axle + d1 * outer, 0.014, &rim);
            }
        }
        if p == part::FRONT_WHEEL {
            for x in [-0.088f32, 0.088] {
                disc_x(b, axle + Vec3::X * x, 0.004, 0.16, 20, &rotor);
            }
        } else {
            disc_x(b, axle - Vec3::X * 0.1, 0.004, 0.11, 16, &rotor);
            // Sprocket.
            disc_x(b, axle - Vec3::X * 0.075, 0.006, 0.1, 14, &rim);
        }
    }
}

/// Fork, yokes, bars and the front fender.
fn front_end(b: &mut MeshBuilder) {
    let slider = bike_mat([0.92, 0.6, 0.1], 0.25, 1.0, part::FORK_LOWER, 0.0);
    let caliper = bike_mat([0.9, 0.12, 0.08], 0.35, 0.4, part::FORK_LOWER, 0.0);
    let fender = bike_mat([0.8, 0.8, 0.8], 0.2, 0.3, part::FORK_LOWER, flag::PAINT);
    let tube = bike_mat([0.85, 0.85, 0.88], 0.08, 1.0, part::STEER, 0.0);
    let yoke = bike_mat([0.5, 0.5, 0.55], 0.3, 1.0, part::STEER, 0.0);
    let bar = bike_mat([0.06, 0.06, 0.07], 0.4, 0.8, part::STEER, 0.0);
    let grip = bike_mat([0.03, 0.03, 0.03], 0.8, 0.0, part::STEER, 0.0);
    let rake = (STEER_HEAD - FRONT_AXLE).normalize();
    for x in [-0.1f32, 0.1] {
        let base = FRONT_AXLE + Vec3::X * x;
        // Sliders hold the axle; the stanchions slide into them.
        b.limb(base, base + rake * 0.34, 0.036, 0.034, &slider);
        b.loft(
            &[
                Section::round(base + rake * 0.2, 0.026),
                Section::round(base + rake * 0.72, 0.027),
            ],
            Vec3::X,
            10,
            &tube,
        );
        b.cuboid(
            base + Vec3::new(0.0, -0.03, 0.09),
            Vec3::new(0.02, 0.07, 0.035),
            Vec2::ONE,
            &caliper,
        );
    }
    b.loft(
        &[
            sec(-0.96, 0.53, 0.06, 0.01, 2.5),
            sec(-0.86, 0.66, 0.075, 0.012, 2.5),
            sec(-0.72, 0.7, 0.08, 0.012, 2.5),
            sec(-0.58, 0.64, 0.07, 0.01, 2.5),
        ],
        Vec3::X,
        10,
        &fender,
    );
    for t in [0.66f32, 0.74] {
        let c = FRONT_AXLE + rake * t;
        b.cuboid(c, Vec3::new(0.13, 0.015, 0.05), Vec2::ONE, &yoke);
    }
    // Clip-ons.
    for x in [-1.0f32, 1.0] {
        let a = Vec3::new(0.1 * x, 0.96, -0.47);
        let h = Vec3::new(0.3 * x, 0.94, -0.41);
        b.beam(a, h, 0.013, &bar);
        b.limb(
            h,
            h + Vec3::new(0.06 * x, -0.005, 0.015),
            0.019,
            0.019,
            &grip,
        );
    }
}

fn swingarm(b: &mut MeshBuilder) {
    let arm = bike_mat([0.5, 0.52, 0.56], 0.28, 1.0, part::SWINGARM, 0.0);
    let chain = bike_mat([0.08, 0.08, 0.08], 0.5, 0.8, part::SWINGARM, 0.0);
    for x in [-1.0f32, 1.0] {
        b.loft(
            &[
                sec_x(
                    0.11 * x,
                    SWINGARM_PIVOT.y,
                    SWINGARM_PIVOT.z,
                    0.03,
                    0.055,
                    4.0,
                ),
                sec_x(0.13 * x, 0.42, 0.32, 0.028, 0.06, 4.0),
                sec_x(0.12 * x, WHEEL_R + 0.01, REAR_AXLE.z, 0.022, 0.035, 4.0),
            ],
            Vec3::X,
            8,
            &arm,
        );
    }
    b.beam(
        Vec3::new(-0.075, 0.42, -0.12),
        Vec3::new(-0.075, WHEEL_R + 0.08, REAR_AXLE.z),
        0.008,
        &chain,
    );
    b.beam(
        Vec3::new(-0.075, 0.33, -0.1),
        Vec3::new(-0.075, WHEEL_R - 0.08, REAR_AXLE.z),
        0.008,
        &chain,
    );
}

fn rider(b: &mut MeshBuilder) {
    let leather = bike_mat([0.5, 0.5, 0.5], 0.5, 0.0, part::RIDER, flag::PAINT2);
    let stripe = bike_mat([0.8, 0.8, 0.8], 0.35, 0.1, part::RIDER, flag::PAINT);
    let helmet = bike_mat([0.8, 0.8, 0.8], 0.12, 0.25, part::RIDER, flag::PAINT);
    let shell = bike_mat([0.05, 0.05, 0.06], 0.2, 0.3, part::RIDER, 0.0);
    let visor = bike_mat([0.02, 0.02, 0.03], 0.04, 0.7, part::RIDER, 0.0);
    let boots = bike_mat([0.04, 0.04, 0.045], 0.55, 0.0, part::RIDER, 0.0);
    let slider = bike_mat([0.85, 0.85, 0.82], 0.5, 0.0, part::RIDER, 0.0);

    // Torso in a racing crouch, chest down towards the tank.
    b.loft(
        &[
            sec(0.27, 1.0, 0.15, 0.1, 2.4),
            sec(0.16, 1.07, 0.17, 0.12, 2.4),
            sec(0.02, 1.18, 0.19, 0.12, 2.5),
            sec(-0.1, 1.27, 0.2, 0.12, 2.6),
            sec(-0.2, 1.31, 0.17, 0.1, 2.4),
            sec(-0.25, 1.32, 0.08, 0.06, 2.2),
        ],
        Vec3::X,
        14,
        &leather,
    );
    // Aero hump and a stripe down the spine in the bike's colour.
    b.loft(
        &[
            sec(0.16, 1.17, 0.05, 0.03, 2.2),
            sec(0.0, 1.29, 0.09, 0.07, 2.4),
            sec(-0.12, 1.38, 0.08, 0.05, 2.4),
            sec(-0.18, 1.39, 0.03, 0.02, 2.0),
        ],
        Vec3::X,
        10,
        &stripe,
    );
    b.sphere(
        Vec3::new(0.0, 1.03, 0.22),
        Vec3::new(0.16, 0.1, 0.13),
        5,
        10,
        &leather,
    );
    // Neck, helmet with a chin bar, visor and a small spoiler.
    b.limb(
        Vec3::new(0.0, 1.32, -0.22),
        Vec3::new(0.0, 1.38, -0.3),
        0.055,
        0.055,
        &leather,
    );
    let hc = Vec3::new(0.0, 1.46, -0.34);
    b.sphere(hc, Vec3::new(0.135, 0.14, 0.155), 8, 14, &helmet);
    b.loft(
        &[
            Section::new(hc + Vec3::new(0.0, -0.09, -0.04), 0.1, 0.05, 2.2),
            Section::new(hc + Vec3::new(0.0, -0.085, -0.13), 0.085, 0.05, 2.2),
            Section::new(hc + Vec3::new(0.0, -0.075, -0.165), 0.05, 0.035, 2.0),
        ],
        Vec3::X,
        12,
        &helmet,
    );
    b.sphere(
        hc + Vec3::new(0.0, 0.005, -0.075),
        Vec3::new(0.12, 0.06, 0.095),
        5,
        12,
        &visor,
    );
    b.loft(
        &[
            Section::new(hc + Vec3::new(0.0, 0.1, 0.1), 0.06, 0.012, 2.4),
            Section::new(hc + Vec3::new(0.0, 0.085, 0.17), 0.07, 0.014, 2.4),
        ],
        Vec3::X,
        10,
        &shell,
    );

    // Legs: hip -> knee (tucked into the tank) -> ankle on the peg.
    for (x, p) in [(1.0f32, part::RIGHT_LEG), (-1.0, part::LEFT_LEG)] {
        let leg = bike_mat([0.5, 0.5, 0.5], 0.5, 0.0, p, flag::PAINT2);
        let boot = Mat { kind: p, ..boots };
        let puck = Mat { kind: p, ..slider };
        let hip = if x > 0.0 { RIGHT_HIP } else { LEFT_HIP };
        let knee = Vec3::new(0.24 * x, 0.86, -0.2);
        let ankle = Vec3::new(0.2 * x, 0.55, 0.2);
        b.limb(hip, knee, 0.085, 0.065, &leg);
        b.limb(knee, ankle, 0.065, 0.045, &leg);
        b.sphere(
            knee + Vec3::new(0.05 * x, -0.02, -0.03),
            Vec3::new(0.025, 0.05, 0.06),
            3,
            8,
            &puck,
        );
        b.loft(
            &[
                Section::new(ankle + Vec3::new(0.0, 0.06, 0.03), 0.045, 0.05, 2.4),
                Section::new(ankle + Vec3::new(0.0, -0.06, 0.0), 0.045, 0.05, 2.4),
                Section::new(ankle + Vec3::new(0.0, -0.07, -0.12), 0.035, 0.035, 2.4),
            ],
            Vec3::X,
            10,
            &boot,
        );
    }
    // Arms to the clip-ons.
    for (x, p) in [(1.0f32, part::RIGHT_ARM), (-1.0, part::LEFT_ARM)] {
        let arm = bike_mat([0.5, 0.5, 0.5], 0.5, 0.0, p, flag::PAINT2);
        let pad = bike_mat([0.8, 0.8, 0.8], 0.35, 0.1, p, flag::PAINT);
        let glove = bike_mat([0.04, 0.04, 0.045], 0.6, 0.0, p, 0.0);
        let shoulder = if x > 0.0 {
            RIGHT_SHOULDER
        } else {
            LEFT_SHOULDER
        };
        let elbow = Vec3::new(0.3 * x, 1.12, -0.3);
        let hand = Vec3::new(0.31 * x, 0.97, -0.43);
        b.sphere(shoulder, Vec3::new(0.075, 0.07, 0.08), 4, 8, &pad);
        b.limb(shoulder, elbow, 0.06, 0.05, &arm);
        b.limb(elbow, hand, 0.05, 0.04, &arm);
        b.loft(
            &[
                Section::round(hand + Vec3::new(0.0, 0.03, 0.04), 0.042),
                Section::new(hand, 0.045, 0.04, 2.4),
                Section::new(hand + Vec3::new(0.0, -0.01, -0.06), 0.035, 0.03, 2.4),
            ],
            Vec3::X,
            8,
            &glove,
        );
    }
    weapons(b);
}

/// A club and a chain in each hand, gripped at the hand and held up and
/// back over the shoulder; the shader shows the one the rider carries.
fn weapons(b: &mut MeshBuilder) {
    for x in [1.0f32, -1.0] {
        let hand = Vec3::new(0.31 * x, 0.97, -0.43);
        let up = Vec3::new(0.12 * x, 0.62, 0.34).normalize();
        let (club_part, chain_part) = if x > 0.0 {
            (part::RIGHT_CLUB, part::RIGHT_CHAIN)
        } else {
            (part::LEFT_CLUB, part::LEFT_CHAIN)
        };
        // Club: a tapered wooden baton with a taped grip.
        let wood = bike_mat([0.36, 0.2, 0.09], 0.7, 0.0, club_part, 0.0);
        let tape = bike_mat([0.05, 0.05, 0.05], 0.8, 0.0, club_part, 0.0);
        b.limb(hand - up * 0.08, hand + up * 0.1, 0.022, 0.022, &tape);
        b.limb(hand + up * 0.1, hand + up * 0.62, 0.026, 0.042, &wood);
        b.sphere(hand + up * 0.62, Vec3::splat(0.042), 3, 6, &wood);
        // Chain: steel links hanging off a grip, swinging out behind.
        let steel = bike_mat([0.62, 0.63, 0.66], 0.25, 1.0, chain_part, 0.0);
        let grip = bike_mat([0.04, 0.04, 0.04], 0.8, 0.0, chain_part, 0.0);
        b.limb(hand - up * 0.06, hand + up * 0.08, 0.02, 0.02, &grip);
        let hang = Vec3::new(0.18 * x, 0.35, 0.5).normalize();
        let mut p = hand + up * 0.08;
        for k in 0..9 {
            let dir = (up * (1.0 - k as f32 / 9.0) + hang * (k as f32 / 9.0)).normalize();
            let q = p + dir * 0.075;
            b.limb(p, q, 0.014, 0.014, &steel);
            b.sphere(q, Vec3::splat(0.019), 2, 5, &steel);
            p = q;
        }
    }
}

/// Road car models, one per traffic kind (see `game::traffic::CarKind`),
/// in bike space (+x right, -z forward), sized to the game's boxes.
pub fn cars() -> [MeshBuilder; 4] {
    [
        car_body(0.85, 1.95, 0.0),
        car_body(0.9, 2.35, 1.0),
        van(),
        truck(),
    ]
}

fn car_mats() -> (Mat, Mat, Mat, Mat, Mat, Mat, Mat) {
    let paint = bike_mat([0.8, 0.8, 0.8], 0.22, 0.35, part::BODY, flag::PAINT);
    let glass = bike_mat([0.05, 0.06, 0.08], 0.05, 0.0, part::BODY, 0.0);
    let trim = bike_mat([0.04, 0.04, 0.045], 0.6, 0.1, part::BODY, 0.0);
    let tyre = bike_mat([0.025, 0.025, 0.025], 0.85, 0.0, part::BODY, 0.0);
    let hub = bike_mat([0.6, 0.6, 0.63], 0.3, 1.0, part::BODY, 0.0);
    let head = bike_mat([1.0, 0.95, 0.85], 0.2, 0.0, part::BODY, flag::HEADLIGHT);
    let tail = bike_mat([1.0, 0.05, 0.03], 0.2, 0.0, part::BODY, flag::TAILLIGHT);
    (paint, glass, trim, tyre, hub, head, tail)
}

fn car_wheels(b: &mut MeshBuilder, hw: f32, zs: &[f32], r: f32, tyre: &Mat, hub: &Mat) {
    for &z in zs {
        for x in [-1.0f32, 1.0] {
            let c = Vec3::new(x * (hw - 0.1), r, z);
            disc_x(b, c, 0.11, r, 12, tyre);
            disc_x(b, c + Vec3::X * x * 0.01, 0.105, r * 0.6, 8, hub);
        }
    }
}

/// A flat panel whose normal faces `out`, whatever order the corners came in.
fn quad_out(b: &mut MeshBuilder, out: Vec3, mut p: [Vec3; 4], m: &Mat) {
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(out) < 0.0 {
        p.reverse();
    }
    b.quad(p, m);
}

fn car_lights(b: &mut MeshBuilder, hw: f32, hl: f32, y: f32, head: &Mat, tail: &Mat) {
    for x in [-1.0f32, 1.0] {
        let cx = x * (hw - 0.22);
        quad_out(
            b,
            -Vec3::Z,
            [
                Vec3::new(cx - 0.16, y - 0.06, -hl - 0.01),
                Vec3::new(cx + 0.16, y - 0.06, -hl - 0.01),
                Vec3::new(cx + 0.16, y + 0.06, -hl - 0.01),
                Vec3::new(cx - 0.16, y + 0.06, -hl - 0.01),
            ],
            head,
        );
        quad_out(
            b,
            Vec3::Z,
            [
                Vec3::new(cx + 0.15, y - 0.05, hl + 0.01),
                Vec3::new(cx - 0.15, y - 0.05, hl + 0.01),
                Vec3::new(cx - 0.15, y + 0.06, hl + 0.01),
                Vec3::new(cx + 0.15, y + 0.06, hl + 0.01),
            ],
            tail,
        );
    }
}

/// Hatchback (`boot` 0) or saloon (`boot` 1): rounded lower body, a glass
/// cabin under a painted roof.
fn car_body(hw: f32, hl: f32, boot: f32) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let (paint, glass, trim, tyre, hub, head, tail) = car_mats();
    let sq = 4.0;
    b.loft(
        &[
            sec(hl, 0.55, hw * 0.92, 0.22, sq),
            sec(hl - 0.2, 0.62, hw, 0.32, sq),
            sec(-hl + 0.45, 0.62, hw, 0.31, sq),
            sec(-hl + 0.05, 0.52, hw * 0.94, 0.2, sq),
        ],
        Vec3::X,
        16,
        &paint,
    );
    // Bumpers.
    b.cube(
        Vec3::new(0.0, 0.38, hl - 0.02),
        Vec3::new(hw * 0.95, 0.08, 0.06),
        &trim,
    );
    b.cube(
        Vec3::new(0.0, 0.38, -hl + 0.02),
        Vec3::new(hw * 0.95, 0.08, 0.06),
        &trim,
    );
    // Cabin: glass greenhouse, painted roof.
    let rear = hl - 0.35 - boot * 0.55;
    let front = -hl * 0.22;
    b.loft(
        &[
            sec(rear + 0.05, 0.95, hw * 0.8, 0.04, 3.0),
            sec(rear - 0.15, 1.18, hw * 0.82, 0.26, 3.0),
            sec(front + 0.2, 1.2, hw * 0.84, 0.27, 3.0),
            sec(front - 0.55, 0.96, hw * 0.88, 0.04, 3.0),
        ],
        Vec3::X,
        12,
        &glass,
    );
    b.cuboid(
        Vec3::new(0.0, 1.44, (rear - 0.15 + front + 0.2) * 0.5),
        Vec3::new(hw * 0.8, 0.03, (rear - front - 0.35) * 0.5),
        Vec2::new(0.97, 0.95),
        &paint,
    );
    car_wheels(&mut b, hw, &[hl - 0.7, -hl + 0.7], 0.31, &tyre, &hub);
    car_lights(&mut b, hw, hl, 0.66, &head, &tail);
    b
}

fn van() -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let (paint, glass, trim, tyre, hub, head, tail) = car_mats();
    let (hw, hl) = (1.0, 2.6);
    b.loft(
        &[
            sec(hl, 1.15, hw * 0.98, 0.78, 6.0),
            sec(-hl + 0.9, 1.15, hw, 0.8, 6.0),
            sec(-hl + 0.25, 0.85, hw * 0.98, 0.5, 5.0),
            sec(-hl, 0.7, hw * 0.94, 0.32, 4.0),
        ],
        Vec3::X,
        16,
        &paint,
    );
    // Windscreen and side windows of the cab.
    quad_out(
        &mut b,
        Vec3::new(0.0, 0.5, -1.0),
        [
            Vec3::new(-hw * 0.88, 1.35, -hl + 0.62),
            Vec3::new(hw * 0.88, 1.35, -hl + 0.62),
            Vec3::new(hw * 0.86, 1.82, -hl + 0.93),
            Vec3::new(-hw * 0.86, 1.82, -hl + 0.93),
        ]
        .map(|p| p + Vec3::new(0.0, 0.0, -0.03)),
        &glass,
    );
    for x in [-1.0f32, 1.0] {
        let xx = x * (hw + 0.005);
        let q = [
            Vec3::new(xx, 1.35, -hl + 0.75),
            Vec3::new(xx, 1.35, -hl + 1.75),
            Vec3::new(xx, 1.8, -hl + 1.75),
            Vec3::new(xx, 1.8, -hl + 1.0),
        ];
        quad_out(&mut b, Vec3::X * x, q, &glass);
    }
    b.cube(
        Vec3::new(0.0, 0.4, hl - 0.02),
        Vec3::new(hw * 0.95, 0.09, 0.06),
        &trim,
    );
    b.cube(
        Vec3::new(0.0, 0.4, -hl - 0.02),
        Vec3::new(hw * 0.95, 0.09, 0.06),
        &trim,
    );
    car_wheels(&mut b, hw, &[hl - 0.75, -hl + 0.8], 0.34, &tyre, &hub);
    car_lights(&mut b, hw, hl, 0.72, &head, &tail);
    b
}

fn truck() -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let (paint, glass, trim, tyre, hub, head, tail) = car_mats();
    let (hw, hl) = (1.2, 4.2);
    let cargo = bike_mat([0.82, 0.82, 0.8], 0.6, 0.0, part::BODY, 0.0);
    // Cab.
    b.cuboid(
        Vec3::new(0.0, 1.45, -hl + 0.8),
        Vec3::new(hw, 0.95, 0.8),
        Vec2::new(0.96, 0.9),
        &paint,
    );
    quad_out(
        &mut b,
        Vec3::new(0.0, 0.3, -1.0),
        [
            Vec3::new(-hw * 0.85, 1.55, -hl - 0.005),
            Vec3::new(hw * 0.85, 1.55, -hl - 0.005),
            Vec3::new(hw * 0.82, 2.25, -hl + 0.15),
            Vec3::new(-hw * 0.82, 2.25, -hl + 0.15),
        ],
        &glass,
    );
    // Box body and chassis.
    b.cube(
        Vec3::new(0.0, 2.0, 1.0),
        Vec3::new(hw, 1.45, hl - 1.0),
        &cargo,
    );
    b.cube(
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::new(hw * 0.7, 0.15, hl - 0.1),
        &trim,
    );
    b.cube(
        Vec3::new(0.0, 0.55, -hl + 0.02),
        Vec3::new(hw * 0.98, 0.14, 0.06),
        &trim,
    );
    // A painted stripe down the box.
    for x in [-1.0f32, 1.0] {
        let xx = x * (hw + 0.006);
        let q = [
            Vec3::new(xx, 1.4, hl - 0.1),
            Vec3::new(xx, 1.4, -hl + 1.7),
            Vec3::new(xx, 1.7, -hl + 1.7),
            Vec3::new(xx, 1.7, hl - 0.1),
        ];
        quad_out(&mut b, Vec3::X * x, q, &paint);
    }
    car_wheels(
        &mut b,
        hw,
        &[hl - 0.9, hl - 2.1, -hl + 1.0],
        0.48,
        &tyre,
        &hub,
    );
    car_lights(&mut b, hw, hl, 0.75, &head, &tail);
    b
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
