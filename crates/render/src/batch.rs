//! Sprite batching: a fixed-capacity instance list the frame loop only ever
//! pushes to and clears, so steady-state rendering allocates nothing.

use world::Viewport;

use crate::instance::SpriteInstance;

/// Fixed-capacity instance list.
///
/// Capacity comes from the quality tier and is allocated once at startup;
/// `push` beyond capacity is a rejected write, not a grow, keeping the
/// blueprint's no-allocation-in-the-loop invariant honest.
///
/// Draw order is insertion order: the game pushes sprites back-to-front in
/// painter's order (the road strips are drawn separately by the pipeline), so
/// the batch never sorts.
pub struct SpriteBatch {
    instances: Vec<SpriteInstance>,
}

impl SpriteBatch {
    /// An empty batch with exactly `capacity` slots preallocated.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            instances: Vec::with_capacity(capacity),
        }
    }

    /// An empty batch sized for the tier's instance limit.
    pub fn for_tier(tier: crate::tier::QualityTier) -> Self {
        Self::with_capacity(tier.max_instances())
    }

    /// Appends an instance. Returns `false` (and changes nothing) when the
    /// batch is full.
    pub fn push(&mut self, instance: SpriteInstance) -> bool {
        if self.instances.len() == self.instances.capacity() {
            return false;
        }
        self.instances.push(instance);
        true
    }

    /// Appends an instance unless it is entirely off-screen.
    ///
    /// Culling uses a rotation-proof bound: a rotated quad never leaves the
    /// circle of radius `hypot(w, h) / 2` around its centre, so a sprite whose
    /// circle misses the viewport is skipped. Returns whether the instance
    /// was pushed.
    pub fn push_on_screen(&mut self, viewport: Viewport, instance: SpriteInstance) -> bool {
        let [x, y] = instance.position;
        let [w, h] = instance.scale;
        let radius = 0.5 * (w * w + h * h).sqrt();
        let off_screen = x + radius < 0.0
            || x - radius > viewport.width
            || y + radius < 0.0
            || y - radius > viewport.height;
        if off_screen {
            return false;
        }
        self.push(instance)
    }

    /// Drops all instances, keeping the allocation. This is the only
    /// per-frame mutation besides `push`.
    pub fn clear(&mut self) {
        self.instances.clear();
    }

    /// Instances in push (painter's) order, ready for `write_buffer`.
    pub fn instances(&self) -> &[SpriteInstance] {
        &self.instances
    }

    /// Number of instances currently batched.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// True when nothing is batched.
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}
