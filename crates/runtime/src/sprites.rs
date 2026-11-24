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
/// Fairing red.
const BODY: [u8; 4] = [196, 38, 46, 255];
/// Dark red shadow of the fairing.
const BODY_DARK: [u8; 4] = [138, 20, 26, 255];
/// Chrome/metal grey.
const METAL: [u8; 4] = [168, 172, 180, 255];
/// Rider leathers, blue.
const SUIT: [u8; 4] = [46, 78, 178, 255];
/// Rider leathers in shadow.
const SUIT_DARK: [u8; 4] = [30, 52, 126, 255];
/// Helmet white.
const HELMET: [u8; 4] = [238, 238, 238, 255];
/// Brake-light red, lit.
const TAIL_LIT: [u8; 4] = [255, 42, 30, 255];
/// Scuff grey for the crash debris.
const SCUFF: [u8; 4] = [120, 116, 108, 255];
/// Foliage green, lit side.
const LEAF: [u8; 4] = [34, 112, 48, 255];
/// Foliage green, shadow side.
const LEAF_DARK: [u8; 4] = [22, 78, 34, 255];
/// Trunk brown.
const TRUNK: [u8; 4] = [96, 64, 38, 255];
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

/// The riding frame: upright bike, seated rider, chrome exhaust.
fn riding_frame() -> RgbaImage {
    let mut img = blank_frame();
    // Wheels: tyre disc with a lighter hub.
    fill_disc(&mut img, 7, 25, 4, TIRE);
    fill_disc(&mut img, 7, 25, 2, HUB);
    fill_disc(&mut img, 17, 25, 4, TIRE);
    fill_disc(&mut img, 17, 25, 2, HUB);
    // Engine block and tank.
    fill_rect(&mut img, 8, 20, 16, 24, BODY);
    fill_rect(&mut img, 9, 17, 15, 20, BODY);
    fill_rect(&mut img, 9, 19, 15, 20, BODY_DARK);
    // Seat behind the rider, exhaust ahead of the rear wheel.
    fill_rect(&mut img, 5, 16, 9, 19, BODY_DARK);
    fill_rect(&mut img, 14, 25, 21, 27, METAL);
    // Rider: torso leaned forward, helmet, arm to the bars.
    fill_rect(&mut img, 10, 10, 14, 18, SUIT);
    fill_rect(&mut img, 13, 13, 14, 18, SUIT_DARK);
    fill_disc(&mut img, 12, 7, 3, HELMET);
    fill_rect(&mut img, 13, 12, 18, 14, SUIT);
    img
}

/// The braking frame: rider crouched over the tank, brake light lit.
fn braking_frame() -> RgbaImage {
    let mut img = riding_frame();
    // Crouch: torso lower and further forward, helmet down.
    fill_rect(&mut img, 10, 10, 14, 18, CLEAR);
    fill_disc(&mut img, 12, 7, 3, CLEAR);
    fill_rect(&mut img, 12, 12, 17, 19, SUIT);
    fill_rect(&mut img, 15, 14, 16, 19, SUIT_DARK);
    fill_disc(&mut img, 16, 9, 3, HELMET);
    // Lit tail light at the seat tail.
    fill_rect(&mut img, 4, 17, 6, 19, TAIL_LIT);
    img
}

/// The crash frame: bike on its side, rider sprawled behind it, scuff dust.
fn crash_frame() -> RgbaImage {
    let mut img = blank_frame();
    // Bike lying on its right side: wheels side by side, body flat between.
    fill_disc(&mut img, 5, 26, 4, TIRE);
    fill_disc(&mut img, 5, 26, 2, HUB);
    fill_disc(&mut img, 19, 26, 4, TIRE);
    fill_disc(&mut img, 19, 26, 2, HUB);
    fill_rect(&mut img, 8, 23, 17, 27, BODY);
    fill_rect(&mut img, 8, 25, 17, 27, BODY_DARK);
    // Handlebar sticking up.
    fill_rect(&mut img, 15, 19, 20, 21, METAL);
    // Rider sprawled behind the bike, above it on the ground.
    fill_rect(&mut img, 8, 15, 16, 22, SUIT);
    fill_rect(&mut img, 8, 20, 11, 22, SUIT_DARK);
    fill_disc(&mut img, 12, 12, 3, HELMET);
    // Scuff dust kicked up along the slide.
    fill_rect(&mut img, 2, 24, 4, 26, SCUFF);
    fill_rect(&mut img, 21, 22, 22, 23, SCUFF);
    img
}

/// All placeholder frames as `(name, image)` pairs, ready for the packer.
pub fn placeholder_frames() -> Vec<(String, RgbaImage)> {
    vec![
        ("bike_ride".to_owned(), riding_frame()),
        ("bike_brake".to_owned(), braking_frame()),
        ("bike_crash".to_owned(), crash_frame()),
        ("tree".to_owned(), tree_frame()),
        ("bush".to_owned(), bush_frame()),
        ("sign".to_owned(), sign_frame()),
        ("car".to_owned(), car_frame()),
        ("cone".to_owned(), cone_frame()),
    ]
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
        fill_rect(&mut img, 4 + step, 4 + step, 5 + step, 10 - step, BODY);
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
