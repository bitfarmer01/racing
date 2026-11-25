//! Procedural HD sprite frames, packed through the `assets` crate's atlas
//! packer.
//!
//! Everything here is drawn in code into RGBA buffers — no binary art files
//! exist in the repo. The visual direction is modern HD 2.5D (wave 7
//! decision, 2026-11-24): frames render at 4x the original pixel-art scale,
//! every silhouette is anti-aliased via supersampled coverage, and every
//! surface shades with a smooth gradient. The frames are placeholders in the
//! Road Rash silhouette sense — correct scale and pose — to be replaced by
//! authored art through the same atlas pipeline.
//!
//! Determinism: every frame is a pure function of its palette — same frame
//! name, same pixels, always.

use assets::PackOptions;
use image::RgbaImage;

/// Side-view frame size in pixels, shared by all bike frames. 4x the
/// original 24x32 pixel-art scale.
pub const FRAME_WIDTH: u32 = 96;
/// Side-view frame size in pixels, shared by all bike frames.
pub const FRAME_HEIGHT: u32 = 128;

/// Subsamples per pixel axis for silhouette coverage. 3x3 supersampling
/// gives 9-level alpha steps — smooth enough that linear filtering on top
/// reads as a clean soft edge.
const SS: i32 = 3;

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
/// Palm trunk tan.
const PALM_TRUNK: [u8; 4] = [122, 88, 52, 255];
/// Building facade stone, lit and shaded.
const STONE_LIT: [u8; 4] = [206, 198, 182, 255];
const STONE_DARK: [u8; 4] = [148, 138, 124, 255];
/// Fence/pole timber.
const TIMBER_LIT: [u8; 4] = [166, 128, 76, 255];
const TIMBER_DARK: [u8; 4] = [104, 74, 42, 255];

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

// ---------------------------------------------------------------------------
// Soft drawing primitives — supersampled coverage, straight-alpha
// compositing, gradient shading. These are the whole "HD" mechanism: hard
// pixel fills are retired.
// ---------------------------------------------------------------------------

/// Linear interpolation between two colors, `t` in [0, 1]. Pure.
fn lerp(c0: [u8; 4], c1: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0u8; 4];
    for c in 0..4 {
        let v = c0[c] as f32 + (c1[c] as f32 - c0[c] as f32) * t;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Lightens a color toward white by `amt` (0..1). Pure.
fn lighten(c: [u8; 4], amt: f32) -> [u8; 4] {
    lerp(c, [255, 255, 255, c[3]], amt)
}

/// Composites `color` over the pixel at straight alpha `a` (0..1): the
/// atlas stores non-premultiplied RGBA8, which is what the renderer's
/// alpha-blending pipeline expects.
fn blend_px(img: &mut RgbaImage, x: u32, y: u32, a: f32, color: [u8; 4]) {
    if a <= 0.0 {
        return;
    }
    let dst = img.get_pixel(x, y).0;
    let da = dst[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    let mut out = [0u8; 4];
    for c in 0..3 {
        let v = (color[c] as f32 * a + dst[c] as f32 * da * (1.0 - a)) / oa;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
    img.put_pixel(x, y, out.into());
}

/// Fills a pixel by supersampled coverage `hits` in [0, 1] with `color`,
/// compositing over what is already there.
fn paint(img: &mut RgbaImage, x: u32, y: u32, hits: f32, color: [u8; 4]) {
    blend_px(img, x, y, hits, color);
}

/// Anti-aliased rectangle from (x0, y0) inclusive to (x1, y1) exclusive,
/// f32 edges so shapes land on sub-pixel boundaries cleanly.
fn fill_rect_soft(img: &mut RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 4]) {
    let (ix0, ix1) = (
        x0.floor().max(0.0) as u32,
        x1.ceil().min(img.width() as f32) as u32,
    );
    let (iy0, iy1) = (
        y0.floor().max(0.0) as u32,
        y1.ceil().min(img.height() as f32) as u32,
    );
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if px >= x0 && px < x1 && py >= y0 && py < y1 {
                        hits += 1;
                    }
                }
            }
            paint(img, x, y, hits as f32 / (SS * SS) as f32, color);
        }
    }
}

/// Anti-aliased vertical-gradient rectangle: `top` at y0, `bottom` at y1.
fn fill_rect_gradient(
    img: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    top: [u8; 4],
    bottom: [u8; 4],
) {
    let (ix0, ix1) = (
        x0.floor().max(0.0) as u32,
        x1.ceil().min(img.width() as f32) as u32,
    );
    let (iy0, iy1) = (
        y0.floor().max(0.0) as u32,
        y1.ceil().min(img.height() as f32) as u32,
    );
    for y in iy0..iy1 {
        let t = ((y as f32 + 0.5 - y0) / (y1 - y0)).clamp(0.0, 1.0);
        let shade = lerp(top, bottom, t);
        for x in ix0..ix1 {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if px >= x0 && px < x1 && py >= y0 && py < y1 {
                        hits += 1;
                    }
                }
            }
            paint(img, x, y, hits as f32 / (SS * SS) as f32, shade);
        }
    }
}

/// Anti-aliased rounded rectangle (circular corners of radius `r`).
fn fill_round_rect_soft(
    img: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    r: f32,
    color: [u8; 4],
) {
    let (ix0, ix1) = (
        x0.floor().max(0.0) as u32,
        x1.ceil().min(img.width() as f32) as u32,
    );
    let (iy0, iy1) = (
        y0.floor().max(0.0) as u32,
        y1.ceil().min(img.height() as f32) as u32,
    );
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if round_rect_contains(px, py, x0, y0, x1, y1, r) {
                        hits += 1;
                    }
                }
            }
            paint(img, x, y, hits as f32 / (SS * SS) as f32, color);
        }
    }
}

/// Whether the point lies inside the rounded rectangle.
fn round_rect_contains(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if px < x0 || px >= x1 || py < y0 || py >= y1 {
        return false;
    }
    let (cx, cy) = (
        (px - (x0 + r)).max(-(px - (x1 - r))),
        (py - (y0 + r)).max(-(py - (y1 - r))),
    );
    cx * cx + cy * cy <= r * r || (px >= x0 + r && px < x1 - r) || (py >= y0 + r && py < y1 - r)
}

/// Anti-aliased disc with a top-left radial highlight: `base` in the
/// lower-right shade, lightened toward the upper rim. The workhorse for
/// canopies, tyres, and helmets.
fn fill_disc_shaded(
    img: &mut RgbaImage,
    cx: f32,
    cy: f32,
    r: f32,
    base: [u8; 4],
    highlight: [u8; 4],
) {
    let (ix0, ix1) = (
        (cx - r).floor().max(0.0) as u32,
        (cx + r).ceil().min(img.width() as f32) as u32,
    );
    let (iy0, iy1) = (
        (cy - r).floor().max(0.0) as u32,
        (cy + r).ceil().min(img.height() as f32) as u32,
    );
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            let mut hits = 0;
            let mut shade_acc = 0.0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    let (dx, dy) = (px - cx, py - cy);
                    if dx * dx + dy * dy <= r * r {
                        hits += 1;
                        // Light from the top-left: u/v in [-1, 1].
                        let (u, v) = (dx / r.max(1.0), dy / r.max(1.0));
                        shade_acc += ((-u) * 0.35 + (-v) * 0.65 + 1.0) * 0.5;
                    }
                }
            }
            if hits == 0 {
                continue;
            }
            let shade = lerp(base, highlight, (shade_acc / hits as f32).clamp(0.0, 1.0));
            paint(img, x, y, hits as f32 / (SS * SS) as f32, shade);
        }
    }
}

/// Hard clear to alpha-zero — for redrawing a region of a base frame.
fn clear_rect(img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) {
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            img.put_pixel(x, y, CLEAR.into());
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
    // Rear tyre: a shaded disc with a small hub, flanked by exhaust hints.
    fill_disc_shaded(&mut img, 48.0, 106.0, 17.0, TIRE, lighten(TIRE, 0.35));
    fill_round_rect_soft(&mut img, 42.0, 108.0, 54.0, 120.0, 3.0, HUB);
    // Exhaust hints: metal gradient falling into shadow at the bottom.
    fill_rect_gradient(&mut img, 22.0, 94.0, 34.0, 118.0, METAL, [90, 92, 98, 255]);
    fill_rect_gradient(&mut img, 62.0, 94.0, 74.0, 118.0, METAL, [90, 92, 98, 255]);
    // Tail fairing over the wheel; body shade falling to the dark edge.
    fill_round_rect_soft(&mut img, 32.0, 70.0, 64.0, 96.0, 6.0, palette.body);
    fill_rect_gradient(
        &mut img,
        32.0,
        84.0,
        64.0,
        96.0,
        palette.body,
        palette.body_dark,
    );
    // Unlit tail light slot.
    fill_round_rect_soft(&mut img, 38.0, 74.0, 58.0, 84.0, 3.0, palette.body_dark);
    // Rider's back, shading down the right side.
    fill_rect_gradient(
        &mut img,
        34.0,
        34.0,
        62.0,
        74.0,
        palette.suit,
        palette.suit_dark,
    );
    // Shoulders, shaded underneath.
    fill_round_rect_soft(&mut img, 26.0, 28.0, 70.0, 48.0, 8.0, palette.suit);
    fill_rect_gradient(
        &mut img,
        26.0,
        40.0,
        70.0,
        48.0,
        palette.suit,
        palette.suit_dark,
    );
    // Arms reaching out to the bars.
    fill_rect_gradient(
        &mut img,
        16.0,
        40.0,
        34.0,
        56.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_rect_gradient(
        &mut img,
        62.0,
        40.0,
        80.0,
        56.0,
        palette.suit,
        palette.suit_dark,
    );
    // Handlebar grips visible at the frame edges, shading with the metal.
    fill_rect_gradient(&mut img, 8.0, 36.0, 22.0, 54.0, METAL, [120, 122, 130, 255]);
    fill_rect_gradient(
        &mut img,
        74.0,
        36.0,
        88.0,
        54.0,
        METAL,
        [120, 122, 130, 255],
    );
    // Helmet above the shoulders, lit from the top-left.
    fill_disc_shaded(
        &mut img,
        48.0,
        18.0,
        14.0,
        palette.helmet,
        lighten(palette.helmet, 0.4),
    );
    img
}

/// The braking frame: rider crouched low over the tank, tail light lit.
fn braking_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = riding_frame(palette);
    // Crouch: clear the upright torso, shoulders, and helmet, then redraw
    // them lower and wider.
    clear_rect(&mut img, 4, 0, 92, 66);
    // Arms bracing low to the grips.
    fill_rect_gradient(
        &mut img,
        18.0,
        50.0,
        34.0,
        66.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_rect_gradient(
        &mut img,
        62.0,
        50.0,
        78.0,
        66.0,
        palette.suit,
        palette.suit_dark,
    );
    // Grips stay at the edges, shading with the metal.
    fill_rect_gradient(&mut img, 8.0, 46.0, 22.0, 62.0, METAL, [120, 122, 130, 255]);
    fill_rect_gradient(
        &mut img,
        74.0,
        46.0,
        88.0,
        62.0,
        METAL,
        [120, 122, 130, 255],
    );
    // Hunched torso and lowered shoulders.
    fill_rect_gradient(
        &mut img,
        30.0,
        50.0,
        66.0,
        74.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_round_rect_soft(&mut img, 24.0, 46.0, 72.0, 60.0, 8.0, palette.suit);
    // Helmet tucked down between the shoulders.
    fill_disc_shaded(
        &mut img,
        48.0,
        38.0,
        14.0,
        palette.helmet,
        lighten(palette.helmet, 0.4),
    );
    // Lit tail light: braking — a red gradient falling to a deep ember edge.
    fill_rect_gradient(
        &mut img,
        38.0,
        74.0,
        58.0,
        84.0,
        TAIL_LIT,
        [120, 12, 8, 255],
    );
    img
}

/// The crash frame: bike on its side, rider sprawled behind it, scuff dust.
/// A downed rider is seen from behind as a sprawl next to the fallen bike.
fn crash_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = blank_frame();
    // Bike lying on its right side: wheels side by side, body flat between.
    fill_disc_shaded(&mut img, 20.0, 104.0, 17.0, TIRE, lighten(TIRE, 0.35));
    fill_disc_shaded(&mut img, 76.0, 104.0, 17.0, TIRE, lighten(TIRE, 0.35));
    fill_round_rect_soft(&mut img, 32.0, 92.0, 64.0, 110.0, 5.0, palette.body);
    fill_rect_gradient(
        &mut img,
        32.0,
        100.0,
        64.0,
        110.0,
        palette.body,
        palette.body_dark,
    );
    // Handlebar sticking up.
    fill_round_rect_soft(&mut img, 58.0, 74.0, 80.0, 86.0, 4.0, METAL);
    // Rider sprawled behind the bike, above it on the ground.
    fill_rect_gradient(
        &mut img,
        30.0,
        56.0,
        64.0,
        88.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_disc_shaded(
        &mut img,
        48.0,
        44.0,
        14.0,
        palette.helmet,
        lighten(palette.helmet, 0.4),
    );
    // Scuff dust kicked up along the slide — soft translucent patches.
    fill_rect_soft(
        &mut img,
        6.0,
        96.0,
        16.0,
        106.0,
        [SCUFF[0], SCUFF[1], SCUFF[2], 180],
    );
    fill_rect_soft(
        &mut img,
        84.0,
        90.0,
        92.0,
        96.0,
        [SCUFF[0], SCUFF[1], SCUFF[2], 160],
    );
    img
}

/// The punch swing: rider twisted off-line, one fist thrown past the
/// shoulder at whoever is alongside. Same rear-view grammar as the ride
/// frame, with the throwing arm replacing the left bar grip.
fn punch_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = riding_frame(palette);
    // Clear the upper body to redraw the throw.
    clear_rect(&mut img, 4, 0, 92, 62);
    // Right hand still braced low on the bars.
    fill_rect_gradient(
        &mut img,
        62.0,
        44.0,
        80.0,
        58.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_rect_gradient(
        &mut img,
        74.0,
        38.0,
        88.0,
        56.0,
        METAL,
        [120, 122, 130, 255],
    );
    // Torso twisted into the swing, shading across the back.
    fill_rect_gradient(
        &mut img,
        30.0,
        38.0,
        66.0,
        74.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_round_rect_soft(&mut img, 26.0, 32.0, 72.0, 52.0, 8.0, palette.suit);
    // The throw: a gradient arm up past the shoulder, ending in a big
    // shaded fist — the point of contact, lit from above.
    fill_rect_gradient(
        &mut img,
        12.0,
        14.0,
        34.0,
        44.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_disc_shaded(
        &mut img,
        16.0,
        14.0,
        11.0,
        palette.suit,
        lighten(palette.suit, 0.45),
    );
    // Helmet, tucked behind the swing.
    fill_disc_shaded(
        &mut img,
        54.0,
        20.0,
        14.0,
        palette.helmet,
        lighten(palette.helmet, 0.4),
    );
    img
}

/// The kick swing: one boot swung out wide of the bike at rider height —
/// a shaded leg off the peg, boot disc at the end of the arc.
fn kick_frame(palette: &BikePalette) -> RgbaImage {
    let mut img = riding_frame(palette);
    // Clear the upper body and the right flank for the swung leg.
    clear_rect(&mut img, 4, 0, 92, 62);
    clear_rect(&mut img, 58, 56, 96, 78);
    // Left hand stays on the bars.
    fill_rect_gradient(
        &mut img,
        16.0,
        44.0,
        34.0,
        58.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_rect_gradient(&mut img, 8.0, 38.0, 24.0, 56.0, METAL, [120, 122, 130, 255]);
    // Torso leaned away from the kick.
    fill_rect_gradient(
        &mut img,
        28.0,
        36.0,
        62.0,
        74.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_round_rect_soft(&mut img, 24.0, 30.0, 66.0, 50.0, 8.0, palette.suit);
    // The swung leg: gradient shin out of the frame right, boot disc at
    // the end of the arc.
    fill_rect_gradient(
        &mut img,
        56.0,
        54.0,
        86.0,
        72.0,
        palette.suit,
        palette.suit_dark,
    );
    fill_disc_shaded(&mut img, 86.0, 60.0, 10.0, TIRE, lighten(TIRE, 0.5));
    // Helmet, upright over the lean.
    fill_disc_shaded(
        &mut img,
        44.0,
        20.0,
        14.0,
        palette.helmet,
        lighten(palette.helmet, 0.4),
    );
    img
}

/// All placeholder frames as `(name, image)` pairs, ready for the packer.
pub fn placeholder_frames() -> Vec<(String, RgbaImage)> {
    vec![
        ("bike_ride".to_owned(), riding_frame(&PLAYER_PALETTE)),
        ("bike_brake".to_owned(), braking_frame(&PLAYER_PALETTE)),
        ("bike_crash".to_owned(), crash_frame(&PLAYER_PALETTE)),
        ("bike_punch".to_owned(), punch_frame(&PLAYER_PALETTE)),
        ("bike_kick".to_owned(), kick_frame(&PLAYER_PALETTE)),
        ("bike_rival".to_owned(), riding_frame(RIVAL_PALETTES[0])),
        (
            "bike_punch_rival".to_owned(),
            punch_frame(RIVAL_PALETTES[0]),
        ),
        ("bike_kick_rival".to_owned(), kick_frame(RIVAL_PALETTES[0])),
        ("bike_rival2".to_owned(), riding_frame(RIVAL_PALETTES[1])),
        (
            "bike_punch_rival2".to_owned(),
            punch_frame(RIVAL_PALETTES[1]),
        ),
        ("bike_kick_rival2".to_owned(), kick_frame(RIVAL_PALETTES[1])),
        ("bike_rival3".to_owned(), riding_frame(RIVAL_PALETTES[2])),
        (
            "bike_punch_rival3".to_owned(),
            punch_frame(RIVAL_PALETTES[2]),
        ),
        ("bike_kick_rival3".to_owned(), kick_frame(RIVAL_PALETTES[2])),
        ("tree".to_owned(), tree_frame()),
        ("tree2".to_owned(), broadleaf_frame()),
        ("tree3".to_owned(), tall_pine_frame()),
        ("bush".to_owned(), bush_frame()),
        ("sign".to_owned(), sign_frame()),
        ("car".to_owned(), car_frame()),
        ("cone".to_owned(), cone_frame()),
        ("shadow".to_owned(), shadow_frame()),
        // Wave 7c world construction: zone dressing...
        ("palm".to_owned(), palm_frame()),
        ("building".to_owned(), building_frame()),
        ("fence".to_owned(), fence_frame()),
        ("pole".to_owned(), pole_frame()),
        ("marker".to_owned(), marker_frame()),
        ("rock".to_owned(), rock_frame()),
        // ...and the large per-lap landmarks.
        ("water_tower".to_owned(), water_tower_frame()),
        ("windmill".to_owned(), windmill_frame()),
        ("billboard".to_owned(), billboard_frame()),
    ]
}

/// A soft elliptical ground shadow: dark ink fading to transparent at the
/// rim, alpha-blended under trees, bikes, and cars (D3).
fn shadow_frame() -> RgbaImage {
    let (w, h) = (128_u32, 40_u32);
    let (cx, cy) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    let (rx, ry) = ((w - 1) as f32 / 2.0, (h - 1) as f32 / 2.0);
    RgbaImage::from_fn(w, h, |x, y| {
        let dx = (x as f32 - cx) / rx;
        let dy = (y as f32 - cy) / ry;
        let r2 = dx * dx + dy * dy;
        if r2 <= 1.0 {
            // Dark ink at the core, lightening and fading toward the rim —
            // both color and alpha shade continuously.
            let ink = lerp([12, 18, 10, 255], [46, 56, 42, 255], r2);
            let alpha = (255.0 * (1.0 - r2) * 0.55) as u8;
            [ink[0], ink[1], ink[2], alpha].into()
        } else {
            CLEAR.into()
        }
    })
}

/// A roadside pine: trunk and a three-tier canopy, each tier lit from the
/// top-left. 96x160, anchored on the road.
fn tree_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(96, 160, |_, _| CLEAR.into());
    fill_rect_gradient(&mut img, 40.0, 118.0, 56.0, 158.0, TRUNK, [64, 40, 22, 255]);
    // Canopy tiers, wider at the bottom, each with its own top-left light.
    fill_disc_shaded(&mut img, 48.0, 86.0, 36.0, LEAF, lighten(LEAF, 0.3));
    fill_disc_shaded(&mut img, 30.0, 100.0, 20.0, LEAF_DARK, LEAF);
    fill_disc_shaded(&mut img, 48.0, 46.0, 26.0, LEAF, lighten(LEAF, 0.3));
    fill_disc_shaded(&mut img, 48.0, 16.0, 17.0, LEAF, lighten(LEAF, 0.35));
    img
}

/// A broadleaf: short trunk under a wide round crown. 112x136.
fn broadleaf_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(112, 136, |_, _| CLEAR.into());
    fill_rect_gradient(&mut img, 48.0, 96.0, 64.0, 134.0, TRUNK, [64, 40, 22, 255]);
    // A big round crown: layered shaded discs, light from the top-right.
    fill_disc_shaded(&mut img, 56.0, 52.0, 48.0, LEAF, lighten(LEAF, 0.3));
    fill_disc_shaded(&mut img, 36.0, 66.0, 24.0, LEAF_DARK, LEAF);
    fill_disc_shaded(&mut img, 78.0, 40.0, 24.0, LEAF, lighten(LEAF, 0.25));
    fill_disc_shaded(&mut img, 46.0, 34.0, 20.0, LEAF, lighten(LEAF, 0.35));
    fill_disc_shaded(&mut img, 66.0, 58.0, 20.0, LEAF_DARK, LEAF);
    img
}

/// A tall slim pine: narrow spire, four stacked skirts. 64x192.
fn tall_pine_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(64, 192, |_, _| CLEAR.into());
    fill_rect_gradient(&mut img, 28.0, 158.0, 36.0, 190.0, TRUNK, [64, 40, 22, 255]);
    for (cy, r) in [
        (150.0, 26.0),
        (118.0, 22.0),
        (86.0, 18.0),
        (54.0, 14.0),
        (26.0, 10.0),
    ] {
        fill_disc_shaded(&mut img, 32.0, cy, r, LEAF, lighten(LEAF, 0.3));
    }
    img
}

/// A low roadside bush. 64x40.
fn bush_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(64, 40, |_, _| CLEAR.into());
    fill_rect_gradient(&mut img, 6.0, 22.0, 58.0, 38.0, LEAF, LEAF_DARK);
    fill_disc_shaded(&mut img, 20.0, 20.0, 13.0, LEAF, lighten(LEAF, 0.25));
    fill_disc_shaded(&mut img, 44.0, 20.0, 13.0, LEAF, lighten(LEAF, 0.25));
    fill_disc_shaded(&mut img, 20.0, 20.0, 5.0, LEAF_DARK, LEAF);
    img
}

/// A roadside warning sign: pole and a chevron board with a soft sheen
/// falling down the board. 72x96.
fn sign_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(72, 96, |_, _| CLEAR.into());
    fill_rect_gradient(
        &mut img,
        32.0,
        46.0,
        40.0,
        94.0,
        METAL,
        [120, 122, 130, 255],
    );
    fill_round_rect_soft(&mut img, 8.0, 8.0, 64.0, 48.0, 5.0, SIGN_YELLOW);
    // Sheen: one continuous light-to-deep gradient down the whole board.
    fill_rect_gradient(
        &mut img,
        8.0,
        12.0,
        64.0,
        48.0,
        SIGN_YELLOW,
        [196, 150, 24, 255],
    );
    // Chevron pointing right (corners bend right around here).
    for step in 0..5 {
        fill_rect_soft(
            &mut img,
            (16 + step * 8) as f32,
            (14 + step * 5) as f32,
            (22 + step * 8) as f32,
            (38 - step * 5) as f32,
            SIGN_CHEVRON,
        );
    }
    img
}

/// A traffic car from behind: the view the player actually sees. 104x72.
fn car_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(104, 72, |_, _| CLEAR.into());
    // Cabin and body, each with a soft top-to-bottom falloff.
    fill_round_rect_soft(&mut img, 20.0, 4.0, 84.0, 28.0, 8.0, CAR_BODY);
    fill_rect_gradient(
        &mut img,
        20.0,
        8.0,
        84.0,
        28.0,
        [226, 228, 234, 255],
        [176, 178, 186, 255],
    );
    fill_round_rect_soft(&mut img, 8.0, 24.0, 96.0, 64.0, 10.0, CAR_BODY);
    fill_rect_gradient(
        &mut img,
        8.0,
        40.0,
        96.0,
        64.0,
        CAR_BODY,
        [172, 174, 184, 255],
    );
    // Rear window: sky reflection at the top falling into dark glass.
    fill_round_rect_soft(&mut img, 28.0, 8.0, 76.0, 24.0, 5.0, CAR_GLASS);
    fill_rect_gradient(
        &mut img,
        28.0,
        12.0,
        76.0,
        24.0,
        [110, 120, 140, 255],
        CAR_GLASS,
    );
    // Bumper shade.
    fill_rect_gradient(
        &mut img,
        8.0,
        52.0,
        96.0,
        64.0,
        [70, 78, 94, 255],
        CAR_GLASS,
    );
    // Taillights.
    fill_rect_gradient(
        &mut img,
        12.0,
        32.0,
        24.0,
        40.0,
        [235, 70, 50, 255],
        CAR_TAIL,
    );
    fill_rect_gradient(
        &mut img,
        80.0,
        32.0,
        92.0,
        40.0,
        [235, 70, 50, 255],
        CAR_TAIL,
    );
    // Tyres.
    fill_rect_soft(&mut img, 12.0, 64.0, 28.0, 70.0, TIRE);
    fill_rect_soft(&mut img, 76.0, 64.0, 92.0, 70.0, TIRE);
    img
}

/// A traffic cone: the on-road hazard, drawn as a smooth gradient taper
/// with a white band. 40x48. One color pass per pixel row — the taper, the
/// band, and the base all shade continuously, so no boundary is left hard.
fn cone_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(40, 48, |_, _| CLEAR.into());
    for y in 0..48 {
        // Tip starts at y=6; above that the frame stays empty.
        if y < 6 {
            continue;
        }
        let t = (y as f32 - 6.0) / 34.0;
        // Half-width: taper from 3 px at the tip to 12 px at the base,
        // then the flat base plate widens from the taper's foot.
        let (half, base_t) = if y < 40 {
            (3.0 + t * 9.0, 0.0_f32)
        } else {
            (12.0 + (y - 40) as f32 / 7.0 * 3.0, (y - 40) as f32 / 6.0)
        };
        // Color: orange falling into shadow with height, continuing into
        // the base plate so the foot does not step.
        let mut shade = lerp([248, 132, 60, 255], [172, 58, 12, 255], t.min(1.0));
        if base_t > 0.0 {
            shade = lerp([172, 58, 12, 255], [110, 38, 10, 255], base_t);
        }
        // Reflective band, blended in and out over three rows so the band
        // edge shades instead of stepping.
        let band = match y {
            17..=19 => (y - 17) as f32 / 3.0,
            20..=24 => 1.0,
            25..=27 => (27 - y) as f32 / 3.0,
            _ => 0.0,
        };
        if band > 0.0 {
            shade = lerp(
                shade,
                lerp(
                    [250, 250, 248, 255],
                    [210, 210, 206, 255],
                    t.clamp(0.0, 1.0),
                ),
                band,
            );
        }
        fill_rect_soft(
            &mut img,
            20.0 - half,
            y as f32,
            20.0 + half,
            (y + 1) as f32,
            shade,
        );
    }
    img
}

// ---------------------------------------------------------------------------
// Wave 7c — world construction frames: zone dressing and landmarks. Same
// grammar as the rest of the HD set — every surface shades smoothly.
// ---------------------------------------------------------------------------

/// A coastal palm: curved trunk, radiating frond clusters, coconuts. 96x160.
fn palm_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(96, 160, |_, _| CLEAR.into());
    // Curved trunk: gradient segments leaning as they climb.
    for i in 0..8 {
        let y = 150.0 - i as f32 * 14.0;
        let x = 40.0 + i as f32 * 1.4;
        fill_rect_gradient(
            &mut img,
            x,
            y,
            x + 9.0,
            y + 15.0,
            PALM_TRUNK,
            [70, 48, 26, 255],
        );
    }
    // Fronds: shaded leaf discs fanning from the crown.
    let (cx, cy) = (58.0_f32, 36.0_f32);
    for (dx, dy, r) in [
        (-26.0, 8.0, 17.0),
        (26.0, 8.0, 17.0),
        (-18.0, -8.0, 15.0),
        (18.0, -8.0, 15.0),
        (0.0, -14.0, 16.0),
        (0.0, 12.0, 14.0),
    ] {
        fill_disc_shaded(&mut img, cx + dx, cy + dy, r, LEAF, lighten(LEAF, 0.35));
    }
    // Coconuts at the crown.
    fill_disc_shaded(
        &mut img,
        cx - 6.0,
        cy + 6.0,
        5.0,
        PALM_TRUNK,
        lighten(PALM_TRUNK, 0.3),
    );
    fill_disc_shaded(
        &mut img,
        cx + 6.0,
        cy + 6.0,
        5.0,
        PALM_TRUNK,
        lighten(PALM_TRUNK, 0.3),
    );
    img
}

/// A town block: gradient facade, shaded window grid, roof cap, door. 128x160.
fn building_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(128, 160, |_, _| CLEAR.into());
    // Facade: stacked gradient courses — one 138-row ramp quantizes into
    // flat rows; courses keep the stone shading moving.
    for i in 0..7 {
        let y0 = 20.0 + i as f32 * 20.0;
        fill_rect_gradient(&mut img, 8.0, y0, 120.0, y0 + 21.0, STONE_LIT, STONE_DARK);
    }
    // Roof cap.
    fill_rect_gradient(
        &mut img,
        4.0,
        12.0,
        124.0,
        26.0,
        [120, 62, 48, 255],
        [88, 42, 32, 255],
    );
    // Window grid: glass shading from a sky reflection into the room.
    for row in 0..3 {
        for col in 0..4 {
            let x = 18.0 + col as f32 * 26.0;
            let y = 34.0 + row as f32 * 30.0;
            fill_round_rect_soft(&mut img, x, y, x + 16.0, y + 20.0, 2.0, [52, 58, 70, 255]);
            fill_rect_gradient(
                &mut img,
                x,
                y,
                x + 16.0,
                y + 20.0,
                [120, 132, 150, 255],
                [52, 58, 70, 255],
            );
        }
    }
    // Door.
    fill_rect_gradient(
        &mut img,
        54.0,
        124.0,
        74.0,
        158.0,
        [96, 64, 40, 255],
        [64, 40, 24, 255],
    );
    img
}

/// A fence run: two shaded rails over posts. 128x48.
fn fence_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(128, 48, |_, _| CLEAR.into());
    for i in 0..5 {
        let x = 8.0 + i as f32 * 28.0;
        fill_rect_gradient(&mut img, x, 8.0, x + 7.0, 46.0, TIMBER_LIT, TIMBER_DARK);
    }
    fill_rect_gradient(
        &mut img,
        0.0,
        14.0,
        128.0,
        22.0,
        lighten(TIMBER_LIT, 0.1),
        TIMBER_DARK,
    );
    fill_rect_gradient(
        &mut img,
        0.0,
        30.0,
        128.0,
        38.0,
        lighten(TIMBER_LIT, 0.1),
        TIMBER_DARK,
    );
    img
}

/// A power pole: shaded mast, crossarm, insulators. 48x192.
fn pole_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(48, 192, |_, _| CLEAR.into());
    // Mast: stacked gradient segments — a single 184-row ramp quantizes
    // into flat rows; segments keep the shade moving.
    for i in 0..8 {
        let y0 = 6.0 + i as f32 * 23.0;
        fill_rect_gradient(&mut img, 20.0, y0, 28.0, y0 + 24.0, TIMBER_LIT, TIMBER_DARK);
    }
    fill_rect_gradient(
        &mut img,
        6.0,
        22.0,
        42.0,
        30.0,
        lighten(TIMBER_LIT, 0.1),
        [88, 64, 36, 255],
    );
    fill_disc_shaded(
        &mut img,
        12.0,
        20.0,
        4.0,
        [40, 44, 52, 255],
        [90, 96, 108, 255],
    );
    fill_disc_shaded(
        &mut img,
        36.0,
        20.0,
        4.0,
        [40, 44, 52, 255],
        [90, 96, 108, 255],
    );
    img
}

/// A 500 m distance-marker post: white post with a shaded marker cap. 40x72.
fn marker_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(40, 72, |_, _| CLEAR.into());
    // Cap band.
    fill_rect_gradient(
        &mut img,
        14.0,
        10.0,
        26.0,
        26.0,
        [40, 84, 168, 255],
        [24, 52, 110, 255],
    );
    // Post shaft: two ramps — one 44-row ramp quantizes into flat rows.
    fill_rect_gradient(
        &mut img,
        14.0,
        26.0,
        26.0,
        48.0,
        [238, 238, 234, 255],
        [212, 212, 206, 255],
    );
    fill_rect_gradient(
        &mut img,
        14.0,
        48.0,
        26.0,
        70.0,
        [212, 212, 206, 255],
        [188, 188, 182, 255],
    );
    img
}

/// A ridge boulder: two shaded grey masses. 80x56.
fn rock_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(80, 56, |_, _| CLEAR.into());
    fill_disc_shaded(
        &mut img,
        30.0,
        34.0,
        22.0,
        [138, 132, 124, 255],
        [188, 184, 176, 255],
    );
    fill_disc_shaded(
        &mut img,
        56.0,
        38.0,
        18.0,
        [120, 114, 106, 255],
        [170, 164, 156, 255],
    );
    img
}

/// A water tower: splayed legs, shaded tank, cone roof. 128x192.
fn water_tower_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(128, 192, |_, _| CLEAR.into());
    // Legs: an outer pair and an inner pair.
    fill_rect_gradient(
        &mut img,
        34.0,
        60.0,
        42.0,
        190.0,
        METAL,
        [110, 112, 120, 255],
    );
    fill_rect_gradient(
        &mut img,
        86.0,
        60.0,
        94.0,
        190.0,
        METAL,
        [110, 112, 120, 255],
    );
    fill_rect_gradient(
        &mut img,
        52.0,
        70.0,
        58.0,
        190.0,
        [150, 152, 160, 255],
        [104, 106, 114, 255],
    );
    fill_rect_gradient(
        &mut img,
        70.0,
        70.0,
        76.0,
        190.0,
        [150, 152, 160, 255],
        [104, 106, 114, 255],
    );
    // Tank: a shaded barrel.
    fill_round_rect_soft(&mut img, 36.0, 52.0, 92.0, 96.0, 12.0, [188, 168, 138, 255]);
    fill_rect_gradient(
        &mut img,
        36.0,
        60.0,
        92.0,
        96.0,
        [204, 186, 152, 255],
        [150, 130, 100, 255],
    );
    // Cone roof: stacked shrinking bands, shading as they climb.
    let mut w = 60.0_f32;
    let mut y = 52.0_f32;
    while w > 8.0 {
        fill_rect_gradient(
            &mut img,
            64.0 - w / 2.0,
            y,
            64.0 + w / 2.0,
            y + 5.0,
            [140, 90, 60, 255],
            [104, 62, 40, 255],
        );
        w -= 7.0;
        y -= 5.0;
    }
    img
}

/// A windmill: tapered stone tower and four blades shading to their tips.
/// 128x224.
fn windmill_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(128, 224, |_, _| CLEAR.into());
    // Tapered tower: narrowing gradient bands.
    for i in 0..10 {
        let t = i as f32 / 10.0;
        let half = 30.0 - t * 18.0;
        let y = 214.0 - i as f32 * 16.0;
        fill_rect_gradient(
            &mut img,
            64.0 - half,
            y,
            64.0 + half,
            y + 17.0,
            [176, 168, 156, 255],
            [128, 120, 110, 255],
        );
    }
    // Hub.
    fill_disc_shaded(&mut img, 64.0, 58.0, 9.0, METAL, lighten(METAL, 0.4));
    // Four blades: gradient segments lightening near the hub, darkening to
    // the tip — no flat squares anywhere.
    for (dx, dy) in [(-1.0_f32, -0.5_f32), (1.0, 0.5), (-0.4, 1.0), (0.4, -1.0)] {
        for t in 0..26 {
            let x = 64.0 + dx * t as f32 * 3.4;
            let y = 58.0 + dy * t as f32 * 3.4;
            let shade = lerp([222, 218, 210, 255], [150, 146, 138, 255], t as f32 / 26.0);
            fill_rect_gradient(
                &mut img,
                x - 3.0,
                y - 3.0,
                x + 3.0,
                y + 3.0,
                lighten(shade, 0.2),
                shade,
            );
        }
    }
    img
}

/// A billboard cluster: two shaded boards on shared posts. 128x96.
fn billboard_frame() -> RgbaImage {
    let mut img = RgbaImage::from_fn(128, 96, |_, _| CLEAR.into());
    fill_rect_gradient(
        &mut img,
        30.0,
        40.0,
        38.0,
        94.0,
        METAL,
        [110, 112, 120, 255],
    );
    fill_rect_gradient(
        &mut img,
        90.0,
        40.0,
        98.0,
        94.0,
        METAL,
        [110, 112, 120, 255],
    );
    fill_round_rect_soft(&mut img, 8.0, 8.0, 60.0, 44.0, 3.0, SIGN_YELLOW);
    fill_rect_gradient(
        &mut img,
        8.0,
        12.0,
        60.0,
        44.0,
        [240, 196, 48, 255],
        [196, 150, 24, 255],
    );
    fill_round_rect_soft(&mut img, 68.0, 8.0, 120.0, 44.0, 3.0, [196, 38, 46, 255]);
    fill_rect_gradient(
        &mut img,
        68.0,
        12.0,
        120.0,
        44.0,
        [214, 60, 60, 255],
        [150, 24, 28, 255],
    );
    img
}

/// Packs the placeholder frames into a deterministic atlas through the
/// `assets` crate — the same packer authored art will go through.
pub fn pack_placeholder_atlas() -> Result<assets::PackedAtlas, assets::PackError> {
    let packed = assets::pack(PackOptions::default(), &placeholder_frames())?;
    Ok(align_atlas_width(packed))
}

/// Pads the atlas on the right to a width that is a multiple of 64 px, so
/// `width * 4` bytes satisfies WebGL2's 256-byte row-alignment floor when
/// the renderer uploads the rows (`render::gpu::upload_atlas`). The pad is
/// transparent and outside every sprite rect, so UVs and the look are
/// unchanged; packing stays a pure function of the input set.
fn align_atlas_width(packed: assets::PackedAtlas) -> assets::PackedAtlas {
    let width = packed.atlas.width();
    let height = packed.atlas.height();
    let aligned = width.div_ceil(64) * 64;
    if aligned == width {
        return packed;
    }
    let mut padded = RgbaImage::new(aligned, height);
    for y in 0..height {
        for x in 0..width {
            padded.put_pixel(x, y, *packed.atlas.get_pixel(x, y));
        }
    }
    assets::PackedAtlas {
        atlas: padded,
        index: packed.index,
    }
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
        let mut failed = Vec::new();
        for (name, img) in placeholder_frames() {
            let (span, flat_frac) = assets::gradient::gradient_shading_stats(&img);
            let ok = is_smooth_gradient_shaded(&img);
            println!("{name}: span={span} flat_frac={flat_frac:.3}");
            if !ok {
                failed.push(name.clone());
            }
            if std::env::var("SPRITE_DEBUG_DIR").is_ok() {
                let dir = std::env::var("SPRITE_DEBUG_DIR").unwrap();
                std::fs::create_dir_all(&dir).unwrap();
                img.save(format!("{dir}/{name}.png")).unwrap();
            }
        }
        assert!(
            failed.is_empty(),
            "{failed:?} are flat or banded — HD sprites shade smoothly, no pixel art"
        );
    }

    #[test]
    fn combat_swing_frames_exist_and_shade_in_the_hd_style() {
        let frames = placeholder_frames();
        let names: Vec<&str> = frames.iter().map(|(name, _)| name.as_str()).collect();
        // The player's swing poses.
        for name in ["bike_punch", "bike_kick"] {
            assert!(
                names.contains(&name),
                "{name} missing — combat ships with HD swing poses"
            );
        }
        // And the rivals' — one swing pair per rival livery, so a rival
        // mid-retaliation poses in its own colors.
        for name in [
            "bike_punch_rival",
            "bike_kick_rival",
            "bike_punch_rival2",
            "bike_kick_rival2",
            "bike_punch_rival3",
            "bike_kick_rival3",
        ] {
            assert!(names.contains(&name), "{name} missing");
        }
        // Every combat frame obeys the HD bar: no flat fills, no banding.
        let mut failed = Vec::new();
        for (name, img) in frames {
            if (name.starts_with("bike_punch") || name.starts_with("bike_kick"))
                && !is_smooth_gradient_shaded(&img)
            {
                failed.push(name);
            }
        }
        assert!(
            failed.is_empty(),
            "{failed:?} — swing poses must shade smoothly like the rest of the HD set"
        );
    }

    #[test]
    fn world_construction_frames_exist_in_the_hd_set() {
        let frames = placeholder_frames();
        let names: Vec<&str> = frames.iter().map(|(name, _)| name.as_str()).collect();
        // Zone dressing: palm (coast), building and fence (town), rock
        // (ridge), power pole (dense zones), 500 m marker post.
        for name in ["palm", "building", "fence", "pole", "marker", "rock"] {
            assert!(
                names.contains(&name),
                "{name} missing — zones need their own dressing"
            );
        }
        // Landmarks: the large "you are here" anchors.
        for name in ["water_tower", "windmill", "billboard"] {
            assert!(
                names.contains(&name),
                "{name} missing — landmarks ride the atlas"
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

    #[test]
    fn atlas_width_satisfies_the_webgl2_row_alignment_floor() {
        let packed = pack_placeholder_atlas().expect("placeholder atlas packs");
        assert_eq!(
            (packed.atlas.width() * 4) % 256,
            0,
            "atlas rows must upload without a staging copy on WebGL2"
        );
    }

    #[test]
    fn the_packed_atlas_fits_the_stated_byte_budget() {
        // Item 16-test: the packer's 4096 px dimension cap alone would allow
        // a 64 MB RGBA8 texture; the real pack must sit inside the stated
        // budget (render::check_atlas_budget), or the suite fails.
        let packed = pack_placeholder_atlas().expect("placeholder atlas packs");
        let dims = render::AtlasDims::new(packed.atlas.width(), packed.atlas.height());
        let bytes = render::check_atlas_budget(dims)
            .expect("the packed atlas must fit the stated byte budget");
        assert!(bytes > 0, "the packed atlas must be non-empty");
    }
}
