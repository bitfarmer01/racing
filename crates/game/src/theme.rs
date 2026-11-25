//! Per-circuit environment: landform, light, sky and palette.
//!
//! Pure data. The game uses the landform to shape terrain and the road's
//! elevation; the renderer reads the light/sky/palette fields. Colours are
//! linear RGB; light intensities are HDR (values above 1 bloom).

use crate::noise::{fbm, ridged, smoothstep};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landform {
    /// Sea to the south, rolling headlands, mountains far to the north.
    Coast,
    /// A ring of snow-capped peaks around a high lake.
    Alpine,
    /// Red mesas and dry washes under a dusk sky.
    Canyon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeStyle {
    Pine,
    Broadleaf,
    Palm,
    Cactus,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub landform: Landform,
    pub seed: u32,
    /// Sun direction as (elevation, azimuth) in radians; azimuth 0 = +x.
    pub sun_elevation: f32,
    pub sun_azimuth: f32,
    pub sun_color: [f32; 3],
    pub sky_zenith: [f32; 3],
    pub sky_horizon: [f32; 3],
    /// Bounce light from the ground for the hemisphere ambient term.
    pub ground_bounce: [f32; 3],
    pub fog_density: f32,
    pub cloud_cover: f32,
    pub exposure: f32,
    pub water_level: f32,
    pub water_color: [f32; 3],
    pub grass: [f32; 3],
    pub grass_dry: [f32; 3],
    pub rock: [f32; 3],
    pub sand: [f32; 3],
    /// Height above which snow settles on gentle slopes (f32::MAX for none).
    pub snow_line: f32,
    pub trees: [TreeStyle; 2],
    /// Tree density multiplier (1 = forested).
    pub tree_density: f32,
    /// Street lamps along the circuit (they glow at dusk).
    pub lamps: bool,
    /// Grade: warm/cool tint applied in post (linear multipliers).
    pub grade: [f32; 3],
}

impl Theme {
    /// The landform's natural height at (x, z), before the road carves it.
    pub fn natural_height(&self, x: f32, z: f32) -> f32 {
        let s = self.seed;
        match self.landform {
            Landform::Coast => {
                // Sea south of z≈-330, headland plateau, mountains north.
                let shore = smoothstep(-560.0, -230.0, z + fbm(x * 0.004, z * 0.004, 2, s) * 90.0);
                let base = -22.0 + 34.0 * shore;
                let hills = fbm(x * 0.0045, z * 0.0045, 5, s) * 18.0 * shore;
                let far = smoothstep(500.0, 1700.0, z) + 0.6 * smoothstep(900.0, 2200.0, x.abs());
                let mountains = ridged(x * 0.0016, z * 0.0016, 5, s ^ 0xa5) * 330.0 * far;
                base + hills + mountains
            }
            Landform::Alpine => {
                let r = (x * x + z * z).sqrt();
                let bowl = -26.0 * (-(r / 210.0).powi(2)).exp();
                let rise = smoothstep(450.0, 1500.0, r);
                let hills = fbm(x * 0.005, z * 0.005, 5, s) * 22.0;
                let peaks = ridged(x * 0.0013, z * 0.0013, 6, s ^ 0x5c) * 620.0 * rise;
                18.0 + bowl + hills + peaks
            }
            Landform::Canyon => {
                // Flat washes broken by terraced mesas.
                let n = fbm(x * 0.0022, z * 0.0022, 4, s);
                let mesa = smoothstep(0.08, 0.22, n);
                let terrace = (mesa * 4.0).floor() / 4.0 * 0.5 + mesa * 0.5;
                let r = (x * x + z * z).sqrt();
                let rim = smoothstep(700.0, 1600.0, r);
                let cliffs = terrace * (60.0 + 140.0 * rim);
                let wash = fbm(x * 0.01, z * 0.01, 3, s ^ 3) * 3.0;
                8.0 + cliffs + wash + rim * 40.0
            }
        }
    }

    pub fn sun_direction(&self) -> [f32; 3] {
        let (e, a) = (self.sun_elevation, self.sun_azimuth);
        [e.cos() * a.cos(), e.sin(), e.cos() * a.sin()]
    }
}

pub const GOLDEN_COAST: Theme = Theme {
    landform: Landform::Coast,
    seed: 0x00c0_a571,
    sun_elevation: 0.16,
    sun_azimuth: -1.95,
    sun_color: [9.5, 6.2, 3.4],
    sky_zenith: [0.16, 0.26, 0.52],
    sky_horizon: [1.25, 0.78, 0.48],
    ground_bounce: [0.30, 0.22, 0.14],
    fog_density: 0.00085,
    cloud_cover: 0.42,
    exposure: 0.95,
    water_level: 0.0,
    water_color: [0.02, 0.09, 0.13],
    grass: [0.16, 0.24, 0.06],
    grass_dry: [0.42, 0.34, 0.14],
    rock: [0.36, 0.30, 0.25],
    sand: [0.72, 0.60, 0.42],
    snow_line: 260.0,
    trees: [TreeStyle::Pine, TreeStyle::Palm],
    tree_density: 0.85,
    lamps: true,
    grade: [1.06, 1.0, 0.92],
};

pub const ALPINE_LAKE: Theme = Theme {
    landform: Landform::Alpine,
    seed: 0x0a1b_e55e,
    sun_elevation: 0.62,
    sun_azimuth: 0.9,
    sun_color: [8.2, 7.7, 6.9],
    sky_zenith: [0.10, 0.26, 0.66],
    sky_horizon: [0.62, 0.78, 0.98],
    ground_bounce: [0.16, 0.20, 0.12],
    fog_density: 0.00055,
    cloud_cover: 0.36,
    exposure: 0.82,
    water_level: 0.0,
    water_color: [0.01, 0.07, 0.10],
    grass: [0.11, 0.22, 0.06],
    grass_dry: [0.27, 0.31, 0.11],
    rock: [0.32, 0.31, 0.30],
    sand: [0.50, 0.46, 0.38],
    snow_line: 150.0,
    trees: [TreeStyle::Pine, TreeStyle::Broadleaf],
    tree_density: 1.0,
    lamps: false,
    grade: [0.98, 1.0, 1.04],
};

pub const DUSK_CANYON: Theme = Theme {
    landform: Landform::Canyon,
    seed: 0x0d05_c4a7,
    sun_elevation: 0.05,
    sun_azimuth: 2.6,
    sun_color: [8.0, 3.6, 1.6],
    sky_zenith: [0.07, 0.07, 0.22],
    sky_horizon: [1.05, 0.46, 0.30],
    ground_bounce: [0.26, 0.12, 0.08],
    fog_density: 0.0011,
    cloud_cover: 0.30,
    exposure: 1.15,
    water_level: -50.0,
    water_color: [0.02, 0.05, 0.08],
    grass: [0.30, 0.22, 0.10],
    grass_dry: [0.52, 0.30, 0.16],
    rock: [0.55, 0.24, 0.13],
    sand: [0.70, 0.43, 0.25],
    snow_line: f32::MAX,
    trees: [TreeStyle::Cactus, TreeStyle::Broadleaf],
    tree_density: 0.22,
    lamps: true,
    grade: [1.08, 0.97, 0.95],
};
