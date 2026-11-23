//! Input ring buffer, drained exactly once at tick start.
//!
//! The player's intent enters the simulation through this buffer and nowhere
//! else: the tick drains it at the top of every tick, so an input ages at
//! most one tick before the simulation sees it.
//!
//! If the buffer overflows before a drain, the oldest input is dropped —
//! stale intent is worth less than fresh intent.

use std::array;

/// Capacity of the default input buffer: comfortably more than a human can
/// produce between two 60 Hz ticks.
pub const DEFAULT_INPUT_CAP: usize = 128;

/// Fixed-capacity FIFO ring of inputs. Producers push; the tick's single
/// [`InputBuffer::drain`] consumes everything pushed so far, after which the
/// buffer is empty and a second drain yields nothing.
#[derive(Clone, Copy, Debug)]
pub struct InputBuffer<T, const CAP: usize = DEFAULT_INPUT_CAP> {
    /// Ring slots; `None` marks empty slots.
    slots: [Option<T>; CAP],
    /// Index of the oldest unconsumed input.
    head: usize,
    /// Number of unconsumed inputs, always `<= CAP`.
    len: usize,
}

impl<T, const CAP: usize> InputBuffer<T, CAP> {
    /// An empty buffer.
    pub fn new() -> Self {
        Self {
            slots: array::from_fn(|_| None),
            head: 0,
            len: 0,
        }
    }

    /// Enqueue one input. When full, the oldest input is dropped to make
    /// room. Never allocates.
    pub fn push(&mut self, input: T) {
        let next = (self.head + self.len) % CAP;
        if self.len == CAP {
            // Full: overwrite the oldest and slide the window forward.
            self.slots[next] = Some(input);
            self.head = (self.head + 1) % CAP;
        } else {
            self.slots[next] = Some(input);
            self.len += 1;
        }
    }

    /// Drain every buffered input, oldest first. The buffer is emptied by
    /// the returned iterator — consuming none of it, some of it, or dropping
    /// the iterator early all end in the same state: empty.
    ///
    /// Call this exactly once, at tick start.
    pub fn drain(&mut self) -> Drain<'_, T, CAP> {
        let remaining = self.len;
        Drain {
            buffer: self,
            remaining,
        }
    }

    /// Number of inputs waiting to be drained.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no inputs are waiting.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T, const CAP: usize> Default for InputBuffer<T, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator returned by [`InputBuffer::drain`]. Consumes the buffer as it
/// yields; anything not consumed is discarded when it is dropped.
#[derive(Debug)]
pub struct Drain<'a, T, const CAP: usize> {
    buffer: &'a mut InputBuffer<T, CAP>,
    remaining: usize,
}

impl<T, const CAP: usize> Iterator for Drain<'_, T, CAP> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.remaining == 0 {
            return None;
        }
        let input = self.buffer.slots[self.buffer.head].take();
        self.buffer.head = (self.buffer.head + 1) % CAP;
        self.buffer.len -= 1;
        self.remaining -= 1;
        input
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T, const CAP: usize> ExactSizeIterator for Drain<'_, T, CAP> {}

impl<T, const CAP: usize> Drop for Drain<'_, T, CAP> {
    fn drop(&mut self) {
        // Drain semantics: unconsumed inputs are discarded, not left behind.
        while self.next().is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_with(values: &[u64]) -> InputBuffer<u64> {
        let mut buffer = InputBuffer::new();
        for value in values {
            buffer.push(*value);
        }
        buffer
    }

    #[test]
    fn drains_in_fifo_order() {
        let mut buffer = buffer_with(&[1, 2, 3]);
        let drained: Vec<u64> = buffer.drain().collect();
        assert_eq!(drained, vec![1, 2, 3]);
    }

    #[test]
    fn drain_consumes_everything_exactly_once() {
        let mut buffer = buffer_with(&[7, 8]);
        assert_eq!(buffer.drain().count(), 2);
        assert!(buffer.is_empty());
        // The second drain must yield nothing.
        assert_eq!(buffer.drain().count(), 0);
        assert_eq!(buffer.drain().count(), 0);
    }

    #[test]
    fn buffer_accepts_pushes_after_a_drain() {
        let mut buffer = buffer_with(&[1]);
        buffer.drain().for_each(drop);
        buffer.push(2);
        let drained: Vec<u64> = buffer.drain().collect();
        assert_eq!(drained, vec![2]);
    }

    #[test]
    fn overflow_drops_the_oldest_input() {
        let mut buffer: InputBuffer<u8, 4> = InputBuffer::new();
        for value in 1..=5u8 {
            buffer.push(value);
        }
        assert_eq!(buffer.len(), 4);
        let drained: Vec<u8> = buffer.drain().collect();
        // Input 1 was the oldest and was dropped.
        assert_eq!(drained, vec![2, 3, 4, 5]);
    }

    #[test]
    fn partially_consumed_drain_discards_the_rest() {
        let mut buffer = buffer_with(&[1, 2, 3]);
        {
            let mut drain = buffer.drain();
            assert_eq!(drain.next(), Some(1));
            // Dropped here without consuming 2 and 3.
        }
        assert!(buffer.is_empty());
        assert_eq!(buffer.drain().count(), 0);
    }

    #[test]
    fn ring_wraps_around_the_cap_boundary() {
        let mut buffer: InputBuffer<u8, 4> = InputBuffer::new();
        for value in 1..=4u8 {
            buffer.push(value);
        }
        buffer.drain().for_each(drop);
        for value in 5..=8u8 {
            buffer.push(value);
        }
        let drained: Vec<u8> = buffer.drain().collect();
        assert_eq!(drained, vec![5, 6, 7, 8]);
    }

    #[test]
    fn size_hint_reflects_remaining_inputs() {
        let mut buffer = buffer_with(&[1, 2, 3]);
        let mut drain = buffer.drain();
        assert_eq!(drain.size_hint(), (3, Some(3)));
        drain.next();
        assert_eq!(drain.size_hint(), (2, Some(2)));
    }
}
