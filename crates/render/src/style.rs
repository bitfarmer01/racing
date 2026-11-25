//! The design system: one palette and one set of finish settings shared by
//! the 3D scene, the post "finish" and the HTML HUD (exported as CSS custom
//! properties by the web crate). Change a colour here and the bikes, flames,
//! impact frames, speed lines and HUD all move together.

use game::theme::{Landform, Theme};

/// An sRGB colour as authored (hex in the HUD, decoded to linear for shading).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Srgb(pub u8, pub u8, pub u8);

impl Srgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Linear-light RGB, for shaders and HDR particle colours.
    pub fn linear(self) -> [f32; 3] {
        let f = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [f(self.0), f(self.1), f(self.2)]
    }
}

/// Colours every track shares.
pub mod palette {
    use super::Srgb;
    /// Line work and the dark tone of impact frames.
    pub const INK: Srgb = Srgb(0x14, 0x12, 0x1a);
    /// Light tone: HUD text, speed lines, impact-frame paper.
    pub const PAPER: Srgb = Srgb(0xf4, 0xf1, 0xea);
    /// The player's colour: bike paint, HUD accents, the impact-frame hit tone.
    pub const HOT: Srgb = Srgb(0xff, 0x4d, 0x2e);
    /// Gear readout, rev band warning.
    pub const GOLD: Srgb = Srgb(0xff, 0xc2, 0x3d);
    /// Boost: flames, the boost meter, the tach while boosting.
    pub const BOOST: Srgb = Srgb(0x48, 0xc8, 0xff);
    /// Good news: personal bests, places gained, GO.
    pub const GOOD: Srgb = Srgb(0x6f, 0xe3, 0x9b);
}

/// Per-track part of the look.
#[derive(Debug, Clone, Copy)]
pub struct TrackStyle {
    /// Signature colour of the track: menu, minimap, lap banners.
    pub accent: Srgb,
    /// Ink colour for this light (warm or cool black).
    pub ink: Srgb,
    /// Outline strength 0..1.
    pub ink_strength: f32,
    /// Split-tone multipliers for shadows and highlights (linear).
    pub shadow_tint: [f32; 3],
    pub highlight_tint: [f32; 3],
    /// Film grain amount.
    pub grain: f32,
}

pub fn track_style(theme: &Theme) -> TrackStyle {
    match theme.landform {
        Landform::Coast => TrackStyle {
            accent: Srgb(0xff, 0xb3, 0x47),
            ink: Srgb(0x1f, 0x16, 0x1c),
            ink_strength: 0.75,
            shadow_tint: [0.93, 0.98, 1.05],
            highlight_tint: [1.04, 1.0, 0.95],
            grain: 0.035,
        },
        Landform::Alpine => TrackStyle {
            accent: Srgb(0x6f, 0xd3, 0xff),
            ink: Srgb(0x12, 0x1a, 0x26),
            ink_strength: 0.7,
            shadow_tint: [0.95, 0.99, 1.06],
            highlight_tint: [1.02, 1.0, 0.98],
            grain: 0.03,
        },
        Landform::Canyon => TrackStyle {
            accent: Srgb(0xff, 0x5f, 0xa2),
            ink: Srgb(0x22, 0x10, 0x1e),
            ink_strength: 0.8,
            shadow_tint: [1.0, 0.94, 1.08],
            highlight_tint: [1.05, 0.99, 0.92],
            grain: 0.04,
        },
    }
}

/// CSS custom properties for the page, as `--name: value;` pairs.
pub fn css_vars(theme: &Theme) -> String {
    use palette::*;
    let s = track_style(theme);
    let a = s.accent;
    [
        ("--ink", INK.hex()),
        ("--paper", PAPER.hex()),
        ("--hot", HOT.hex()),
        ("--gold", GOLD.hex()),
        ("--boost", BOOST.hex()),
        ("--good", GOOD.hex()),
        ("--accent", a.hex()),
        ("--accent-rgb", format!("{}, {}, {}", a.0, a.1, a.2)),
    ]
    .iter()
    .map(|(k, v)| format!("{k}: {v};"))
    .collect::<Vec<_>>()
    .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use game::tracks::catalog;

    #[test]
    fn srgb_decodes_to_linear() {
        assert_eq!(Srgb(0, 0, 0).linear(), [0.0; 3]);
        assert_eq!(Srgb(255, 255, 255).linear(), [1.0; 3]);
        let mid = Srgb(188, 188, 188).linear()[0];
        assert!((mid - 0.5).abs() < 0.01, "{mid}");
        assert_eq!(palette::HOT.hex(), "#ff4d2e");
    }

    #[test]
    fn every_track_has_a_distinct_accent() {
        let accents: Vec<String> = catalog()
            .iter()
            .map(|d| track_style(&d.theme).accent.hex())
            .collect();
        for (i, a) in accents.iter().enumerate() {
            assert!(!accents[i + 1..].contains(a), "{a} reused");
        }
        let css = css_vars(&catalog()[0].theme);
        assert!(
            css.contains("--hot: #ff4d2e;") && css.contains("--accent:"),
            "{css}"
        );
    }
}
