//! Difficulty levels.
//!
//! One knob that changes how the field races, not how the bike handles:
//! rivals' pace and cornering nerve, how often they fight and defend, how
//! hard hits land on the player, and how much the pack waits for (or runs
//! away from) a player who is off the pace.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Difficulty {
    /// Slower, gentler rivals that wait for you; hits on you land softer.
    Easy,
    #[default]
    Normal,
    /// Rivals at full pace and nerve who block and parry, hit harder and
    /// never ease off.
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hard => "Hard",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hard => "hard",
        }
    }

    /// Parses a slug; anything unknown is Normal.
    pub fn from_slug(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "easy" => Difficulty::Easy,
            "hard" => Difficulty::Hard,
            _ => Difficulty::Normal,
        }
    }

    /// Multiplies rivals' engine power and top speed.
    pub fn rival_pace(self) -> f32 {
        match self {
            Difficulty::Easy => 0.93,
            Difficulty::Normal => 1.0,
            Difficulty::Hard => 1.035,
        }
    }

    /// Added to how close to their grip limit rivals dare to corner.
    pub fn rival_nerve(self) -> f32 {
        match self {
            Difficulty::Easy => -0.07,
            Difficulty::Normal => 0.0,
            Difficulty::Hard => 0.035,
        }
    }

    /// Scales how often rivals start fights.
    pub fn rival_aggression(self) -> f32 {
        match self {
            Difficulty::Easy => 0.45,
            Difficulty::Normal => 1.0,
            Difficulty::Hard => 1.5,
        }
    }

    /// Chance a rival raises its guard against an attack it sees coming.
    pub fn rival_guard(self) -> f32 {
        match self {
            Difficulty::Easy => 0.1,
            Difficulty::Normal => 0.3,
            Difficulty::Hard => 0.6,
        }
    }

    /// Scales balance damage the player takes.
    pub fn damage_to_player(self) -> f32 {
        match self {
            Difficulty::Easy => 0.65,
            Difficulty::Normal => 1.0,
            Difficulty::Hard => 1.25,
        }
    }

    /// Power multiplier for a rival `gap` metres behind the player
    /// (negative gap: the rival is ahead). Easy holds the leaders back and
    /// never pushes the stragglers; Hard has no catch-up at all.
    pub fn catch_up(self, gap: f32) -> f32 {
        match self {
            Difficulty::Easy => {
                if gap < -60.0 {
                    1.0 - ((-gap - 60.0) / 200.0).min(1.0) * 0.1
                } else {
                    1.0
                }
            }
            Difficulty::Normal => {
                if gap > 120.0 {
                    1.04
                } else if gap < -160.0 {
                    0.95
                } else {
                    1.0
                }
            }
            Difficulty::Hard => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip() {
        for d in Difficulty::ALL {
            assert_eq!(Difficulty::from_slug(d.slug()), d);
        }
        assert_eq!(Difficulty::from_slug("nonsense"), Difficulty::Normal);
        assert_eq!(Difficulty::from_slug(" HARD "), Difficulty::Hard);
    }

    #[test]
    fn levels_are_ordered() {
        let [e, n, h] = Difficulty::ALL;
        assert!(e.rival_pace() < n.rival_pace() && n.rival_pace() < h.rival_pace());
        assert!(e.rival_guard() < n.rival_guard() && n.rival_guard() < h.rival_guard());
        assert!(e.damage_to_player() < h.damage_to_player());
        // Easy waits for a player far behind; Hard never does.
        assert!(e.catch_up(-300.0) < 1.0);
        assert_eq!(h.catch_up(-300.0), 1.0);
        assert_eq!(h.catch_up(300.0), 1.0);
    }
}
