//! View camera with a reversed, infinite-far depth projection.

use glam::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub pos: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub near: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pos: Vec3::new(0.0, 10.0, -20.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y: 1.1,
            near: 0.25,
        }
    }
}

impl Camera {
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.pos, self.target, self.up)
    }

    /// Reversed-Z: depth 1 at the near plane, 0 at infinity.
    pub fn proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(self.fov_y, aspect, self.near)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    pub fn forward(&self) -> Vec3 {
        (self.target - self.pos).normalize_or(Vec3::Z)
    }
}

/// Five clip planes (no far plane: the projection is infinite).
#[derive(Debug, Clone, Copy)]
pub struct Frustum {
    planes: [Vec4; 5],
}

impl Frustum {
    pub fn from_view_proj(m: Mat4) -> Self {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let mut planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 - r2];
        for p in &mut planes {
            let l = p.truncate().length();
            if l > 0.0 {
                *p /= l;
            }
        }
        Self { planes }
    }

    /// Orthographic shadow frustum: all six planes are proper.
    pub fn from_ortho(m: Mat4) -> [Vec4; 6] {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let mut planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
        for p in &mut planes {
            let l = p.truncate().length();
            if l > 0.0 {
                *p /= l;
            }
        }
        planes
    }

    pub fn aabb_visible(&self, lo: Vec3, hi: Vec3) -> bool {
        aabb_in_planes(&self.planes, lo, hi)
    }
}

pub fn aabb_in_planes(planes: &[Vec4], lo: Vec3, hi: Vec3) -> bool {
    for p in planes {
        let n = p.truncate();
        // Farthest corner along the plane normal.
        let v = Vec3::new(
            if n.x >= 0.0 { hi.x } else { lo.x },
            if n.y >= 0.0 { hi.y } else { lo.y },
            if n.z >= 0.0 { hi.z } else { lo.z },
        );
        if n.dot(v) + p.w < 0.0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frustum_culls_behind_and_keeps_ahead() {
        let cam = Camera {
            pos: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            ..Default::default()
        };
        let f = Frustum::from_view_proj(cam.view_proj(16.0 / 9.0));
        assert!(f.aabb_visible(Vec3::new(-1.0, -1.0, -11.0), Vec3::new(1.0, 1.0, -9.0)));
        assert!(!f.aabb_visible(Vec3::new(-1.0, -1.0, 9.0), Vec3::new(1.0, 1.0, 11.0)));
        assert!(!f.aabb_visible(Vec3::new(200.0, -1.0, -11.0), Vec3::new(201.0, 1.0, -9.0)));
    }
}
