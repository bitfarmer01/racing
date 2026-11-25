//! Generation-indexed, structure-of-arrays entity pool for concurrent bikes.
//!
//! Bikes live in fixed-capacity parallel arrays (no heap, no resizing) with a
//! free list for slot reuse. Handles carry a per-slot generation counter, so
//! a handle to a removed-and-recycled slot is rejected instead of silently
//! addressing the slot's new occupant.
//!
//! Sized for a full grid of riders: [`MAX_BIKES`] concurrent
//! bikes, never felt as a cap.

/// Maximum concurrent bikes the pool can hold.
pub const MAX_BIKES: usize = 64;

/// A stable reference to one bike slot. Only the pool can mint handles, so a
/// handle's index and generation are always internally consistent at creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BikeHandle {
    index: u32,
    generation: u32,
}

/// Snapshot of one bike's simulation state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BikeState {
    /// Position along the track's forward axis, in meters.
    pub pos_x: f32,
    /// Lateral position across the road, in meters.
    pub pos_y: f32,
    /// Forward speed, in meters per tick.
    pub speed: f32,
    /// Facing angle, in radians.
    pub heading: f32,
}

/// Fixed-capacity bike pool: parallel state arrays plus a free-list stack and
/// per-slot generation counters.
#[derive(Clone, Debug)]
pub struct BikePool {
    /// Which slots currently hold a live bike.
    alive: [bool; MAX_BIKES],
    /// Generation per slot, bumped on every remove. Handles whose generation
    /// no longer matches are stale.
    generations: [u32; MAX_BIKES],
    /// Free-list stack of vacant slot indices.
    free: [u8; MAX_BIKES],
    /// Number of entries in the free-list stack.
    free_len: usize,
    // Structure-of-arrays state, indexed by slot.
    pos_x: [f32; MAX_BIKES],
    pos_y: [f32; MAX_BIKES],
    speed: [f32; MAX_BIKES],
    heading: [f32; MAX_BIKES],
}

impl BikePool {
    /// An empty pool with every slot free.
    pub fn new() -> Self {
        // Seeded so the first pop hands out slot 0, the second slot 1, and so
        // on: deterministic slot assignment while the pool fills.
        let free = std::array::from_fn(|i| (MAX_BIKES - 1 - i) as u8);
        Self {
            alive: [false; MAX_BIKES],
            generations: [0; MAX_BIKES],
            free,
            free_len: MAX_BIKES,
            pos_x: [0.0; MAX_BIKES],
            pos_y: [0.0; MAX_BIKES],
            speed: [0.0; MAX_BIKES],
            heading: [0.0; MAX_BIKES],
        }
    }

    /// Spawn a bike with the given state. Returns `None` when the pool is
    /// full — the hard cap of [`MAX_BIKES`] is the memory guarantee.
    pub fn spawn(&mut self, state: BikeState) -> Option<BikeHandle> {
        if self.free_len == 0 {
            return None;
        }
        self.free_len -= 1;
        let index = self.free[self.free_len] as usize;
        self.alive[index] = true;
        self.pos_x[index] = state.pos_x;
        self.pos_y[index] = state.pos_y;
        self.speed[index] = state.speed;
        self.heading[index] = state.heading;
        Some(BikeHandle {
            index: index as u32,
            generation: self.generations[index],
        })
    }

    /// Remove the bike `handle` refers to. Returns `false` — leaving the pool
    /// untouched — when the handle is stale (bike already removed, or the
    /// slot was recycled). The freed slot's generation is bumped so the old
    /// handle can never match again.
    pub fn remove(&mut self, handle: BikeHandle) -> bool {
        if !self.is_alive(handle) {
            return false;
        }
        let index = handle.index as usize;
        self.alive[index] = false;
        self.generations[index] = self.generations[index].wrapping_add(1);
        self.free[self.free_len] = index as u8;
        self.free_len += 1;
        true
    }

    /// Whether `handle` refers to a live bike.
    pub fn is_alive(&self, handle: BikeHandle) -> bool {
        let index = handle.index as usize;
        index < MAX_BIKES && self.alive[index] && self.generations[index] == handle.generation
    }

    /// Read one bike's state. `None` when the handle is stale.
    pub fn get(&self, handle: BikeHandle) -> Option<BikeState> {
        if !self.is_alive(handle) {
            return None;
        }
        let i = handle.index as usize;
        Some(BikeState {
            pos_x: self.pos_x[i],
            pos_y: self.pos_y[i],
            speed: self.speed[i],
            heading: self.heading[i],
        })
    }

    /// Overwrite one bike's state. Returns `false` when the handle is stale.
    pub fn set(&mut self, handle: BikeHandle, state: BikeState) -> bool {
        if !self.is_alive(handle) {
            return false;
        }
        let i = handle.index as usize;
        self.pos_x[i] = state.pos_x;
        self.pos_y[i] = state.pos_y;
        self.speed[i] = state.speed;
        self.heading[i] = state.heading;
        true
    }

    /// Number of live bikes.
    pub fn live_count(&self) -> usize {
        MAX_BIKES - self.free_len
    }

    /// Pool capacity, always [`MAX_BIKES`].
    pub fn capacity(&self) -> usize {
        MAX_BIKES
    }
}

impl Default for BikePool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(x: f32) -> BikeState {
        BikeState {
            pos_x: x,
            pos_y: 0.0,
            speed: 1.0,
            heading: 0.0,
        }
    }

    #[test]
    fn spawn_fills_slots_in_order_up_to_the_cap() {
        let mut pool = BikePool::new();
        let mut handles = Vec::new();
        for i in 0..MAX_BIKES as u32 {
            let handle = pool.spawn(state(i as f32)).expect("pool below cap");
            assert_eq!(handle.index, i, "slots hand out in order while filling");
            handles.push(handle);
        }
        assert_eq!(pool.live_count(), MAX_BIKES);
        // The next spawn must be refused: the cap is hard.
        assert!(pool.spawn(state(999.0)).is_none());
        assert_eq!(pool.live_count(), MAX_BIKES);
    }

    #[test]
    fn removed_slots_are_reused_by_the_free_list() {
        let mut pool = BikePool::new();
        let a = pool.spawn(state(1.0)).unwrap();
        let b = pool.spawn(state(2.0)).unwrap();
        pool.remove(a).should_be_true();
        pool.remove(b).should_be_true();
        // LIFO free list: the most recently freed slot (b's) is reused first.
        let reused = pool.spawn(state(3.0)).unwrap();
        assert_eq!(reused.index, b.index);
        let reused_two = pool.spawn(state(4.0)).unwrap();
        assert_eq!(reused_two.index, a.index);
        assert_eq!(pool.live_count(), 2);
    }

    #[test]
    fn stale_handle_is_rejected_after_slot_recycling() {
        let mut pool = BikePool::new();
        let original = pool.spawn(state(1.0)).unwrap();
        assert!(pool.remove(original));
        // Slot recycled: same index, bumped generation.
        let replacement = pool.spawn(state(2.0)).unwrap();
        assert_eq!(replacement.index, original.index);
        assert_ne!(replacement.generation, original.generation);
        // The old handle must not read, write, or remove the new occupant.
        assert!(pool.get(original).is_none());
        assert!(!pool.set(original, state(9.0)));
        assert!(!pool.remove(original));
        assert_eq!(pool.get(replacement).unwrap().pos_x, 2.0);
    }

    #[test]
    fn double_remove_is_rejected() {
        let mut pool = BikePool::new();
        let handle = pool.spawn(state(1.0)).unwrap();
        assert!(pool.remove(handle));
        assert!(!pool.remove(handle));
        assert_eq!(pool.live_count(), 0);
    }

    #[test]
    fn set_updates_only_the_targeted_bike() {
        let mut pool = BikePool::new();
        let a = pool.spawn(state(1.0)).unwrap();
        let b = pool.spawn(state(2.0)).unwrap();
        assert!(pool.set(a, state(10.0)));
        assert_eq!(pool.get(a).unwrap().pos_x, 10.0);
        assert_eq!(pool.get(b).unwrap().pos_x, 2.0);
    }

    #[test]
    fn freed_slots_spawn_in_a_deterministic_cycle() {
        // Same sequence of operations on two pools must produce identical
        // handle index/generation traces — replay determinism depends on it.
        let trace = |operations: &[f32]| {
            let mut pool = BikePool::new();
            let mut live = Vec::new();
            let mut trace = Vec::new();
            for (step, x) in operations.iter().enumerate() {
                if step % 3 == 2 {
                    let handle = live.remove(0);
                    pool.remove(handle);
                    trace.push((handle.index, handle.generation, u32::MAX));
                } else {
                    let handle = pool.spawn(state(*x)).unwrap();
                    trace.push((handle.index, handle.generation, *x as u32));
                    live.push(handle);
                }
            }
            trace
        };
        let ops: Vec<f32> = (0..30).map(|i| i as f32).collect();
        assert_eq!(trace(&ops), trace(&ops));
    }

    /// Small helper so `assert!(pool.remove(h))` reads as intent.
    trait ShouldBeTrue {
        fn should_be_true(&self);
    }
    impl ShouldBeTrue for bool {
        fn should_be_true(&self) {
            assert!(self);
        }
    }
}
