//! Player intent, distilled from the sim input ring.
//!
//! The tick drains the [`sim::input::InputBuffer`] once and folds the events
//! into one [`Intent`]: the last throttle-family command wins, and the last
//! steering direction wins. One intent per tick keeps the state machine pure
//! and replayable.

/// One input event, as pushed by the platform layer into the sim input ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameInput {
    /// Throttle on.
    Accelerate,
    /// Brake on.
    Brake,
    /// Lean left.
    SteerLeft,
    /// Lean right.
    SteerRight,
}

/// Throttle family of the [`Intent`] for this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Throttle {
    /// No throttle or brake command seen: coast.
    #[default]
    Coast,
    /// The last command seen was `Accelerate`.
    Accelerate,
    /// The last command seen was `Brake`.
    Brake,
}

/// Steering direction of the [`Intent`] for this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Steer {
    /// No steering command seen.
    #[default]
    None,
    /// The last steering command seen was `SteerLeft`.
    Left,
    /// The last steering command seen was `SteerRight`.
    Right,
}

/// Per-tick control intent: the last command of each family wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Intent {
    /// Last throttle-family command seen this tick.
    pub throttle: Throttle,
    /// Last steering command seen this tick.
    pub steer: Steer,
}

impl Intent {
    /// Fold one drained input event into the running intent.
    pub fn absorb(&mut self, input: GameInput) {
        match input {
            GameInput::Accelerate => self.throttle = Throttle::Accelerate,
            GameInput::Brake => self.throttle = Throttle::Brake,
            GameInput::SteerLeft => self.steer = Steer::Left,
            GameInput::SteerRight => self.steer = Steer::Right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_command_of_each_family_wins() {
        let mut intent = Intent::default();
        for input in [
            GameInput::Accelerate,
            GameInput::Brake,
            GameInput::SteerRight,
            GameInput::SteerLeft,
        ] {
            intent.absorb(input);
        }
        assert_eq!(intent.throttle, Throttle::Brake);
        assert_eq!(intent.steer, Steer::Left);
    }

    #[test]
    fn an_empty_tick_is_full_coast() {
        let intent = Intent::default();
        assert_eq!(intent.throttle, Throttle::Coast);
        assert_eq!(intent.steer, Steer::None);
    }
}
