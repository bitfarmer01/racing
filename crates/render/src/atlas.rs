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
