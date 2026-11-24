//! Procedural placeholder bike sprites, packed through the `assets` crate's
//! atlas packer.
//!
//! Everything here is drawn in code into RGBA buffers — no binary art files
//! exist in the repo. Three frames cover the player state machine: riding,
//! braking, and crashed. They are placeholders in the Road Rash silhouette
//! sense: correct scale and pose, deliberately crude detail, to be replaced
//! by authored sprites through the same atlas pipeline.

use assets::PackOptions;
use image::RgbaImage;

/// Side-view frame size in pixels, shared by all placeholder bike frames.
pub const FRAME_WIDTH: u32 = 24;
/// Side-view frame size in pixels, shared by all placeholder bike frames.
pub const FRAME_HEIGHT: u32 = 32;

/// Alpha-zero pixel.
const CLEAR: [u8; 4] = [0, 0, 0, 0];
/// Asphalt-black tyre.
const TIRE: [u8; 4] = [24, 24, 26, 255];
/// Wheel hub grey.
const HUB: [u8; 4] = [130, 130, 136, 255];
/// Chrome/metal grey.
const METAL: [u8; 4] = [168, 172, 180, 255];
/// Helmet white — the player's.
const HELMET: [u8; 4] = [238, 238, 238, 255];
/// Brake-light red, lit.
const TAIL_LIT: [u8; 4] = [255, 42, 30, 255];
/// Scuff grey for the crash debris.
const SCUFF: [u8; 4] = [120, 116, 108, 255];
/// Foliage green, lit side.
const LEAF: [u8; 4] = [48, 128, 56, 255];
/// Foliage green, shadow side.
const LEAF_DARK: [u8; 4] = [30, 90, 40, 255];
/// Ground-shadow ink: dark green-grey, drawn at partial alpha per pixel.
const SHADOW_INK: [u8; 4] = [24, 34, 20, 255];
/// Trunk brown.
const TRUNK: [u8; 4] = [96, 64, 38, 255];
/// Sign-chevron red.
const SIGN_CHEVRON: [u8; 4] = [196, 38, 46, 255];
/// Warning-sign yellow.
const SIGN_YELLOW: [u8; 4] = [232, 188, 40, 255];
/// Traffic-car body white.
const CAR_BODY: [u8; 4] = [210, 212, 218, 255];
/// Traffic-car glass dark.
const CAR_GLASS: [u8; 4] = [40, 46, 58, 255];
/// Traffic-car taillight red.
const CAR_TAIL: [u8; 4] = [200, 40, 30, 255];
/// Traffic-cone orange.
const CONE_ORANGE: [u8; 4] = [232, 96, 22, 255];
/// Traffic-cone white band.
const CONE_WHITE: [u8; 4] = [240, 240, 238, 255];

/// The color scheme of one bike's rear-view sprite: fairing, leathers, and
/// helmet. The player and the rivals share one drawing routine but never a
/// palette, so the player reads at a glance (C3).
struct BikePalette {
    body: [u8; 4],
    body_dark: [u8; 4],
    suit: [u8; 4],
    suit_dark: [u8; 4],
    helmet: [u8; 4],
}

/// The player: red fairing, blue leathers, white helmet.
const PLAYER_PALETTE: BikePalette = BikePalette {
    body: [196, 38, 46, 255],
    body_dark: [138, 20, 26, 255],
    suit: [46, 78, 178, 255],
    suit_dark: [30, 52, 126, 255],
    helmet: HELMET,
};

/// The rivals: blue fairing, grey leathers, tinted helmet — clearly not the
/// player's red/white scheme.
const RIVAL_PALETTE: BikePalette = BikePalette {
    body: [40, 86, 198, 255],
    body_dark: [24, 54, 138, 255],
    suit: [116, 118, 126, 255],
    suit_dark: [84, 86, 94, 255],
    helmet: [70, 72, 80, 255],
};

/// Second rival livery: yellow fairing, black leathers.
const RIVAL_PALETTE_2: BikePalette = BikePalette {
    body: [222, 176, 30, 255],
    body_dark: [160, 122, 16, 255],
    suit: [44, 44, 50, 255],
    suit_dark: [28, 28, 34, 255],
    helmet: [230, 230, 226, 255],
};

/// Third rival livery: green fairing, cream leathers.
const RIVAL_PALETTE_3: BikePalette = BikePalette {
    body: [40, 148, 82, 255],
    body_dark: [22, 102, 54, 255],
    suit: [222, 214, 190, 255],
    suit_dark: [180, 172, 150, 255],
    helmet: [50, 50, 56, 255],
};

/// Rival liveries, indexed by `(slot - 1) % 3` so the pack never shows two
/// identical bikes side by side.
const RIVAL_PALETTES: [&BikePalette; 3] = [&RIVAL_PALETTE, &RIVAL_PALETTE_2, &RIVAL_PALETTE_3];

/// Fill an inclusive pixel rectangle, clipping to the image.
fn fill_rect(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
    for y in y0.max(0)..=y1.min(img.height() as i32 - 1) {
        for x in x0.max(0)..=x1.min(img.width() as i32 - 1) {
            img.put_pixel(x as u32, y as u32, color.into());
        }
    }
}

/// Fill a pixel disc, clipping to the image.
fn fill_disc(img: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
    for y in (cy - radius).max(0)..=(cy + radius).min(img.height() as i32 - 1) {
        for x in (cx - radius).max(0)..=(cx + radius).min(img.width() as i32 - 1) {
            let (dx, dy) = ((x - cx) * (x - cx), (y - cy) * (y - cy));
            if dx + dy <= radius * radius {
                img.put_pixel(x as u32, y as u32, color.into());
            }
        }
    }
}

fn blank_frame() -> RgbaImage {
    RgbaImage::from_fn(FRAME_WIDTH, FRAME_HEIGHT, |_, _| CLEAR.into())
}

/// Rear-view bike drawing shared by the player and the rivals: the camera
/// chases from behind, so the frames show the rear wheel, the rider's back
/// and shoulders, the handlebar grips at the frame edges, and an exhaust
/// hint flanking the wheel (D1).
fn riding_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = blank_frame();
    // Rear tyre: a disc with a small hub, flanked by exhaust hints.
    fill_disc(&mut img, 12, 27, 4, TIRE);
    fill_rect(&mut img, 11, 28, 13, 30, HUB);
    fill_rect(&mut img, 6, 24, 8, 29, METAL);
    fill_rect(&mut img, 16, 24, 18, 29, METAL);
    // Tail fairing over the wheel; dark edge at the bottom.
    fill_rect(&mut img, 8, 18, 16, 23, palette.body);
    fill_rect(&mut img, 8, 22, 16, 23, palette.body_dark);
    // Unlit tail light slot.
    fill_rect(&mut img, 10, 19, 14, 21, palette.body_dark);
    // Rider's back, spine-shaded on the right.
    fill_rect(&mut img, 9, 9, 15, 18, palette.suit);
    fill_rect(&mut img, 13, 10, 15, 18, palette.suit_dark);
    // Shoulders, shaded underneath.
    fill_rect(&mut img, 7, 8, 17, 12, palette.suit);
    fill_rect(&mut img, 7, 11, 17, 12, palette.suit_dark);
    // Arms reaching out to the bars.
    fill_rect(&mut img, 4, 11, 8, 14, palette.suit);
    fill_rect(&mut img, 16, 11, 20, 14, palette.suit);
    // Handlebar grips visible at the frame edges.
    fill_rect(&mut img, 2, 10, 5, 13, METAL);
    fill_rect(&mut img, 19, 10, 22, 13, METAL);
    // Helmet above the shoulders.
    fill_disc(&mut img, 12, 5, 3, palette.helmet);
    img
}

/// The braking frame: rider crouched low over the tank, tail light lit.
fn braking_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = riding_frame(palette);
    // Crouch: clear the upright torso, shoulders, and helmet, then redraw
    // them lower and wider.
    fill_rect(&mut img, 2, 3, 22, 18, CLEAR);
    // Arms bracing low to the grips.
    fill_rect(&mut img, 4, 13, 8, 17, palette.suit);
    fill_rect(&mut img, 16, 13, 20, 17, palette.suit);
    // Grips stay at the edges.
    fill_rect(&mut img, 2, 12, 5, 15, METAL);
    fill_rect(&mut img, 19, 12, 22, 15, METAL);
    // Hunched torso and lowered shoulders.
    fill_rect(&mut img, 8, 13, 16, 19, palette.suit);
    fill_rect(&mut img, 8, 17, 16, 19, palette.suit_dark);
    fill_rect(&mut img, 6, 12, 18, 15, palette.suit);
    // Helmet tucked down between the shoulders.
    fill_disc(&mut img, 12, 10, 3, palette.helmet);
    // Lit tail light: braking.
    fill_rect(&mut img, 10, 19, 14, 21, TAIL_LIT);
    img
}

/// The crash frame: bike on its side, rider sprawled behind it, scuff dust.
/// A downed rider is seen from behind as a sprawl next to the fallen bike.
fn crash_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = blank_frame();
    // Bike lying on its right side: wheels side by side, body flat between.
    fill_disc(&mut img, 5, 26, 4, TIRE);
    fill_disc(&mut img, 5, 26, 2, HUB);
    fill_disc(&mut img, 19, 26, 4, TIRE);
    fill_disc(&mut img, 19, 26, 2, HUB);
    fill_rect(&mut img, 8, 23, 17, 27, palette.body);
    fill_rect(&mut img, 8, 25, 17, 27, palette.body_dark);
    // Handlebar sticking up.
    fill_rect(&mut img, 15, 19, 20, 21, METAL);
    // Rider sprawled behind the bike, above it on the ground.
    fill_rect(&mut img, 8, 15, 16, 22, palette.suit);
    fill_rect(&mut img, 8, 20, 11, 22, palette.suit_dark);
    fill_disc(&mut img, 12, 12, 3, palette.helmet);
    // Scuff dust kicked up along the slide.
    fill_rect(&mut img, 2, 24, 4, 26, SCUFF);
    fill_rect(&mut img, 21, 22, 22, 23, SCUFF);
    img
}

/// All placeholder frames as `(name, image)` pairs, ready for the packer.
pub fn placeholder_frames() -> Vec<(String, RgbaImage)> {
    vec![
        ("bike_ride".to_owned(), riding_frame(&PLAYER_PALETTE)),
        ("bike_brake".to_owned(), braking_frame(&PLAYER_PALETTE)),
        ("bike_crash".to_owned(), crash_frame(&PLAYER_PALETTE)),
        ("bike_rival".to_owned(), riding_frame(RIVAL_PALETTES[0])),
        ("bike_rival2".to_owned(), riding_frame(RIVAL_PALETTES[1])),
        ("bike_rival3".to_owned(), riding_frame(RIVAL_PALETTES[2])),
        ("tree".to_owned(), tree_frame()),
        ("tree2".to_owned(), broadleaf_frame()),
        ("tree3".to_owned(), tall_pine_frame()),
        ("bush".to_owned(), bush_frame()),
        ("sign".to_owned(), sign_frame()),
        ("car".to_owned(), car_frame()),
        ("cone".to_owned(), cone_frame()),
        ("shadow".to_owned(), shadow_frame()),
    ]
}

/// A soft elliptical ground shadow: dark ink fading to transparent at the
/// rim, alpha-blended under trees, bikes, and cars (D3).
fn shadow_frame() -> RgbaImage {
    let (w, h) = (32_u32, 10_u32);
    let (cx, cy) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    let (rx, ry) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    RgbaImage::from_fn(w, h, |x, y| {
        let dx = (x as f32 - cx) / rx;
        let dy = (y as f32 - cy) / ry;
        let r2 = dx * dx + dy * dy;
        if r2 <= 1.0 {
            let alpha = (255.0 * (1.0 - r2) * 0.55) as u8;
            [SHADOW_INK[0], SHADOW_INK[1], SHADOW_INK[2], alpha].into()
        } else {
            CLEAR.into()
        }
    })
}

/// A roadside pine: trunk and a two-tier canopy. 24x40, anchored on the road.
fn tree_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(24, 40, |_, _| CLEAR.into());
    fill_rect(&mut img, 10, 30, 13, 39, TRUNK);
    // Two canopy tiers, wider at the bottom, shadow on the left.
    fill_rect(&mut img, 4, 18, 19, 30, LEAF);
    fill_rect(&mut img, 4, 18, 9, 30, LEAF_DARK);
    fill_rect(&mut img, 7, 6, 16, 18, LEAF);
    fill_rect(&mut img, 7, 6, 10, 18, LEAF_DARK);
    fill_rect(&mut img, 10, 0, 13, 6, LEAF);
    img
}

/// A broadleaf: short trunk under a wide round canopy. 28x34.
fn broadleaf_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(28, 34, |_, _| CLEAR.into());
    fill_rect(&mut img, 12, 24, 15, 33, TRUNK);
    // A big round crown: layered discs, shade on the lower left.
    fill_disc(&mut img, 14, 13, 12, LEAF);
    fill_disc(&mut img, 9, 16, 6, LEAF_DARK);
    fill_disc(&mut img, 19, 9, 6, LEAF);
    fill_disc(&mut img, 11, 8, 5, LEAF);
    fill_disc(&mut img, 16, 14, 5, LEAF_DARK);
    img
}

/// A tall slim pine: narrow spire, three stacked skirts. 16x48.
fn tall_pine_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(16, 48, |_, _| CLEAR.into());
    fill_rect(&mut img, 7, 40, 9, 47, TRUNK);
    fill_rect(&mut img, 3, 32, 12, 40, LEAF);
    fill_rect(&mut img, 3, 36, 7, 40, LEAF_DARK);
    fill_rect(&mut img, 4, 22, 11, 32, LEAF);
    fill_rect(&mut img, 4, 27, 7, 32, LEAF_DARK);
    fill_rect(&mut img, 5, 12, 10, 22, LEAF);
    fill_rect(&mut img, 5, 17, 8, 22, LEAF_DARK);
    fill_rect(&mut img, 6, 2, 9, 12, LEAF);
    img
}

/// A low roadside bush. 16x10.
fn bush_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(16, 10, |_, _| CLEAR.into());
    fill_rect(&mut img, 2, 4, 13, 9, LEAF);
    fill_rect(&mut img, 2, 4, 6, 9, LEAF_DARK);
    fill_disc(&mut img, 5, 4, 3, LEAF);
    fill_disc(&mut img, 11, 4, 3, LEAF);
    fill_disc(&mut img, 5, 4, 1, LEAF_DARK);
    img
}

/// A roadside warning sign: pole and a chevron board. 18x24.
fn sign_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(18, 24, |_, _| CLEAR.into());
    fill_rect(&mut img, 8, 12, 10, 23, METAL);
    fill_rect(&mut img, 2, 2, 15, 12, SIGN_YELLOW);
    // Chevron pointing right (corners bend right around here).
    for step in 0..5 {
        fill_rect(
            &mut img,
            4 + step,
            4 + step,
            5 + step,
            10 - step,
            SIGN_CHEVRON,
        );
    }
    img
}

/// A traffic car from behind: the view the player actually sees. 26x18.
fn car_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(26, 18, |_, _| CLEAR.into());
    // Body and cabin.
    fill_rect(&mut img, 2, 6, 23, 16, CAR_BODY);
    fill_rect(&mut img, 5, 1, 20, 7, CAR_BODY);
    // Rear window.
    fill_rect(&mut img, 7, 2, 18, 6, CAR_GLASS);
    // Bumper shade.
    fill_rect(&mut img, 2, 13, 23, 16, CAR_GLASS);
    // Taillights.
    fill_rect(&mut img, 3, 8, 6, 10, CAR_TAIL);
    fill_rect(&mut img, 19, 8, 22, 10, CAR_TAIL);
    // Tyres.
    fill_rect(&mut img, 3, 16, 7, 17, TIRE);
    fill_rect(&mut img, 18, 16, 22, 17, TIRE);
    img
}

/// A traffic cone: the on-road hazard. 10x12.
fn cone_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(10, 12, |_, _| CLEAR.into());
    fill_rect(&mut img, 2, 10, 7, 11, CONE_ORANGE);
    fill_rect(&mut img, 4, 6, 5, 9, CONE_ORANGE);
    fill_rect(&mut img, 3, 5, 6, 6, CONE_WHITE);
    fill_rect(&mut img, 4, 1, 5, 4, CONE_ORANGE);
    img
}

/// Packs the placeholder frames into a deterministic atlas through the
/// `assets` crate — the same packer authored art will go through.
pub fn pack_placeholder_atlas() -> Result<assets::PackedAtlas, assets::PackError> {
    assets::pack(PackOptions::default(), &placeholder_frames())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assets::gradient::is_smooth_gradient_shaded;

    /// The original pixel-art frame scale the HD direction replaces.
    const PIXEL_ART_FRAME: (u32, u32) = (24, 32);

    #[test]
    fn frames_render_at_hd_resolution() {
        assert!(
            FRAME_WIDTH >= 3 * PIXEL_ART_FRAME.0 && FRAME_HEIGHT >= 3 * PIXEL_ART_FRAME.1,
            "frames are {FRAME_WIDTH}x{FRAME_HEIGHT}: HD direction wants at least 3x the \
             pixel-art scale ({:?})",
            PIXEL_ART_FRAME
        );
    }

    #[test]
    fn every_frame_is_smooth_gradient_shaded() {
        for (name, img) in placeholder_frames() {
            assert!(
                is_smooth_gradient_shaded(&img),
                "'{name}' is flat or banded — HD sprites shade smoothly, no pixel art"
            );
        }
    }

    #[test]
    fn atlas_regeneration_is_deterministic() {
        let first = pack_placeholder_atlas().expect("placeholder atlas packs");
        let second = pack_placeholder_atlas().expect("placeholder atlas packs");
        assert_eq!(first.atlas.dimensions(), second.atlas.dimensions());
        assert!(
            first
                .atlas
                .pixels()
                .zip(second.atlas.pixels())
                .all(|(p, q)| p == q),
            "same frames, same atlas pixels — dressing and packing stay pure functions"
        );
    }
}
