//! Persistent ordinary building damage fires. Stock YR Anim ctor/update and
//! Start_Damage_Fires were checked against the owned executable; see the
//! sibling native/building-damage-fire evidence. Smoke and audio are separate.

use crate::{World, combat::LEVEL_HEIGHT};
use serde::{Deserialize, Serialize};

/// The loader enables this only for verified edition/animation metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub enabled: bool,
    pub condition_yellow: f64,
    pub condition_red: f64,
    /// Original DamageFireTypes order, including repeated entries.
    pub types: Vec<FireType>,
}

impl Default for Rules {
    fn default() -> Self { Self { enabled: false, condition_yellow: 0.5, condition_red: 0.25, types: Vec::new() } }
}

/// Resolved ordinary linear animation metadata. The app rejects extra native
/// animation features rather than silently dropping their RNG or side effects.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FireType {
    pub id: String,
    pub start: i32,
    pub loop_start: i32,
    pub loop_end: i32,
    pub end: i32,
    pub rate: i32,
    pub loop_count: i32,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        if !self.condition_yellow.is_finite() || !self.condition_red.is_finite() {
            return Err("building fire thresholds must be finite".into());
        }
        if self.types.len() > u16::MAX as usize + 1 {
            return Err("too many DamageFireTypes for its bound animation index".into());
        }
        for animation in &self.types {
            if animation.id.is_empty() || animation.start != 0 || animation.loop_start < 0
                || animation.loop_start >= animation.loop_end || animation.loop_end > animation.end
                || animation.rate < 0 || animation.loop_count != -1 {
                return Err(format!("{} is outside the verified linear damage-fire animation subset", animation.id));
            }
        }
        Ok(())
    }
}

/// An owned Anim reference, independent of renderer entities and render ticks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageFire {
    /// Index into Rules.types, not the unrelated global Techno registry.
    pub type_index: u16,
    pub coords: (i32, i32, i32),
    pub z_adjust: i32,
    pub frame: i32,
    pub timer_start: i32,
    pub timer_duration: i32,
    pub rate: i32,
    pub increment: i32,
    pub remaining_iterations: u8,
    pub skip_once: bool,
    pub frame_changed: bool,
    pub loop_delay: i32,
}

impl DamageFire {
    /// Native Update has no catch-up loop. A jump in the frame counter advances
    /// one image and restarts its timer at that new counter.
    fn update(&mut self, animation: &FireType, tick: i32) -> bool {
        if self.skip_once { self.skip_once = false; return true; }
        if self.loop_delay != 0 { self.loop_delay = self.loop_delay.wrapping_sub(1); return true; }
        let time_left = if self.timer_start == -1 { self.timer_duration } else {
            let elapsed = tick.wrapping_sub(self.timer_start);
            if elapsed >= self.timer_duration { 0 } else { self.timer_duration.wrapping_sub(elapsed) }
        };
        if time_left != 0 || self.rate == 0 { self.frame_changed = false; return true; }
        self.frame = self.frame.wrapping_add(self.increment);
        self.frame_changed = true;
        self.timer_start = tick;
        self.timer_duration = self.rate;
        let end = if self.remaining_iterations > 1 { animation.loop_end - animation.start } else { animation.end };
        if self.frame >= end {
            if self.remaining_iterations != 0 && self.remaining_iterations != 255 { self.remaining_iterations -= 1; }
            if self.remaining_iterations == 0 { return false; }
            self.frame = animation.loop_start - animation.start;
        }
        true
    }
}

/// Integer emulation of the original chopped float32 input/matrix stores and
/// ftol. The native coefficient408888CE is exactly8947918/2^21, and410888CE
/// is twice that value. Keeping24 significant bits reproduces the original
/// toward-zero stores without depending on the host's floating-point mode.
fn fire_offset((x, y): (i32, i32)) -> (i32, i32) {
    fn input(value: i32) -> i64 {
        let magnitude = (value as i64).unsigned_abs();
        let shift = (64 - magnitude.leading_zeros()).saturating_sub(24);
        let chopped = ((magnitude >> shift) << shift) as i64;
        if value < 0 { -chopped } else { chopped }
    }
    fn axis(value: i64) -> i32 {
        let scaled = value * 8_947_918;
        let magnitude = scaled.unsigned_abs();
        let shift = (64 - magnitude.leading_zeros()).saturating_sub(45);
        let chopped = (((magnitude >> 21) >> shift) << shift) as i64;
        (if scaled < 0 { -chopped } else { chopped }) as i32
    }
    let (x, y) = (input(x), input(y));
    (axis(x + 2 * y), axis(-x + 2 * y))
}

fn fire_z_adjust(y: i32, size: (i32, i32)) -> i32 {
    ((y.wrapping_sub(15i32.wrapping_mul(size.0.wrapping_add(size.1)))).wrapping_mul(3) >> 1).wrapping_sub(10).min(0)
}

impl World {
    /// Resolved SHP frame counts change RNG requests and must be checked when
    /// any snapshot is restored, even when its raw INI files are equal.
    pub fn damage_fire_rules_fingerprint(&self) -> u64 {
        let buildings: Vec<_> = self.building_types.iter().map(|kind|
            (kind.strength, kind.size, kind.can_be_occupied, kind.damage_fire_offsets)).collect();
        bincode::serialize(&(&self.damage_fire_rules, buildings)).expect("serializing building fire rules")
            .iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x100_0000_01b3))
    }

    fn start_building_damage_fires(&mut self, building: usize) {
        let count = self.damage_fire_rules.types.len();
        if count == 0 { return; }
        // This initial draw precedes the first offset/existing-slot test.
        let mut chosen = self.scenario_random_range(0, count as i32 - 1) as usize;
        for slot in 0..8 {
            let owner = &self.buildings[building];
            let kind = &self.building_types[owner.kind as usize];
            let offset = kind.damage_fire_offsets[slot];
            if offset == (0, 0) || owner.damage_fires[slot].is_some() { break; }
            let delta = fire_offset(offset);
            // Stored entry-cell center minus128 is the original fire anchor;
            // aim_point(Building) instead returns its foundation's center.
            let coords = (owner.cell.0.wrapping_mul(crate::LEPTONS_PER_CELL).wrapping_add(delta.0),
                owner.cell.1.wrapping_mul(crate::LEPTONS_PER_CELL).wrapping_add(delta.1),
                self.map.level(owner.cell.0, owner.cell.1) as i32 * LEVEL_HEIGHT);
            let z_adjust = fire_z_adjust(offset.1, kind.size);
            let animation = &self.damage_fire_rules.types[chosen];
            let (rate, end, iterations) = (animation.rate, animation.end, (animation.loop_count as u8).max(1));
            let frame = if end > 0 { self.scenario_random_range(0, end - 1) } else { 0 };
            self.buildings[building].damage_fires[slot] = Some(DamageFire { type_index: chosen as u16, coords, z_adjust,
                frame, timer_start: self.tick as i32, timer_duration: rate, rate, increment: 1,
                remaining_iterations: iterations, skip_once: true, frame_changed: false, loop_delay: 0 });
            chosen = (chosen + 1) % count;
        }
    }

    /// A single building-update phase after repairs, followed by its owned
    /// ordinary Anim updates. Full original cross-object scheduling is tracked
    /// separately; spawning inside each ReceiveDamage would be the wrong gate.
    pub(crate) fn run_building_damage_fires(&mut self) {
        if !self.damage_fire_rules.enabled { return; }
        debug_assert!(self.damage_fire_rules.validate().is_ok());
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            let Some(kind) = self.building_types.get(building.kind as usize) else { continue; };
            let threshold = if kind.can_be_occupied { self.damage_fire_rules.condition_red } else { self.damage_fire_rules.condition_yellow };
            let desired = building.health as f64 / kind.strength as f64 <= threshold;
            if desired != building.damage_fires_active {
                if desired { self.start_building_damage_fires(index); }
                else { self.buildings[index].damage_fires.fill(None); }
                self.buildings[index].damage_fires_active = desired;
            }
            if self.buildings[index].health <= 0 {
                self.buildings[index].damage_fires.fill(None);
                continue;
            }
            for slot in &mut self.buildings[index].damage_fires {
                if let Some(fire) = slot {
                    let keep = self.damage_fire_rules.types.get(fire.type_index as usize).is_some_and(|kind| fire.update(kind, self.tick as i32));
                    if !keep { *slot = None; }
                }
            }
        }
    }

    pub(crate) fn validate_building_damage_fires(&self) -> Result<(), String> {
        self.damage_fire_rules.validate()?;
        for building in &self.buildings {
            if !self.damage_fire_rules.enabled && (building.damage_fires_active || building.damage_fires.iter().any(Option::is_some)) {
                return Err("snapshot has building damage fires without enabled rules".into());
            }
            for fire in building.damage_fires.iter().flatten() {
                let kind = self.damage_fire_rules.types.get(fire.type_index as usize).ok_or("snapshot damage fire has no bound animation type")?;
                if fire.frame < 0 || fire.frame >= kind.end || fire.rate != kind.rate || fire.increment != 1
                    || fire.timer_duration != fire.rate || fire.remaining_iterations == 0 || fire.loop_delay != 0 {
                    return Err("snapshot damage fire has invalid ordinary animation state".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildingType, House, Map};

    fn field(occupied: bool) -> World {
        let mut map = Map::new(80, 100);
        for y in 72..76 { for x in 55..59 { map.set_cell(x, y, true, 4); } }
        let mut world = World::new(map, None);
        world.houses = vec![House::default(), House { human: true, ..House::default() }];
        let mut offsets = [(0, 0); 8];
        offsets[0] = (72, 15);
        offsets[1] = (-44, 71);
        offsets[3] = (2, 3); // The sentinel before this ends the whole method.
        world.building_types.push(BuildingType { size: (4, 4), strength: 1000, can_be_occupied: occupied,
            damage_fire_offsets: offsets, ..BuildingType::default() });
        world.add_map_building(1, 0, (55, 72), 256);
        world.initialize_campaign_random(0x1234_5678);
        world.damage_fire_rules = Rules { enabled: true, types: [("FIRE01", 30), ("FIRE02", 64), ("FIRE03", 30)]
            .into_iter().map(|(id, end)| FireType { id: id.into(), start: 0, loop_start: 0, loop_end: end,
                end, rate: 2, loop_count: -1 }).collect(), ..Rules::default() };
        world
    }

    #[test]
    fn threshold_transitions_preserve_original_type_frame_draws_anchors_and_owned_slots() {
        // Eight original6D2070→5AFB80→ftol CPU observations, including input
        // float32 truncation, large values and returned low32-bit wrap.
        for (art, expected) in [((72, 15), (435, -179)), ((-44, 71), (418, 793)), ((15, 0), (64, -64)),
            ((30180, 0), (128768, -128768)), ((-30180, 0), (-128768, 128768)),
            ((16777217, 1), (71583352, -71583328)), ((i32::MIN, i32::MAX), (572731392, 1718198272)),
            ((i32::MAX, i32::MIN), (-572733440, -1718198272))] {
            assert_eq!(fire_offset(art), expected);
        }
        for (occupied, threshold) in [(false, 500), (true, 250)] {
            let mut world = field(occupied);
            let mut expected = world.campaign_random_state().unwrap().clone();
            world.buildings[0].health = threshold + 1;
            world.run_building_damage_fires();
            assert!(!world.buildings[0].damage_fires_active);
            assert_eq!(world.campaign_random_state(), Some(&expected));
            world.tick = 15;
            world.buildings[0].health = threshold;
            let chosen = expected.ranged(0, 2) as usize;
            let frames = [expected.ranged(0, world.damage_fire_rules.types[chosen].end - 1),
                expected.ranged(0, world.damage_fire_rules.types[(chosen + 1) % 3].end - 1)];
            world.run_building_damage_fires();
            let building = &world.buildings[0];
            assert!(building.damage_fires_active);
            for (slot, coords, z_adjust) in [(0, (14515, 18253, 416), -168), (1, (14498, 19225, 416), -84)] {
                let fire = building.damage_fires[slot].as_ref().unwrap();
                assert_eq!((fire.type_index as usize, fire.frame), ((chosen + slot) % 3, frames[slot]));
                assert_eq!((fire.coords, fire.z_adjust), (coords, z_adjust), "original StartDamageFires native fixture");
                assert_eq!((fire.timer_start, fire.timer_duration, fire.remaining_iterations, fire.skip_once), (15, 2, 255, false));
            }
            assert!(building.damage_fires[2..].iter().all(Option::is_none));
            assert_eq!(world.campaign_random_state(), Some(&expected));
            world.buildings[0].health = threshold - 1;
            world.buildings[0].damage_fires[0] = None; // Original expired-pointer callback.
            world.tick = 17;
            world.run_building_damage_fires();
            assert!(world.buildings[0].damage_fires[0].is_none(), "an expired fire is not replenished while its latch stays true");
            assert_eq!(world.campaign_random_state(), Some(&expected));
            world.buildings[0].health = threshold + 1;
            world.run_building_damage_fires();
            assert!(!world.buildings[0].damage_fires_active);
            assert!(world.buildings[0].damage_fires.iter().all(Option::is_none));
            assert_eq!(world.campaign_random_state(), Some(&expected));
            world.tick = 19;
            world.buildings[0].health = threshold;
            world.run_building_damage_fires();
            assert!(world.buildings[0].damage_fires[0].is_some());
            assert_ne!(world.campaign_random_state(), Some(&expected), "a new threshold transition constructs a new pair");
        }
        for empty_list in [false, true] {
            let mut world = field(false);
            world.buildings[0].health = 500;
            if empty_list { world.damage_fire_rules.types.clear(); }
            else { world.building_types[0].damage_fire_offsets[0] = (0, 0); }
            let mut expected = world.campaign_random_state().unwrap().clone();
            if !empty_list { expected.ranged(0, 2); }
            world.run_building_damage_fires();
            assert!(world.buildings[0].damage_fires_active);
            assert!(world.buildings[0].damage_fires.iter().all(Option::is_none));
            assert_eq!(world.campaign_random_state(), Some(&expected));
        }
    }

    #[test]
    fn stock_animation_matches_original_skip_timer_loop_and_jump_witnesses() {
        // stock-fire-anim-native-verification.json, original ctor/Update/Stage
        // with actual stock SHPs and coherent original-process type metadata.
        for (end, initial, wrap, last, expected_last) in [(30, 13, 134, 164, 15), (64, 45, 138, 232, 47)] {
            let kind = FireType { end, loop_end: end, rate: 2, loop_count: -1, ..FireType::default() };
            let mut fire = DamageFire { type_index: 0, coords: (14515, 18253, 416), z_adjust: -168,
                frame: initial, timer_start: 100, timer_duration: 2, rate: 2, increment: 1,
                remaining_iterations: 255, skip_once: true, frame_changed: false, loop_delay: 0 };
            for tick in 100..=last {
                assert!(fire.update(&kind, tick));
                if tick == 100 || tick == 101 { assert_eq!(fire.frame, initial); }
                if tick == 102 { assert_eq!(fire.frame, initial + 1); }
                if tick == wrap { assert_eq!((fire.frame, fire.timer_start), (0, wrap)); }
            }
            assert_eq!(fire.frame, expected_last);
            for (tick, frame) in [(last + 10, expected_last + 1), (last + 10, expected_last + 1),
                (last + 11, expected_last + 1), (last + 12, expected_last + 2)] {
                assert!(fire.update(&kind, tick));
                assert_eq!(fire.frame, frame);
                assert_eq!(fire.remaining_iterations, 255);
            }
        }
    }

    #[test]
    fn save_restores_owned_fire_phase_rng_and_rule_fingerprint_across_a_loop_and_repair() {
        let mut world = field(false);
        world.attach_mission(crate::mission::MissionData { id: "owned-damage-fire".into(), player_house: 1, ..Default::default() }).unwrap();
        world.buildings[0].health = 501;
        let before = world.snapshot();
        let mut before_restored = field(false);
        before_restored.attach_mission(world.mission_data.as_ref().unwrap().clone()).unwrap();
        before_restored.restore(&before).unwrap();
        world.buildings[0].health = 500;
        before_restored.buildings[0].health = 500;
        world.tick(); before_restored.tick();
        assert_eq!(world.snapshot(), before_restored.snapshot(), "restoring before creation must not redraw the fire choices");
        for _ in 0..6 { world.tick(); }
        let saved = world.snapshot();
        let mut restored = field(false);
        restored.attach_mission(world.mission_data.as_ref().unwrap().clone()).unwrap();
        restored.restore(&saved).unwrap();
        assert_eq!(world.snapshot(), restored.snapshot());
        for _ in 0..70 {
            world.tick(); restored.tick();
            assert_eq!(world.snapshot(), restored.snapshot());
        }
        world.buildings[0].health = 501;
        restored.buildings[0].health = 501;
        world.tick(); restored.tick();
        assert_eq!(world.snapshot(), restored.snapshot());
        assert!(restored.buildings[0].damage_fires.iter().all(Option::is_none));
        let mut different = field(false);
        different.damage_fire_rules.types[0].end += 1;
        different.attach_mission(world.mission_data.as_ref().unwrap().clone()).unwrap();
        assert!(different.restore(&saved).is_err(), "resolved SHP metadata belongs in the restore fingerprint");
        // Skirmishes have no mission rules hash. The snapshot's own named
        // section must still reject changed SHP metadata before state writes.
        let mut skirmish = field(false);
        assert!(skirmish.mission_data.is_none());
        skirmish.buildings[0].health = 500;
        skirmish.tick();
        let saved_skirmish = skirmish.snapshot();
        let mut skirmish_restored = field(false);
        skirmish_restored.restore(&saved_skirmish).unwrap();
        assert_eq!(skirmish_restored.snapshot(), saved_skirmish);
        skirmish_restored.damage_fire_rules.types[0].end += 1;
        let unchanged = skirmish_restored.snapshot();
        assert!(skirmish_restored.restore(&saved_skirmish).unwrap_err().contains("damage fire rules differ"));
        assert_eq!(skirmish_restored.snapshot(), unchanged, "rules rejection must precede every state write");
        let mut old = saved.clone(); old[..4].copy_from_slice(&19u32.to_le_bytes());
        assert!(restored.restore(&old).unwrap_err().contains("format 19"));
    }

    #[test]
    fn disabled_rules_keep_legacy_rng_and_death_sale_and_packing_release_every_fire() {
        let mut disabled = field(false);
        disabled.damage_fire_rules.enabled = false;
        disabled.damage_fire_rules.types[0].loop_count = 123; // Retained unsupported metadata is safe while disabled.
        disabled.buildings[0].health = 100;
        let random = (disabled.random, disabled.campaign_random.clone());
        disabled.run_building_damage_fires();
        assert!(!disabled.buildings[0].damage_fires_active);
        assert_eq!((disabled.random, disabled.campaign_random.clone()), random);
        assert!(disabled.restore(&disabled.snapshot()).is_ok());
        let mut legacy = field(false);
        legacy.campaign_random = None;
        legacy.damage_fire_rules.types.truncate(1);
        legacy.buildings[0].health = 500;
        let mut expected_legacy = field(false);
        expected_legacy.campaign_random = None;
        expected_legacy.scenario_random_range(0, 0);
        expected_legacy.scenario_random_range(0, 29);
        expected_legacy.scenario_random_range(0, 29);
        legacy.run_building_damage_fires();
        assert_eq!(legacy.random, expected_legacy.random, "non-campaign callers keep the existing range fallback including equal-endpoint consumption");
        for removal in ["death", "sale", "pack"] {
            let mut world = field(false);
            world.buildings[0].health = 500;
            world.run_building_damage_fires();
            assert!(world.buildings[0].damage_fires[0].is_some());
            let id = world.buildings[0].id;
            match removal {
                "death" => {
                    world.warheads.push(crate::combat::Warhead::default());
                    assert!(world.receive_damage(crate::combat::Aim::Building(id), 1000, 0, 0, None));
                    world.run_building_damage_fires();
                    assert!(world.buildings[0].damage_fires.iter().all(Option::is_none));
                    let removed = world.demolish(0);
                    assert!(removed.damage_fires.iter().all(Option::is_none));
                }
                "sale" => world.finish_sale(id),
                "pack" => world.finish_packing(id, (1, 1)),
                _ => unreachable!(),
            }
            assert!(world.buildings.is_empty());
        }
    }
}
