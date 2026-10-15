//! The jumpjet locomotor: how Rocketeers, Nighthawks and Kirovs fly.
//!
//! Flight matches the original's: a straight line with no pathfinder.

use super::*;

/// A type's jumpjet figures. Fractions are kept in thousandths.
// The keys and the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JetStats {
    pub turn_rate: i32,
    /// Leptons a tick.
    pub speed: i32,
    pub climb_milli: i32,
    pub crash_milli: i32,
    pub height: i32,
    pub accel_milli: i32,
    pub wobbles_milli: i32,
    pub no_wobbles: bool,
    pub deviation: i32,
    pub balloon_hover: bool,
    pub hover_attack: bool,
    pub crashable: bool,
}

impl Default for JetStats {
    fn default() -> Self {
        Self {
            turn_rate: 4,
            speed: 14,
            climb_milli: 5000,
            crash_milli: 5000,
            height: 500,
            accel_milli: 2000,
            wobbles_milli: 150,
            no_wobbles: false,
            deviation: 40,
            balloon_hover: false,
            hover_attack: false,
            crashable: false,
        }
    }
}

pub const GROUNDED: u8 = 0;
pub const ASCENDING: u8 = 1;
pub const HOVERING: u8 = 2;
pub const CRUISING: u8 = 3;
pub const DESCENDING: u8 = 4;
pub const CRASHING: u8 = 5;
pub const CRASHED: u8 = 6;

/// A flying unit's state.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Jet {
    pub state: u8,
    /// Where it is flying to, in leptons.
    dest: Option<(i32, i32)>,
    moving: bool,
    /// The way it is flying, which the body follows.
    heading: Turn,
    speed: f64,
    target_speed: f64,
    target_height: i32,
    phase: f64,
    /// Whether its landing place is already taken for it.
    spot_marked: bool,
    /// Its height in leptons, from the same zero as the ground's.
    pub z: i32,
    /// The cell it holds while it hovers.
    pub holds: Option<(i32, i32)>,
    /// Killed in the air and falling; and down.
    pub crashing: bool,
    pub crashed: bool,
}

impl Jet {
    pub(crate) fn on_ground(z: i32) -> Self {
        Self { z, ..Self::default() }
    }

    /// Sets where it is to fly to.
    // Ordered away while landing, it climbs again.
    pub(crate) fn fly_to(&mut self, point: (i32, i32), height: i32) {
        self.dest = Some(point);
        self.moving = true;
        if self.state == DESCENDING {
            self.state = ASCENDING;
            self.target_height = height;
        }
    }
}

fn floor(map: &Map, x: i32, y: i32) -> i32 {
    map.level(x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL)) as i32 * combat::LEVEL_HEIGHT
}

/// The direction from one point to another as a 16-bit facing.
// East when the points are the same.
fn direction(from: (i32, i32), to: (i32, i32)) -> u16 {
    if from == to {
        return 0x4000;
    }
    ((((from.1 - to.1) as f64).atan2((to.0 - from.0) as f64) - std::f64::consts::FRAC_PI_2) * -10430.06004) as i32 as u16
}

fn cell_of(point: (i32, i32)) -> (i32, i32) {
    (point.0.div_euclid(LEPTONS_PER_CELL), point.1.div_euclid(LEPTONS_PER_CELL))
}

/// The height the flyer keeps clear of: the ground, a building or a unit under
/// it, evened out with the cell ahead.
fn clearance(unit: &Unit, jet: &Jet, ground: &Ground) -> i32 {
    let over = |cell: (i32, i32)| -> i32 {
        let (cx, cy) = cell_centre(cell);
        let mut h = floor(ground.map, cx, cy);
        let built = ground.buildings.iter().any(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1));
        let held = ground.map.index(cell.0, cell.1).is_some_and(|i| ground.claims[i] != 0 || ground.spots[i] & SPOT_BITS != 0);
        if built {
            h += 4 * combat::LEVEL_HEIGHT;
        } else if held {
            h += 85;
        }
        h
    };
    let h = over(unit.cell());
    if jet.speed > 0.0 {
        let d = ((((jet.heading.current() >> 12) + 1) >> 1) & 7) as usize;
        let h2 = over(step(unit.cell(), d));
        return if h2 > h { h2 } else { (h2 + h) / 2 };
    }
    h
}

fn held_by_other(unit: &Unit, ground: &Ground) -> bool {
    let cell = unit.cell();
    ground.others.iter().any(|other| other.id != unit.id && other.holds == Some(cell))
}

fn release_ground(unit: &Unit, ground: &mut Ground) {
    if unit.stats.infantry {
        ground.unmark_spot(unit.x, unit.y);
    } else {
        ground.release(unit.cell(), unit.id);
    }
}

fn take_ground(unit: &Unit, point: (i32, i32), ground: &mut Ground) {
    if unit.stats.infantry {
        ground.mark_spot(point.0, point.1, unit.owner);
    } else {
        ground.hold(cell_of(point), unit);
    }
}

/// A place to come down near a cell: the point, or `None` if there is none.
// A free cell near the one wanted, and for a foot
// soldier a free spot in it.
pub(crate) fn landing_point(map: &Map, claims: &[u32], spots: &[u8], unit: &Unit, wanted: (i32, i32), tick: u64) -> Option<(i32, i32)> {
    let cell = nearby(map, claims, spots, unit, wanted, crate::Near::new(false, true, Some(wanted)), tick)?;
    if !unit.stats.infantry {
        return Some(cell_centre(cell));
    }
    let index = map.index(cell.0, cell.1)?;
    let spot = (2..=4usize).find(|spot| spots[index] & (1 << spot) == 0)?;
    Some((cell.0 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].0, cell.1 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].1))
}

/// One tick of a flyer.
pub(crate) fn fly(unit: &mut Unit, ground: &mut Ground) {
    let (Some(stats), Some(mut jet)) = (unit.stats.jet, unit.jet) else {
        return;
    };
    if jet.moving || !matches!(jet.state, GROUNDED | HOVERING) || jet.crashing {
        movement(unit, &mut jet, &stats, ground);
        let height = jet.z - floor(ground.map, unit.x, unit.y);
        if jet.crashing && !matches!(jet.state, CRASHING | CRASHED) && height > 0 {
            jet.target_height = -5;
            jet.state = CRASHING;
        }
        handle_state(unit, &mut jet, &stats, ground);
    }
    unit.jet = Some(jet);
}

/// The physics of a tick, whatever the state.
fn movement(unit: &mut Unit, jet: &mut Jet, stats: &JetStats, ground: &mut Ground) {
    let (climb, accel) = (stats.climb_milli as f64 / 1000.0, stats.accel_milli as f64 / 1000.0);
    if jet.target_speed > jet.speed {
        jet.speed = (jet.speed + accel).min(stats.speed as f64);
    }
    // Slowing, at one and a half times the gain,
    // stops only at nothing; with the built-in gain of 2 a flyer still closes on
    // its destination.
    if jet.target_speed < jet.speed {
        jet.speed = (jet.speed - accel * 1.5).max(0.0);
    }
    unit.fraction = (jet.speed / stats.speed.max(1) as f64 * UNIT_FRACTION as f64) as i32;
    let dest_cell = jet.dest.map_or(unit.cell(), cell_of);
    let at_dest = unit.cell() == dest_cell;

    if matches!(jet.state, HOVERING | CRUISING) && !stats.no_wobbles {
        jet.phase += std::f64::consts::TAU * (stats.wobbles_milli as f64 / 1000.0) / 15.0;
    } else {
        jet.phase = 0.0;
    }
    let want = (jet.phase.sin() * stats.deviation as f64 + jet.target_height as f64) as i32;

    let mut z = jet.z;
    let ground_z = floor(ground.map, unit.x, unit.y);
    let cur = if jet.state != DESCENDING && jet.state != GROUNDED && !(at_dest && !stats.balloon_hover) {
        z - clearance(unit, jet, ground)
    } else {
        z - ground_z
    };
    if cur < want {
        // Lift-off frees the ground it stood on.
        if z - ground_z == 0 && !jet.spot_marked {
            release_ground(unit, ground);
        }
        z = if cur as f64 + climb > want as f64 { z + (want - cur) } else { (z as f64 + climb) as i32 };
    } else if cur > want {
        z = if want as f64 > cur as f64 - climb { z + (want - cur) } else { (z as f64 - climb) as i32 };
        if z <= ground_z {
            z = ground_z;
        }
    }
    if unit.cell() != dest_cell && cur < want / 2 {
        jet.speed = 0.0;
    }
    jet.z = z;

    // The step along its own heading.
    let v = jet.speed as i32 as f64;
    let angle = (jet.heading.current() as i16 as f64 - 16383.0) * -(std::f64::consts::PI / 32768.0);
    let (x, y) = ((angle.cos() * v + unit.x as f64) as i32, (unit.y as f64 - angle.sin() * v) as i32);
    if ground.map.index(x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL)).is_some() {
        unit.x = x;
        unit.y = y;
    }
    unit.facing = (jet.heading.current() >> 8) as u8;
    unit.turn = Turn::facing(unit.facing);
    if jet.heading.remaining > 0 {
        jet.heading.remaining -= 1;
    }
}

fn aim(jet: &mut Jet, want: u16, rate: i32) {
    if jet.heading.desired != want {
        jet.heading = Turn::start(jet.heading.current(), want, rate);
    }
}

/// A neighbouring cell picked at random to move over to.
fn sidestep(unit: &Unit, jet: &mut Jet, ground: &mut Ground) {
    let cell = step(unit.cell(), (random_step(ground.random) % 8) as usize);
    jet.dest = Some(cell_centre(cell));
    jet.moving = true;
}

fn handle_state(unit: &mut Unit, jet: &mut Jet, stats: &JetStats, ground: &mut Ground) {
    let here = (unit.x, unit.y);
    let height = jet.z - floor(ground.map, unit.x, unit.y);
    let has_target = unit.arms.target.is_some();
    match jet.state {
        GROUNDED => {
            if !jet.moving {
                return;
            }
            jet.heading = Turn::facing(unit.facing);
            jet.speed = 0.0;
            jet.target_speed = 0.0;
            jet.target_height = stats.height.max(2 * combat::LEVEL_HEIGHT);
            jet.state = ASCENDING;
        }
        ASCENDING => {
            if height >= jet.target_height {
                if held_by_other(unit, ground) {
                    sidestep(unit, jet, ground);
                    jet.state = CRUISING;
                } else {
                    jet.holds = Some(unit.cell());
                    jet.state = HOVERING;
                }
                return;
            }
            let wait = if stats.balloon_hover && !unit.stats.infantry { jet.target_height / 2 } else { jet.target_height / 4 };
            if height <= wait {
                return;
            }
            jet.target_speed = stats.speed as f64;
            let dest = jet.dest.unwrap_or(here);
            aim(jet, direction(here, dest), stats.turn_rate);
            if unit.cell() == cell_of(dest) && !stats.balloon_hover {
                jet.target_height = height;
                jet.state = CRUISING;
            }
        }
        HOVERING => {
            if !jet.moving {
                return;
            }
            let dest = jet.dest.unwrap_or(here);
            if dest == here {
                if has_target {
                    return;
                }
                if !stats.balloon_hover {
                    jet.state = DESCENDING;
                }
                return;
            }
            aim(jet, direction(here, dest), stats.turn_rate);
            jet.holds = None;
            jet.state = CRUISING;
        }
        CRUISING => {
            let dest = jet.dest.unwrap_or(here);
            let want = direction(here, dest);
            aim(jet, want, stats.turn_rate);
            // An unsigned byte, so that any turn to the left counts as a big one.
            let off = ((want.wrapping_sub(jet.heading.current()).wrapping_add(0x80)) >> 8) as i32 & 0xff;
            let distance = (((dest.0 - here.0) as f64).hypot((dest.1 - here.1) as f64)) as i32;
            let (s, full) = (stats.speed, stats.height.max(2 * combat::LEVEL_HEIGHT));
            if distance < 20 {
                jet.speed = 0.0;
                jet.target_speed = 0.0;
                unit.x = dest.0;
                unit.y = dest.1;
                if !has_target && !stats.balloon_hover {
                    jet.target_height = 0;
                    jet.state = DESCENDING;
                } else if held_by_other(unit, ground) {
                    sidestep(unit, jet, ground);
                } else {
                    jet.holds = Some(unit.cell());
                    jet.state = HOVERING;
                    unit.dest = None;
                }
            } else if distance < s {
                jet.target_speed = (s / 8) as f64;
                if !has_target {
                    jet.target_height = full / 2;
                }
            } else if distance < 2 * s {
                jet.target_speed = (s / 4) as f64;
                if !has_target {
                    jet.target_height = full / 2;
                }
                if off > stats.turn_rate {
                    jet.target_speed = ((s / 10) as f64).max(1.0);
                }
            } else if distance < s * 50 / stats.turn_rate.max(1) {
                jet.target_speed = (s / 2) as f64;
                if !has_target {
                    jet.target_height = (full as f64 * 0.75) as i32;
                }
                if off > 5 * stats.turn_rate {
                    jet.target_speed = ((s / 5) as f64).max(1.0);
                }
            } else {
                jet.target_speed = s as f64;
                jet.target_height = full;
            }
            let cell = cell_of(dest);
            let wet = ground.map.index(cell.0, cell.1).is_some_and(|i| ground.map.land[i] == Land::Water as u8 || ground.map.land[i] == Land::Beach as u8);
            if stats.balloon_hover || wet {
                jet.target_height = full;
            }
        }
        DESCENDING => {
            let dest = jet.dest.unwrap_or(here);
            let cell = cell_of(dest);
            let wet = ground.map.index(cell.0, cell.1).is_some_and(|i| ground.map.land[i] == Land::Water as u8 || ground.map.land[i] == Land::Beach as u8);
            if wet {
                if held_by_other(unit, ground) {
                    sidestep(unit, jet, ground);
                    jet.state = CRUISING;
                } else {
                    jet.holds = Some(unit.cell());
                    jet.state = HOVERING;
                    jet.target_height = stats.height.max(2 * combat::LEVEL_HEIGHT);
                }
                return;
            }
            if !jet.spot_marked {
                let code = ground.can_enter(unit, cell, None, cell);
                let room = if unit.stats.infantry {
                    ground.map.index(cell.0, cell.1).is_some_and(|i| ground.spots[i] & (1 << spot_index(dest.0, dest.1)) == 0)
                } else {
                    true
                };
                if !room || code >= code::MOVING_BLOCK {
                    // Somewhere else to come down, or none at all.
                    match landing_point(ground.map, ground.claims, ground.spots, unit, unit.cell(), ground.now) {
                        Some(point) => {
                            jet.dest = Some(point);
                            jet.state = ASCENDING;
                            jet.target_height = stats.height.max(2 * combat::LEVEL_HEIGHT);
                        }
                        None => jet.crashing = true,
                    }
                    return;
                }
                jet.spot_marked = true;
                take_ground(unit, dest, ground);
            }
            jet.target_height = 0;
            if height != 0 {
                return;
            }
            unit.x = dest.0;
            unit.y = dest.1;
            unit.fraction = 0;
            unit.dest = None;
            jet.dest = None;
            jet.moving = false;
            jet.holds = None;
            jet.spot_marked = false;
            jet.state = GROUNDED;
        }
        CRASHING => {
            jet.z = (jet.z as f64 - stats.crash_milli as f64 / 1000.0) as i32;
            let spin = jet.heading.current().wrapping_add(0x2d00);
            aim(jet, spin, stats.turn_rate);
            if jet.z - floor(ground.map, unit.x, unit.y) <= 0 {
                jet.holds = None;
                jet.target_speed = 0.0;
                jet.state = CRASHED;
                jet.crashed = true;
            }
        }
        _ => {}
    }
}
