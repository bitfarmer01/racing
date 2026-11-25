//! Adjacent-rider combat: the Road Rash core (wave 7b).
//!
//! One attack is an event with an auto-target: the nearest rider within the
//! adjacency window — alongside laterally and inside a short longitudinal
//! band. No target, no swing, no cost. Landed hits add instability to the
//! target and drain its stamina; instability past the threshold, or any hit
//! landing on a rider already out of stamina, dismounts them through the
//! existing crash-and-recover flow in [`crate::bike`] (the crash timer is the
//! down timer — the spec's `down_timer` state is that flow, not a second one).
//!
//! Every rider carries [`Combat`]: the player's stamina is the HUD bar, and
//! the same fields drive rival retaliation so both sides of a fight obey one
//! set of rules. Everything here is pure — same state, same outcome — so a
//! fight replays exactly under the same seed and inputs.

/// A hand-to-hand attack: the player's `X`/`Z`, a rival's retaliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attack {
    /// `X`: dismounts in fewer hits, shorter reach.
    Punch,
    /// `Z`: longer reach, shoves the target sideways — toward traffic when
    /// it is there.
    Kick,
}

/// Per-rider combat state. `down_timer` from the spec is deliberately
/// absent: a dismount is a crash, and the crash timer already lives on
/// [`crate::bike::BikeSnapshot`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Combat {
    /// Fight energy in `0..=1`: drains per attack thrown and hit taken,
    /// regens while clean. A hit landing at zero dismounts the rider.
    pub stamina: f32,
    /// Seconds since this rider's last swing. Ready to swing again at
    /// [`ATTACK_COOLDOWN`] (player) or the retaliation cooldown (rivals).
    /// Starts ready.
    pub cooldown: f32,
    /// Seconds left showing the swing pose; zero when riding clean.
    pub swing: f32,
    /// Which swing the pose shows, while `swing` is running.
    pub swing_attack: Option<Attack>,
}

impl Default for Combat {
    fn default() -> Self {
        Self {
            stamina: 1.0,
            cooldown: ATTACK_COOLDOWN,
            swing: 0.0,
            swing_attack: None,
        }
    }
}

/// Open one rider's swing: the pose shows for [`SWING_SECS`], carrying the
/// attack it is showing.
pub fn start_swing(combat: &mut Combat, attack: Attack) {
    combat.swing = SWING_SECS;
    combat.swing_attack = Some(attack);
}

/// Adjacency: lateral distance across the road under which two riders are
/// "alongside", in meters.
pub const ADJACENT_LATERAL: f32 = 2.5;
/// Punch reach along the road, in meters: short — a hooked punch, not a
/// jab across a gap.
pub const PUNCH_REACH: f32 = 2.4;
/// Kick reach along the road, in meters: a full legswing, longer than the
/// punch.
pub const KICK_REACH: f32 = 4.0;
/// Minimum seconds between the player's swings, in seconds.
pub const ATTACK_COOLDOWN: f32 = 0.4;
/// Base seconds between a rival's retaliatory swings, in seconds.
pub const RETALIATION_COOLDOWN: f32 = 1.2;
/// How much shorter the retaliation cooldown gets for a rival with traffic
/// alongside — a rider you knocked toward the cars is angrier.
pub const RETALIATION_TRAFFIC_RAGE: f32 = 2.0;
/// Longitudinal distance to a car under which a rider counts as "next to
/// traffic", in meters.
pub const NEAR_TRAFFIC_RANGE: f32 = 8.0;
/// Instability at which a rider dismounts.
pub const INSTABILITY_DISMOUNT: f32 = 100.0;
/// Instability one landed punch adds. Two punches at full stability cross
/// the threshold with the decay between swings accounted for.
pub const PUNCH_HIT: f32 = 55.0;
/// Instability one landed kick adds: three kicks cross the threshold.
pub const KICK_HIT: f32 = 40.0;
/// Instability that fades per second while a rider is not being hit, so
/// yesterday's hits do not wait in the bike frame to dismount them.
pub const INSTABILITY_DECAY: f32 = 10.0;
/// Stamina a landed punch costs the attacker.
pub const PUNCH_COST: f32 = 0.35;
/// Stamina a landed kick costs the attacker.
pub const KICK_COST: f32 = 0.25;
/// Stamina a landed hit drains from the target.
pub const HIT_STAMINA_DRAIN: f32 = 0.4;
/// Ticks between a rival's wind-up pose starting and the hit landing: the
/// telegraph. Long enough that the swing pose (same duration) is clearly
/// visible before damage applies.
pub const WINDUP_TICKS: u32 = 18;
/// Seconds a rival in punch reach of the player holds the player's pace,
/// so a fight stays joined long enough for the player to take part.
pub const PACE_MATCH_SECS: f32 = 3.0;
/// How many times a rider can be dismounted by hits in one race. Past the
/// cap, instability and drains still hurt, but the rider stays in the
/// saddle — a race is not a pile-up.
pub const DISMOUNT_CAP: u32 = 1;
/// Stamina regenerated per second while a rider neither attacks nor is hit.
pub const STAMINA_REGEN: f32 = 0.1;
/// Lateral shove a landed kick delivers, in meters.
pub const KICK_PUSH: f32 = 1.2;
/// Seconds the swing pose shows before the rider settles back into the
/// riding frame.
pub const SWING_SECS: f32 = 0.3;

/// Longitudinal reach of one attack, in meters. Pure.
pub fn reach(attack: Attack) -> f32 {
    match attack {
        Attack::Punch => PUNCH_REACH,
        Attack::Kick => KICK_REACH,
    }
}

/// Whether a rider at longitudinal offset `dx` and lateral offset `dy` from
/// the attacker (positive `dx` ahead, `dy` left) is inside the attack's
/// window. Pure.
pub fn in_reach(attack: Attack, dx: f32, dy: f32) -> bool {
    dx.abs() <= reach(attack) && dy.abs() <= ADJACENT_LATERAL
}

/// Instability one landed hit of `attack` adds. Pure.
pub fn hit_instability(attack: Attack) -> f32 {
    match attack {
        Attack::Punch => PUNCH_HIT,
        Attack::Kick => KICK_HIT,
    }
}

/// Stamina one landed hit of `attack` costs the attacker. Pure.
pub fn stamina_cost(attack: Attack) -> f32 {
    match attack {
        Attack::Punch => PUNCH_COST,
        Attack::Kick => KICK_COST,
    }
}

/// The nearest rider to an attacker among `(slot, pos_x, pos_y)` candidates,
/// by squared road distance; a tie goes to the lower slot so the pick is
/// deterministic. Pure.
pub fn pick_target(
    attacker_x: f32,
    attacker_y: f32,
    candidates: impl Iterator<Item = (usize, f32, f32)>,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (slot, x, y) in candidates {
        let dist = (x - attacker_x).powi(2) + (y - attacker_y).powi(2);
        // Ordered by (distance, slot): the lower slot wins a tie.
        match best {
            Some((best_slot, best_dist)) if (dist, slot) >= (best_dist, best_slot) => {}
            _ => best = Some((slot, dist)),
        }
    }
    best.map(|(slot, _)| slot)
}

/// Signed lateral meters a landed kick shoves the target: away from the
/// attacker — or toward traffic when a car sits at `traffic_y` alongside.
/// Pure.
pub fn kick_push_side(attacker_y: f32, target_y: f32, traffic_y: Option<f32>) -> f32 {
    match traffic_y {
        Some(car_y) => (car_y - target_y).signum(),
        None => (target_y - attacker_y).signum(),
    }
}

/// Seconds between a rival's retaliatory swings: the base cooldown divided
/// by the rival's aggression, divided again by the traffic rage when a car
/// is alongside. Pure.
pub fn retaliation_cooldown(anger: f32, near_traffic: bool) -> f32 {
    let rage = if near_traffic {
        RETALIATION_TRAFFIC_RAGE
    } else {
        1.0
    };
    RETALIATION_COOLDOWN / (anger * rage)
}

/// Whether a rider who just took a hit goes down: instability past the
/// threshold, or any hit landing on a rider already out of stamina. Pure.
pub fn takes_dismount(instability: f32, stamina: f32) -> bool {
    instability >= INSTABILITY_DISMOUNT || stamina <= 0.0
}

/// Advance one rider's combat timers one tick: the swing pose decays, the
/// cooldown counts up toward ready, and stamina regenerates while the rider
/// is clean — no swing thrown, no hit taken this tick.
pub fn step_combat(combat: &mut Combat, attacked: bool, was_hit: bool, dt: f32) {
    combat.cooldown += dt;
    if combat.swing > 0.0 {
        combat.swing = (combat.swing - dt).max(0.0);
        if combat.swing == 0.0 {
            combat.swing_attack = None;
        }
    }
    if !attacked && !was_hit {
        combat.stamina = (combat.stamina + STAMINA_REGEN * dt).min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punch_is_shorter_and_kick_is_longer() {
        const { assert!(PUNCH_REACH < KICK_REACH) };
        assert!(reach(Attack::Punch) == PUNCH_REACH);
        assert!(in_reach(Attack::Punch, PUNCH_REACH, 0.0));
        assert!(!in_reach(Attack::Punch, PUNCH_REACH + 0.01, 0.0));
        assert!(in_reach(Attack::Kick, KICK_REACH, 0.0));
        assert!(!in_reach(Attack::Kick, 0.0, ADJACENT_LATERAL + 0.01));
    }

    #[test]
    fn two_punches_or_three_kicks_cross_the_dismount_threshold() {
        // The spec'd hit counts must survive the instability decay over the
        // swing cooldown: punch-punch with the cooldown between must cross.
        let gap_secs = ATTACK_COOLDOWN;
        let punch_chain = PUNCH_HIT + PUNCH_HIT - INSTABILITY_DECAY * gap_secs;
        assert!(
            punch_chain >= INSTABILITY_DISMOUNT,
            "punch chain: {punch_chain}"
        );
        let kick_chain = KICK_HIT * 3.0 - INSTABILITY_DECAY * 2.0 * ATTACK_COOLDOWN;
        assert!(
            kick_chain >= INSTABILITY_DISMOUNT,
            "kick chain: {kick_chain}"
        );
    }

    #[test]
    fn pick_target_takes_the_nearest_and_breaks_ties_toward_the_lower_slot() {
        let candidates = [(1usize, 10.0, 0.0), (2, 12.0, 0.0), (3, 11.0, 0.5)];
        let picked = pick_target(0.0, 0.0, candidates.iter().copied());
        assert_eq!(picked, Some(1), "nearest along the road wins");

        let tied = [(4usize, 5.0, 0.0), (2, -5.0, 0.0)];
        assert_eq!(pick_target(0.0, 0.0, tied.iter().copied()), Some(2));
    }

    #[test]
    fn pick_target_over_an_empty_road_is_no_one() {
        assert_eq!(pick_target(0.0, 0.0, std::iter::empty()), None);
    }

    #[test]
    fn a_kick_shoves_toward_traffic_and_a_clean_road_shoves_away() {
        // Attacker right of the target, no traffic: the shove is away — left.
        assert_eq!(kick_push_side(2.0, 0.5, None), -1.0);
        // Traffic left of the target wins even though the attacker is right:
        // the shove goes toward the car.
        assert_eq!(kick_push_side(2.0, 0.5, Some(-2.0)), -1.0);
        // Traffic right of the target: shoved right, into the car's lane.
        assert_eq!(kick_push_side(-2.0, 0.5, Some(2.0)), 1.0);
    }

    #[test]
    fn traffic_rage_shortens_the_retaliation_cooldown() {
        let plain = retaliation_cooldown(1.0, false);
        let angry = retaliation_cooldown(1.0, true);
        assert!(
            angry < plain,
            "traffic alongside makes a rival swing sooner"
        );
        // Aggression stacks the same way.
        assert!(retaliation_cooldown(1.4, false) < plain);
        assert_eq!(plain, RETALIATION_COOLDOWN);
    }

    #[test]
    fn a_hit_at_zero_stamina_dismounts_but_a_full_rider_needs_instability() {
        assert!(takes_dismount(INSTABILITY_DISMOUNT, 0.5));
        assert!(
            takes_dismount(0.0, 0.0),
            "a hit on an empty rider is a dismount"
        );
        assert!(!takes_dismount(INSTABILITY_DISMOUNT - 1.0, 0.5));
    }

    #[test]
    fn combat_timers_step_and_stamina_regen_is_clean_only() {
        let mut combat = Combat {
            swing: SWING_SECS,
            ..Combat::default()
        };
        step_combat(&mut combat, false, false, 1.0 / 60.0);
        assert!(
            combat.cooldown > ATTACK_COOLDOWN,
            "cooldown counts toward ready"
        );
        assert!(combat.swing < SWING_SECS, "the pose decays");
        assert!(combat.stamina <= 1.0);

        // Full stamina never overflows.
        let mut full = Combat::default();
        step_combat(&mut full, false, false, 10.0);
        assert_eq!(full.stamina, 1.0);

        // Attacking or being hit this tick blocks the regen.
        let mut spent = Combat {
            stamina: 0.5,
            ..Combat::default()
        };
        step_combat(&mut spent, true, false, 1.0);
        assert_eq!(spent.stamina, 0.5);
        let mut beaten = Combat {
            stamina: 0.5,
            ..Combat::default()
        };
        step_combat(&mut beaten, false, true, 1.0);
        assert_eq!(beaten.stamina, 0.5);
    }

    #[test]
    fn combat_starts_ready_and_fresh() {
        let combat = Combat::default();
        assert_eq!(combat.stamina, 1.0);
        assert!(
            combat.cooldown >= ATTACK_COOLDOWN,
            "the first swing is ready"
        );
        assert_eq!(combat.swing, 0.0);
        assert_eq!(combat.swing_attack, None);
    }

    #[test]
    fn the_swing_pose_carries_its_attack_and_clears_when_done() {
        let mut combat = Combat::default();
        start_swing(&mut combat, Attack::Punch);
        assert_eq!(combat.swing, SWING_SECS);
        assert_eq!(combat.swing_attack, Some(Attack::Punch));
        // Enough ticks to run the pose out.
        for _ in 0..(SWING_SECS * 60.0).ceil() as usize + 1 {
            step_combat(&mut combat, false, false, 1.0 / 60.0);
        }
        assert_eq!(combat.swing, 0.0);
        assert_eq!(combat.swing_attack, None, "the pose clears when it ends");
    }

    #[test]
    fn hit_costs_never_overdraw_the_attacker_past_the_budget() {
        // The player's full swing budget: a full bar throws at least two
        // punches — the fight economics the HUD bar has to make legible.
        let punches = (1.0 / PUNCH_COST).floor();
        assert!(punches >= 2.0, "a full bar throws at least two punches");
    }
}
