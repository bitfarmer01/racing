//! Sprite atlas packer for the racing runtime.
//!
//! Reads PNG sprites from a directory, shelf-packs them into a single atlas
//! [`RgbaImage`], and emits a name -> (x, y, w, h) index for the game runtime.
//! Packing is a pure function of the input set: sprites are ordered by
//! descending height then name, so the same input always yields the same atlas.

#![forbid(unsafe_code)]

pub mod gradient;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use image::{GenericImage, ImageError, RgbaImage};
use serde::Serialize;

/// Default atlas edge limit in pixels.
pub const DEFAULT_MAX_SIZE: u32 = 4096;

/// Default inter-sprite and edge padding in pixels.
pub const DEFAULT_PADDING: u32 = 2;

/// Packing parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackOptions {
    /// Pixels kept clear between neighbouring sprites, both within a row and
    /// between shelf rows. Padding is not applied against the atlas edges.
    pub padding: u32,
    /// Upper bound on either atlas edge.
    pub max_size: u32,
}

impl PackOptions {
    pub fn new(padding: u32, max_size: u32) -> Self {
        Self { padding, max_size }
    }
}

impl Default for PackOptions {
    fn default() -> Self {
        Self::new(DEFAULT_PADDING, DEFAULT_MAX_SIZE)
    }
}

/// One sprite's placement in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct SpriteRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// A packed atlas plus its index. `index` keys are sprite names (file stems).
#[derive(Debug, Clone)]
pub struct PackedAtlas {
    pub atlas: RgbaImage,
    pub index: BTreeMap<String, SpriteRect>,
}

/// Everything that can go wrong while packing.
#[derive(Debug)]
pub enum PackError {
    Io(std::io::Error),
    Image(ImageError),
    Json(serde_json::Error),
    NoSprites,
    DuplicateName(String),
    OversizedSprite {
        name: String,
        width: u32,
        height: u32,
        max_size: u32,
    },
    AtlasTooLarge {
        width: u32,
        height: u32,
        max_size: u32,
    },
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Image(e) => write!(f, "image error: {e}"),
            Self::Json(e) => write!(f, "json error: {e}"),
            Self::NoSprites => write!(f, "no PNG sprites found in input directory"),
            Self::DuplicateName(name) => write!(f, "duplicate sprite name: {name}"),
            Self::OversizedSprite {
                name,
                width,
                height,
                max_size,
            } => write!(
                f,
                "sprite '{name}' is {width}x{height}, which does not fit an atlas edge of {max_size} px"
            ),
            Self::AtlasTooLarge { width, height, max_size } => write!(
                f,
                "packed atlas is {width}x{height}, exceeding the {max_size} px atlas edge limit"
            ),
        }
    }
}

impl std::error::Error for PackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Image(e) => Some(e),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PackError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ImageError> for PackError {
    fn from(e: ImageError) -> Self {
        Self::Image(e)
    }
}

impl From<serde_json::Error> for PackError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Load every PNG under `dir` (file stem becomes the sprite name) and pack it.
///
/// Directory order is irrelevant to the result: entries are sorted by name
/// before packing, and the packer imposes its own deterministic ordering.
pub fn pack_directory(dir: &Path, options: PackOptions) -> Result<PackedAtlas, PackError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let is_png = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
        if is_png && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(PackError::NoSprites);
    }

    let mut sprites = Vec::with_capacity(files.len());
    for path in &files {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                PackError::Io(std::io::Error::other(format!(
                    "sprite path is not valid UTF-8: {}",
                    path.display()
                )))
            })?
            .to_owned();
        let image = image::open(path)?.into_rgba8();
        sprites.push((name, image));
    }
    pack(options, &sprites)
}

/// Pack a pre-loaded sprite set. Names must be unique; order does not matter.
///
/// A sprite is rejected when `w + padding` or `h + padding` exceeds
/// `max_size` — it could not fit on any row of the atlas with its padding
/// share intact. The packed height is checked against `max_size` too; the
/// chosen width never exceeds it.
pub fn pack(
    options: PackOptions,
    sprites: &[(String, RgbaImage)],
) -> Result<PackedAtlas, PackError> {
    if sprites.is_empty() {
        return Err(PackError::NoSprites);
    }

    // Validate names and sizes before allocating anything.
    let mut seen = std::collections::BTreeSet::new();
    for (name, image) in sprites {
        if !seen.insert(name.as_str()) {
            return Err(PackError::DuplicateName(name.clone()));
        }
        let (w, h) = (image.width(), image.height());
        let budget = options.max_size.saturating_sub(options.padding);
        if w > budget || h > budget {
            return Err(PackError::OversizedSprite {
                name: name.clone(),
                width: w,
                height: h,
                max_size: options.max_size,
            });
        }
    }

    // Deterministic placement order: tallest first, name breaks ties.
    let mut order: Vec<usize> = (0..sprites.len()).collect();
    order.sort_by(|&a, &b| {
        let (ha, hb) = (sprites[a].1.height(), sprites[b].1.height());
        hb.cmp(&ha).then_with(|| sprites[a].0.cmp(&sprites[b].0))
    });

    // Choose a width large enough for the tallest-first shelf layout to
    // approach a square: at least the widest sprite's cell, grown until the
    // width covers the total cell area. Pure function of the input set.
    let widest = sprites
        .iter()
        .map(|(_, img)| img.width() + options.padding)
        .max()
        .expect("sprites is non-empty; checked above");
    let total_area: u64 = sprites
        .iter()
        .map(|(_, img)| {
            (img.width() as u64 + options.padding as u64)
                * (img.height() as u64 + options.padding as u64)
        })
        .sum();
    let mut width = widest.min(options.max_size);
    while width < options.max_size && u64::from(width) * u64::from(width) < total_area {
        width = width.saturating_mul(2).min(options.max_size);
    }

    // Shelf packing: fill rows left to right, wrap at the right edge.
    let mut rects = vec![
        SpriteRect {
            x: 0,
            y: 0,
            w: 0,
            h: 0
        };
        sprites.len()
    ];
    let mut cursor_x = 0u32;
    let mut cursor_y = 0u32;
    let mut row_height = 0u32;
    let mut total_height = 0u32;
    for &i in &order {
        let img = &sprites[i].1;
        let (w, h) = (img.width(), img.height());
        if cursor_x + w > width {
            cursor_y += row_height + options.padding;
            cursor_x = 0;
            row_height = 0;
        }
        rects[i] = SpriteRect {
            x: cursor_x,
            y: cursor_y,
            w,
            h,
        };
        cursor_x += w + options.padding;
        row_height = row_height.max(h);
        total_height = total_height.max(cursor_y + h);
    }
    if total_height > options.max_size {
        return Err(PackError::AtlasTooLarge {
            width,
            height: total_height,
            max_size: options.max_size,
        });
    }

    let mut atlas = RgbaImage::new(width, total_height);
    let mut index = BTreeMap::new();
    for (i, (name, img)) in sprites.iter().enumerate() {
        atlas.copy_from(img, rects[i].x, rects[i].y)?;
        index.insert(name.clone(), rects[i]);
    }

    Ok(PackedAtlas { atlas, index })
}

/// Write the atlas PNG and index JSON as `<prefix>.png` / `<prefix>.json`,
/// creating the output directory when it does not exist.
pub fn write_outputs(packed: &PackedAtlas, out_prefix: &Path) -> Result<(), PackError> {
    let png_path = out_prefix.with_extension("png");
    let json_path = out_prefix.with_extension("json");
    if let Some(parent) = png_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    packed.atlas.save(&png_path)?;
    let json = serde_json::to_string_pretty(&packed.index)?;
    std::fs::write(&json_path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small RGBA image filled with a name-derived color, so copied
    /// pixels can be positively identified inside the packed atlas.
    fn sprite(name: &str, w: u32, h: u32) -> (String, RgbaImage) {
        let hash = name.bytes().fold(0x811c_9dc5u32, |acc, b| {
            (acc ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
        let color = image::Rgba([hash as u8, (hash >> 8) as u8, (hash >> 16) as u8, 255]);
        (name.to_owned(), RgbaImage::from_fn(w, h, |_, _| color))
    }

    /// Panics unless every pair of rects keeps at least `padding` px clear in
    /// some axis — the definition of "padding honored".
    fn assert_separated(index: &BTreeMap<String, SpriteRect>, padding: u32) {
        let rects: Vec<&SpriteRect> = index.values().collect();
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let separated = a.x + a.w + padding <= b.x
                    || b.x + b.w + padding <= a.x
                    || a.y + a.h + padding <= b.y
                    || b.y + b.h + padding <= a.y;
                assert!(
                    separated,
                    "sprites within {padding} px of each other: {a:?} vs {b:?}"
                );
            }
        }
    }

    fn mixed_sprites() -> Vec<(String, RgbaImage)> {
        [
            ("bike_a", 16, 16),
            ("bike_b", 32, 8),
            ("rider", 8, 64),
            ("tree", 48, 48),
            ("rock", 12, 7),
            ("hud_bar", 100, 10),
        ]
        .iter()
        .map(|(n, w, h)| sprite(n, *w, *h))
        .collect()
    }

    #[test]
    fn packs_all_sprites_without_overlap() {
        let sprites = mixed_sprites();
        let options = PackOptions::new(3, 256);
        let packed = pack(options, &sprites).expect("mixed sprites should pack");

        assert_eq!(packed.index.len(), sprites.len(), "every sprite in index");
        for (name, img) in &sprites {
            let rect = packed.index[name.as_str()];
            assert_eq!((rect.w, rect.h), (img.width(), img.height()), "for {name}");
            assert!(
                rect.x + rect.w <= packed.atlas.width() && rect.y + rect.h <= packed.atlas.height(),
                "rect for {name} escapes the atlas"
            );
            assert_eq!(
                packed.atlas.get_pixel(rect.x, rect.y),
                img.get_pixel(0, 0),
                "atlas pixel at {name}'s origin should be that sprite's color"
            );
        }
        assert_separated(&packed.index, 3);
    }

    #[test]
    fn layout_is_deterministic() {
        let options = PackOptions::new(2, 512);
        let sprites = mixed_sprites();

        let first = pack(options, &sprites).unwrap();
        let second = pack(options, &sprites).unwrap();
        assert_eq!(first.index, second.index, "same input, same index");
        assert_eq!(first.atlas.dimensions(), second.atlas.dimensions());
        assert!(
            first
                .atlas
                .pixels()
                .zip(second.atlas.pixels())
                .all(|(p, q)| p == q),
            "same input, same atlas pixels"
        );

        let mut reversed = sprites.clone();
        reversed.reverse();
        let third = pack(options, &reversed).unwrap();
        assert_eq!(first.index, third.index, "input order must not matter");
    }

    #[test]
    fn padding_is_honored() {
        let sprites = mixed_sprites();
        let options = PackOptions::new(6, 512);
        let packed = pack(options, &sprites).unwrap();
        assert_separated(&packed.index, 6);
    }

    #[test]
    fn rejects_oversized_sprite_with_clear_error() {
        let sprites = vec![sprite("tiny", 8, 8), sprite("huge", 300, 50)];
        let err = pack(PackOptions::new(0, 256), &sprites).unwrap_err();
        match &err {
            PackError::OversizedSprite {
                name,
                width,
                height,
                max_size,
            } => {
                assert_eq!(name, "huge");
                assert_eq!((*width, *height, *max_size), (300, 50, 256));
            }
            other => panic!("expected OversizedSprite, got: {other:?}"),
        }
        let message = err.to_string();
        assert!(
            message.contains("huge"),
            "error must name the sprite: {message}"
        );
        assert!(
            message.contains("300"),
            "error must state the size: {message}"
        );
    }

    #[test]
    fn pack_directory_loads_pngs_ignores_other_files_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("assets-pack-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp sprite dir");

        let mut names = Vec::new();
        for (name, w, h) in [("bike_a", 16, 16), ("bike_b", 32, 8), ("tree", 24, 40)] {
            let (name, img) = sprite(name, w, h);
            img.save(dir.join(format!("{name}.png"))).unwrap();
            names.push(name);
        }
        std::fs::write(dir.join("notes.txt"), b"not a sprite").unwrap();

        let packed = pack_directory(&dir, PackOptions::default()).unwrap();
        assert_eq!(packed.index.len(), names.len());
        for name in &names {
            assert!(packed.index.contains_key(name));
        }

        let prefix = dir.join("out").join("atlas");
        write_outputs(&packed, &prefix).unwrap();
        let json = std::fs::read_to_string(prefix.with_extension("json")).unwrap();
        let parsed: BTreeMap<String, SpriteRect> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, packed.index, "index round-trips through JSON");
        assert!(prefix.with_extension("png").is_file());

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(matches!(
            pack_directory(&empty, PackOptions::default()),
            Err(PackError::NoSprites)
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
