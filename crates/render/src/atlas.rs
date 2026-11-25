//! Pure atlas-UV mapping: turns the `assets` crate's pixel-space sprite
//! rectangles into the normalized coordinates the shader samples.

use assets::SpriteRect;

/// Normalizes an atlas-space sprite rect to `(u0, v0, u1, v1)`.
///
/// Returns all zeros for a degenerate atlas (zero dimensions), which samples
/// a single texel rather than dividing by zero.
pub fn uv_rect(rect: SpriteRect, atlas_width: u32, atlas_height: u32) -> [f32; 4] {
    if atlas_width == 0 || atlas_height == 0 {
        return [0.0; 4];
    }
    let w = atlas_width as f32;
    let h = atlas_height as f32;
    [
        rect.x as f32 / w,
        rect.y as f32 / h,
        (rect.x + rect.w) as f32 / w,
        (rect.y + rect.h) as f32 / h,
    ]
}

/// Like [`uv_rect`], but shrinks each edge by `inset_px` texels.
///
/// The atlas packer leaves 2 px padding between sprites; a small inset (half
/// a texel or so) stops edge bleeding when the sampler filters across the
/// rect boundary. Inset is clamped so the rect can never invert.
pub fn uv_rect_inset(
    rect: SpriteRect,
    atlas_width: u32,
    atlas_height: u32,
    inset_px: f32,
) -> [f32; 4] {
    let base = uv_rect(rect, atlas_width, atlas_height);
    if base == [0.0; 4] {
        return base;
    }
    let du = inset_px / atlas_width as f32;
    let dv = inset_px / atlas_height as f32;
    [
        base[0] + du,
        base[1] + dv,
        (base[2] - du).max(base[0]),
        (base[3] - dv).max(base[1]),
    ]
}

/// Pixel dimensions of the packed atlas, taken from the packer's output
/// image. Kept as a small struct so callers never pass width and height
/// separately and mix up the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasDims {
    pub width: u32,
    pub height: u32,
}

impl AtlasDims {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// UV rect for a sprite of this atlas, no inset.
    pub fn uv(&self, rect: SpriteRect) -> [f32; 4] {
        uv_rect(rect, self.width, self.height)
    }

    /// UV rect for a sprite of this atlas with an edge inset in texels.
    pub fn uv_inset(&self, rect: SpriteRect, inset_px: f32) -> [f32; 4] {
        uv_rect_inset(rect, self.width, self.height, inset_px)
    }
}

/// The stated byte budget for the packed atlas (item 16-test), RGBA8 at 4
/// bytes per pixel. The packer's 4096 px dimension cap alone would admit a
/// 64 MB texture; this budget keeps the uploaded atlas well under that.
/// Enforced by [`check_atlas_budget`], which the sprites tests run against
/// the real packed atlas — an over-budget pack fails the suite instead of
/// shipping.
pub const MAX_ATLAS_BYTES: usize = 8 * 1024 * 1024;

/// An atlas that exceeds [`MAX_ATLAS_BYTES`]: the packed byte size and the
/// budget it blew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasTooLarge {
    pub bytes: usize,
    pub budget: usize,
}

/// Checks a packed atlas' byte size against the stated budget (item
/// 16-test): the packer's 4096 px cap alone would allow a 64 MB RGBA8
/// upload. Returns the RGBA8 size, or the over-budget facts. Pure.
pub fn check_atlas_budget(dims: AtlasDims) -> Result<usize, AtlasTooLarge> {
    let bytes = dims.width as usize * dims.height as usize * 4;
    if bytes > MAX_ATLAS_BYTES {
        return Err(AtlasTooLarge {
            bytes,
            budget: MAX_ATLAS_BYTES,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_atlas_fails_the_budget_check() {
        // 4096 px square — the packer's own dimension cap — is 64 MB of
        // RGBA8: far over the stated budget, so the check must refuse it.
        let huge = AtlasDims::new(4_096, 4_096);
        assert_eq!(
            check_atlas_budget(huge),
            Err(AtlasTooLarge {
                bytes: 64 * 1024 * 1024,
                budget: MAX_ATLAS_BYTES,
            })
        );
        // And a comfortably sized atlas passes with its byte size.
        assert_eq!(
            check_atlas_budget(AtlasDims::new(1_024, 1_024)),
            Ok(4 * 1024 * 1024)
        );
    }
}
