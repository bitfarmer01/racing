//! Gradient-shading analysis: does a sprite read as smoothly shaded HD art
//! rather than flat or hard-banded pixel art? Pure functions over RGBA, so
//! the HD direction is testable on the host without a device.
//!
//! The metric separates two signals:
//!
//! - **Span** — the shading range across the sprite. Flat fills have none.
//! - **Flat fraction** — the share of opaque pixels whose every opaque
//!   4-neighbour has the same luminance. Flat fills and quantized banding
//!   are almost entirely such pixels; gradient shading almost none.
//!
//! Material boundaries are deliberately invisible to the metric: a dark
//! tyre meeting a bright fairing is one hard edge between two shaded
//! regions, which is legitimate HD art — what the metric rejects is the
//! flat/banded *interior* the pixel-art look is made of.

/// Thresholds for [`is_smooth_gradient_shaded`], as tunable numbers rather
/// than magic constants in the predicate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientCheck {
    /// Smallest channel spread (max − min over opaque pixels, per channel)
    /// that counts as "actually shaded, not a flat fill".
    pub min_span: u32,
    /// Largest share of opaque pixels allowed to sit in locally flat
    /// regions (all opaque 4-neighbours at the same luminance).
    pub max_flat_frac: f32,
}

impl GradientCheck {
    /// The HD bar: real shading spread, and shading almost everywhere.
    pub const fn hd() -> Self {
        Self {
            min_span: 24,
            max_flat_frac: 0.30,
        }
    }
}

/// Integer luminance of one pixel, `0..=255`. Pure.
fn luminance(px: [u8; 4]) -> u32 {
    (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) / 3
}

/// Shading statistics over one sprite's opaque pixels (alpha > 0):
/// `(channel_span, flat_fraction)`. The span is the widest per-channel
/// spread; the flat fraction is the share of opaque pixels whose opaque
/// 4-neighbours all share their luminance. Isolated pixels (no opaque
/// neighbours) count as neither. Pure.
pub fn gradient_shading_stats(img: &image::RgbaImage) -> (u32, f32) {
    let (w, h) = (img.width(), img.height());
    let mut min = [u8::MAX; 3];
    let mut max = [u8::MIN; 3];
    let mut opaque = 0u64;
    let mut flat = 0u64;

    let lum = |x: u32, y: u32| -> Option<u32> {
        let px: [u8; 4] = img.get_pixel(x, y).0;
        if px[3] > 0 {
            Some(luminance(px))
        } else {
            None
        }
    };

    for y in 0..h {
        for x in 0..w {
            let Some(here) = lum(x, y) else { continue };
            let px: [u8; 4] = img.get_pixel(x, y).0;
            for c in 0..3 {
                min[c] = min[c].min(px[c]);
                max[c] = max[c].max(px[c]);
            }
            opaque += 1;
            let mut has_neighbour = false;
            let mut all_equal = true;
            for (nx, ny) in [
                (x.checked_sub(1), Some(y)),
                (Some(x + 1), Some(y)),
                (Some(x), y.checked_sub(1)),
                (Some(x), Some(y + 1)),
            ] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                if nx >= w || ny >= h {
                    continue;
                }
                if let Some(there) = lum(nx, ny) {
                    has_neighbour = true;
                    all_equal &= there == here;
                }
            }
            if has_neighbour && all_equal {
                flat += 1;
            }
        }
    }

    let span = max
        .iter()
        .zip(min.iter())
        .map(|(hi, lo)| u32::from(*hi) - u32::from(*lo))
        .max()
        .unwrap_or(0);
    let flat_frac = if opaque == 0 {
        0.0
    } else {
        flat as f32 / opaque as f32
    };
    (span, flat_frac)
}

/// Whether `img` reads as smoothly gradient-shaded under the HD check.
/// Pure.
pub fn is_smooth_gradient_shaded(img: &image::RgbaImage) -> bool {
    is_smooth_gradient_under(img, GradientCheck::hd())
}

/// [`is_smooth_gradient_shaded`] with explicit thresholds, so the predicate
/// itself is testable at the boundaries.
pub fn is_smooth_gradient_under(img: &image::RgbaImage, check: GradientCheck) -> bool {
    let (span, flat_frac) = gradient_shading_stats(img);
    span >= check.min_span && flat_frac <= check.max_flat_frac
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
        // A gradient quantized into 4 hard bands: the pixel-art staircase.
        RgbaImage::from_fn(32, 32, |x, y| {
            let band = ((x + y) / 16) as u8; // 0..=4
            let v = band.saturating_mul(63);
            [v, v, v, 255].into()
        })
    }

    /// A stand-in for the retired pixel-art look: small flat regions of
    /// different colors meeting at hard edges, like the original 24x32
    /// sprite frames.
    fn pixel_art_sprawl() -> RgbaImage {
        RgbaImage::from_fn(24, 32, |x, y| {
            if y < 4 {
                [238, 238, 238, 255].into()
            } else if x < 8 {
                [196, 38, 46, 255].into()
            } else if x < 16 {
                [46, 78, 178, 255].into()
            } else {
                [24, 24, 26, 255].into()
            }
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
    fn a_pixel_art_sprawl_fails() {
        assert!(
            !is_smooth_gradient_shaded(&pixel_art_sprawl()),
            "flat regions behind hard edges — the retired pixel-art look — must fail"
        );
    }

    #[test]
    fn material_boundaries_do_not_defeat_the_check() {
        // Two smoothly shaded halves meeting at one hard edge — like a tyre
        // against a fairing — must still read as gradient-shaded: the
        // boundary touches two pixel rows, the shading covers everything.
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
