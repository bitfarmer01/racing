//! Quality tiers: the blueprint's four-step ladder. A tier scales instance
//! limits, resolution, and optional effects — it never gates gameplay.

/// Render quality tier, chosen once at load from device capability facts.
///
/// Names follow the blueprint's tier ladder: the WebGL2 path (the 2015 iGPU
/// floor) and the WebGPU path, each split low-end / capable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTier {
    WebGl2LowEnd,
    WebGl2Capable,
    WebGpuLowEnd,
    WebGpuCapable,
}

/// Optional post-processing and pixel-shader features a tier can afford.
///
/// All flags are off on the low-end WebGL2 tier: the grading LUT alone carries
/// the mood there, and the quarter-resolution bloom pass is the first thing
/// the blueprint says to cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    /// Threshold bloom + vignette quarter-res pass.
    pub bloom: bool,
    /// 16x16x16 grading LUT fullscreen pass (on for every tier — the mood
    /// carrier; listed as a flag so a future ultra-low tier could drop it).
    pub grading_lut: bool,
    /// Sun-lit normal maps on sprites (two texture samples per pixel).
    pub normal_mapping: bool,
    /// Dynamic resolution scaling toward the frame-time target.
    pub dynamic_resolution: bool,
}

impl Effects {
    /// Nothing but the grading LUT.
    pub const MINIMAL: Self = Self {
        bloom: false,
        grading_lut: true,
        normal_mapping: false,
        dynamic_resolution: false,
    };

    /// Everything the blueprint allows.
    pub const FULL: Self = Self {
        bloom: true,
        grading_lut: true,
        normal_mapping: true,
        dynamic_resolution: true,
    };
}

impl QualityTier {
    /// Hard cap on batched sprites per frame. Caps are well above what a
    /// Road Rash-scale scene needs (~64 concurrent actors) so the limit is
    /// never felt in play — it exists to bound memory, not gameplay.
    pub const fn max_instances(self) -> usize {
        match self {
            Self::WebGl2LowEnd => 512,
            Self::WebGl2Capable => 2048,
            Self::WebGpuLowEnd => 1024,
            Self::WebGpuCapable => 4096,
        }
    }

    /// Fraction of native resolution the game renders at before upscale.
    /// The weakest tier accepts the most softness; the tier ladder in the
    /// blueprint runs 0.6–0.8.
    pub const fn resolution_scale(self) -> f32 {
        match self {
            Self::WebGl2LowEnd => 0.6,
            Self::WebGl2Capable => 0.75,
            Self::WebGpuLowEnd => 0.7,
            Self::WebGpuCapable => 0.8,
        }
    }

    /// Optional effects this tier enables.
    pub const fn effects(self) -> Effects {
        match self {
            Self::WebGl2LowEnd => Effects::MINIMAL,
            Self::WebGl2Capable => Effects {
                bloom: false,
                grading_lut: true,
                normal_mapping: true,
                dynamic_resolution: true,
            },
            Self::WebGpuLowEnd => Effects {
                bloom: true,
                grading_lut: true,
                normal_mapping: false,
                dynamic_resolution: true,
            },
            Self::WebGpuCapable => Effects::FULL,
        }
    }

    /// Pure tier pick from the loader's capability probes — the JS bootstrap
    /// decides `has_webgpu` (adapter present) and `low_end_device`, this maps
    /// the two facts onto the ladder with no device access.
    pub const fn select(has_webgpu: bool, low_end_device: bool) -> Self {
        match (has_webgpu, low_end_device) {
            (true, false) => Self::WebGpuCapable,
            (true, true) => Self::WebGpuLowEnd,
            (false, false) => Self::WebGl2Capable,
            (false, true) => Self::WebGl2LowEnd,
        }
    }

    /// The tier the renderer's cargo feature build targets, as a backend tag.
    /// Two builds ship (WebGPU / WebGL2), selected at load time.
    pub const fn backend(self) -> Backend {
        match self {
            Self::WebGl2LowEnd | Self::WebGl2Capable => Backend::WebGl2,
            Self::WebGpuLowEnd | Self::WebGpuCapable => Backend::WebGpu,
        }
    }
}

/// Which shader backend the current build was compiled for. Two cargo
/// feature builds exist; the JS loader picks one, never runtime branching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    WebGl2,
    WebGpu,
}
