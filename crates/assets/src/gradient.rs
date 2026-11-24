//! Gradient-shading analysis: does a sprite read as smoothly shaded HD art
//! rather than flat or hard-banded pixel art? Pure functions over RGBA, so
//! the HD direction is testable on the host without a device.
//!
//! The metric must tolerate material boundaries — a dark tyre meeting a
//! bright fairing is a legitimate hard edge — while still rejecting flat
//! fills and banded (quantized) shading. It therefore scores only the
//! *gradual* fraction of adjacent-pixel color steps inside the sprite:
//! smooth shading makes nearly every nonzero step small; banding makes them
//! all large at once.

/// Thresholds for [`is_smooth_gradient_shaded`], as tunable numbers rather
/// than magic constants in the predicate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientCheck {
    /// Smallest channel spread (max − min over opaque pixels, per channel)
    /// that counts as "actually shaded, not a flat fill".
    pub min_span: u32,
    /// Largest per-channel step between adjacent opaque pixels that still
    /// counts as gradual (smooth shading, not a band boundary).
    pub gradual_step: u32,
    /// Fraction of nonzero adjacent steps that must be gradual.
    pub min_gradual_frac: f32,
}

impl GradientCheck {
    /// The HD bar: real shading spread, steps that never jump a band.
    pub const fn hd() -> Self {
        Self {
            min_span: 24,
            gradual_step: 12,
            min_gradual_frac: 0.9,
        }
    }
}

/// Shading statistics over one sprite's opaque pixels (alpha > 0):
/// `(channel_span, gradual_fraction)`. `gradual_fraction` is `None` when no
/// adjacent opaque pair differs at all — a flat fill, shaded nowhere.
/// A step counts as gradual when it stays within `gradual_step` levels.
/// Pure.
pub fn gradient_shading_stats(img: &image::RgbaImage, gradual_step: u32) -> (u32, Option<f32>) {
    let (w, h) = (img.width(), img.height());
    let mut min = [u8::MAX; 3];
    let mut max = [u8::MIN; 3];
    let mut nonzero_steps = 0u64;
    let mut gradual_steps = 0u64;

    let mut visit = |a: [u8; 4], b: [u8; 4]| {
        if a[3] > 0 && b[3] > 0 {
            for c in 0..3 {
                min[c] = min[c].min(a[c]).min(b[c]);
                max[c] = max[c].max(a[c]).max(b[c]);
            }
            let step = a
                .iter()
                .zip(b.iter())
                .take(3)
                .map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs())
                .max()
                .unwrap_or(0);
            if step > 0 {
                nonzero_steps += 1;
                if step as u32 <= gradual_step {
                    gradual_steps += 1;
                }
            }
        }
    };
    for y in 0..h {
        for x in 0..w {
            let a: [u8; 4] = img.get_pixel(x, y).0;
            if x + 1 < w {
                visit(a, img.get_pixel(x + 1, y).0);
            }
            if y + 1 < h {
                visit(a, img.get_pixel(x, y + 1).0);
            }
        }
    }

    let span = max
        .iter()
        .zip(min.iter())
        .map(|(hi, lo)| u32::from(*hi) - u32::from(*lo))
        .max()
        .unwrap_or(0);
    let gradual_frac = if nonzero_steps == 0 {
        None
    } else {
        Some(gradual_steps as f32 / nonzero_steps as f32)
    };
    (span, gradual_frac)
}

/// Whether `img` reads as smoothly gradient-shaded under the HD check.
/// Pure.
pub fn is_smooth_gradient_shaded(img: &image::RgbaImage) -> bool {
    is_smooth_gradient_under(img, GradientCheck::hd())
}

/// [`is_smooth_gradient_shaded`] with explicit thresholds, so the predicate
/// itself is testable at the boundaries.
pub fn is_smooth_gradient_under(img: &image::RgbaImage, check: GradientCheck) -> bool {
    let (span, gradual_frac) = gradient_shading_stats(img, check.gradual_step);
    if span < check.min_span {
        return false;
    }
    match gradual_frac {
        Some(frac) => frac >= check.min_gradual_frac,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn flat(color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_fn(32, 32, |_, _| color.into())
    }

    fn smooth_gradient() -> RgbaImage {
        RgbaImage::from_fn(32, 32, |x, y| {
            let t = (x + y) as f32 / 62.0;
            let v = (t * 255.0) as u8;
            [v, v, v, 255].into()
        })
    }

    fn banded_gradient() -> RgbaImage {
        // A gradient quantized into 4 hard bands: big jumps, no gradual steps.
        RgbaImage::from_fn(32, 32, |x, y| {
            let band = ((x + y) / 16) as u8; // 0..=4
            let v = band.saturating_mul(63);
            [v, v, v, 255].into()
        })
    }

    #[test]
    fn a_flat_fill_is_not_gradient_shaded() {
        assert!(!is_smooth_gradient_shaded(&flat([90, 90, 90, 255])));
    }

    #[test]
    fn a_smooth_gradient_passes() {
        assert!(is_smooth_gradient_shaded(&smooth_gradient()));
    }

    #[test]
    fn a_banded_gradient_fails() {
        assert!(
            !is_smooth_gradient_shaded(&banded_gradient()),
            "quantized banding is exactly the pixel-art look the HD direction retires"
        );
    }

    #[test]
    fn material_boundaries_do_not_defeat_the_check() {
        // Two smoothly shaded halves meeting at one hard edge — like a tyre
        // against a fairing — must still read as gradient-shaded: the single
        // boundary step is diluted by the many gradual steps inside each half.
        let mut img = RgbaImage::from_fn(32, 32, |x, y| {
            if x < 16 {
                let v = (y as f32 / 31.0 * 80.0) as u8;
                [v, v, v, 255].into()
            } else {
                let v = (200.0 - (y as f32 / 31.0 * 80.0)) as u8;
                [v, v, v, 255].into()
            }
        });
        assert!(is_smooth_gradient_shaded(&img));
        // And a little transparent padding around the art must not matter:
        img = RgbaImage::from_fn(34, 34, |x, y| {
            if x == 0 || y == 0 || x == 33 || y == 33 {
                [0, 0, 0, 0].into()
            } else {
                img.get_pixel(x - 1, y - 1).to_owned()
            }
        });
        assert!(is_smooth_gradient_shaded(&img));
    }
}
