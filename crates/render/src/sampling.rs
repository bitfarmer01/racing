//! Atlas sampling policy: the render path's texture-filtering decisions in
//! one pure module, so the HD look is specified and tested without a device.
//!
//! The visual direction is modern HD 2.5D — smooth gradients, no pixelation
//! (wave 7 decision, 2026-11-24). That reverses the pixel-art choice that
//! picked nearest sampling: sprites are shaded at high resolution and must
//! sample smoothly as they scale.

use wgpu::FilterMode;

/// How the atlas is sampled, mag and min. Linear (bilinear) filtering
/// smooths the HD sprite shading as it scales; nearest re-introduces the
/// texel stair-stepping the HD direction retired.
pub const fn atlas_filter_mode() -> FilterMode {
    FilterMode::Linear
}

/// Lower edge of the fragment shader's smooth alpha ramp: coverage at or
/// below this is fully discarded, everything above blends. Keep in sync
/// with the `smoothstep(0.0, 0.06, …)` in `SPRITE_SHADER` — the WGSL string
/// is a const, so the ramp values cannot be interpolated into it.
pub const ALPHA_RAMP_HI: f32 = 0.06;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::SPRITE_SHADER;

    #[test]
    fn atlas_is_sampled_linear_not_nearest() {
        assert_eq!(
            atlas_filter_mode(),
            FilterMode::Linear,
            "HD direction: smooth filtered sampling — nearest re-pixelates the scaled atlas"
        );
    }

    #[test]
    fn sprite_fragment_shader_blends_smooth_edges_instead_of_hard_cutting() {
        // The smooth-silhouette look dies if the fragment shader discards
        // semi-transparent edge pixels with a pixel-art hard cut; the
        // shader must ramp alpha smoothly and let blending do the edge.
        assert!(
            !SPRITE_SHADER.contains("color.a < 0.01"),
            "sprite shader must not hard-cut alpha at 0.01 — smooth silhouettes blend"
        );
        assert!(
            SPRITE_SHADER.contains("smoothstep"),
            "sprite shader must apply a smooth alpha ramp to the sampled coverage"
        );
        assert!(
            SPRITE_SHADER.contains("textureSample"),
            "sprite shader must sample the atlas texture"
        );
    }
}
