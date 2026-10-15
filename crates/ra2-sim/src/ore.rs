//! Ore growing denser, spreading to the cells beside it, and coming out of ore
//! drills.
//!
//! Follows the original game (1 and 2) and

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::path::DIRECTIONS;
use crate::World;

/// A kind of ore, from its section of the rules (`[Riparius]`, `[Cruentus]`).
#[derive(Clone, Copy, Debug)]
pub struct OreType {
    /// `Growth`, `Spread`: frames between passes.
    pub growth: u32,
    pub spread: u32,
    /// `GrowthPercentage`, `SpreadPercentage`: the share of the queued cells a
    /// pass may deal with.
    pub growth_percentage: f64,
    pub spread_percentage: f64,
}

/// One kind's two queues: the cells waiting to grow and to spread, the
/// earliest priority first, and whether each cell is in each.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Queues {
    growth: BinaryHeap<Reverse<(u64, u64, (i32, i32))>>,
    spread: BinaryHeap<Reverse<(u64, u64, (i32, i32))>>,
    spread_queued: Vec<bool>,
    /// Counts entries, to keep those of equal priority in order.
    serial: u64,
    /// Entries ever put in each queue since it was last rebuilt (the
    /// engine's node arrays, which only grow).
    growth_nodes: usize,
    spread_nodes: usize,
    growth_at: u64,
    spread_at: u64,
}

impl World {
    /// The kinds of ore, in the order of the rules' `[Tiberiums]`.
    pub fn set_ore_types(&mut self, types: Vec<OreType>) {
        self.ore_queues = vec![Queues::default(); types.len()];
        self.ore_types = types;
        self.ore_ready = false;
    }

    fn ore_kind_at(&self, cell: (i32, i32)) -> Option<usize> {
        self.map.ore_at(cell.0, cell.1).map(|(kind, _)| kind as usize)
    }

    // Flat ground, less than full, and a kind that
    // grows at all.
    fn ore_can_grow(&self, cell: (i32, i32)) -> bool {
        self.map.ore_at(cell.0, cell.1).is_some_and(|(kind, amount)| {
            amount < 11 && self.map.ramp(cell.0, cell.1) == 0 && self.ore_types.get(kind as usize).is_some_and(|kind| kind.growth_percentage >= 1e-5)
        })
    }

    // More on it than half its kind's number (so
    // at least two bales for both stock kinds), flat ground, a kind that
    // spreads at all, and nothing standing on the cell.
    fn ore_can_spread(&self, cell: (i32, i32)) -> bool {
        self.map.ore_at(cell.0, cell.1).is_some_and(|(kind, amount)| {
            amount as usize > kind as usize / 2
                && amount >= 1
                && self.map.ramp(cell.0, cell.1) == 0
                && self.ore_types.get(kind as usize).is_some_and(|kind| kind.spread_percentage >= 1e-5)
                && !self.cell_has_object(cell)
        })
    }

    fn cell_has_object(&self, cell: (i32, i32)) -> bool {
        self.building_on(cell).is_some() || self.units.iter().any(|unit| unit.cell() == cell && unit.health > 0 && self.altitude(unit) == 0)
    }

    // Inside the playfield, no building, no ore
    // drill, land that can be built on, no overlay, flat, and a tile whose
    // set allows ore.
    fn ore_can_accept(&self, cell: (i32, i32)) -> bool {
        let map = &self.map;
        map.in_playfield(cell.0, cell.1)
            && map.ore_ground(cell.0, cell.1)
            && map.ramp(cell.0, cell.1) == 0
            && !map.has_overlay(cell.0, cell.1)
            && map.land_buildable(cell.0, cell.1)
            && self.building_on(cell).is_none()
            && !self.animated.iter().any(|object| object.cell == cell)
    }

    /// Empties a kind's growth queue and puts back every cell of it that can
    /// grow, all due at once.
    // In the map's cell order, with no random numbers. This is what brings
    // mined ore back: a full cell that is mined is not queued again, so
    // until the next rebuild it does not grow.
    fn rebuild_growth(&mut self, kind: usize) {
        let cells: Vec<(i32, i32)> = self.map.ore_cells().filter(|&(_, k, _)| k as usize == kind).map(|(cell, _, _)| cell).collect();
        let cells: Vec<(i32, i32)> = cells.into_iter().filter(|&cell| self.ore_can_grow(cell)).collect();
        let Some(queues) = self.ore_queues.get_mut(kind) else {
            return;
        };
        queues.growth.clear();
        queues.growth_nodes = 0;
        for cell in cells {
            queues.serial += 1;
            queues.growth.push(Reverse((0, queues.serial, cell)));
            queues.growth_nodes += 1;
        }
    }

    // The same for spreading.
    fn rebuild_spread(&mut self, kind: usize) {
        let cells: Vec<(i32, i32)> = self.map.ore_cells().filter(|&(_, k, _)| k as usize == kind).map(|(cell, _, _)| cell).collect();
        let cells: Vec<(i32, i32)> = cells.into_iter().filter(|&cell| self.ore_can_spread(cell)).collect();
        let indices: Vec<usize> = cells.iter().map(|cell| self.map.index(cell.0, cell.1).unwrap_or(0)).collect();
        let Some(queues) = self.ore_queues.get_mut(kind) else {
            return;
        };
        queues.spread.clear();
        queues.spread_nodes = 0;
        queues.spread_queued.iter_mut().for_each(|queued| *queued = false);
        for (cell, index) in cells.into_iter().zip(indices) {
            queues.serial += 1;
            queues.spread.push(Reverse((0, queues.serial, cell)));
            queues.spread_nodes += 1;
            if let Some(queued) = queues.spread_queued.get_mut(index) {
                *queued = true;
            }
        }
    }

    /// How many entries a queue's node array holds.
    // `(Size height + 4) x Size width x 2`.
    fn ore_capacity(&self) -> usize {
        self.map.cell_array_size()
    }

    // Only a cell less than full (whether it is
    // queued already is not asked); due at this frame plus up to 49. A node
    // array within ten of full is rebuilt first.
    fn queue_growth(&mut self, kind: usize, cell: (i32, i32)) {
        if !self.map.ore_at(cell.0, cell.1).is_some_and(|(_, amount)| amount < 11) {
            return;
        }
        if self.ore_queues.get(kind).is_some_and(|queues| queues.growth_nodes + 10 > self.ore_capacity()) {
            self.rebuild_growth(kind);
        }
        let due = self.tick + (self.next_random() % 50) as u64;
        if let Some(queues) = self.ore_queues.get_mut(kind) {
            queues.serial += 1;
            queues.growth.push(Reverse((due, queues.serial, cell)));
            queues.growth_nodes += 1;
        }
    }

    // Only a cell that can spread and is not
    // queued already.
    fn queue_spread(&mut self, kind: usize, cell: (i32, i32)) {
        let Some(index) = self.map.index(cell.0, cell.1) else {
            return;
        };
        if !self.ore_can_spread(cell) || self.ore_queues.get(kind).is_none_or(|queues| queues.spread_queued[index]) {
            return;
        }
        // A node array within twenty of full is
        // rebuilt first.
        if self.ore_queues[kind].spread_nodes + 20 >= self.ore_capacity() {
            self.rebuild_spread(kind);
        }
        let due = self.tick + (self.next_random() % 50) as u64;
        let queues = &mut self.ore_queues[kind];
        queues.serial += 1;
        queues.spread.push(Reverse((due, queues.serial, cell)));
        queues.spread_nodes += 1;
        queues.spread_queued[index] = true;
    }

    /// Puts ore on a cell or adds to what is there. Whether it did.
    // On a cell that can take new ore, one of the
    // kind's twelve pictures at random, queued to grow; on a cell of the same
    // kind, that much more up to full, queued to spread.
    fn place_ore(&mut self, cell: (i32, i32), kind: usize, amount: u8) -> bool {
        if amount >= 12 {
            return false;
        }
        if self.ore_can_accept(cell) {
            let picture = (self.next_random() % 12) as u8;
            self.map.put_ore(cell.0, cell.1, kind as u8, amount);
            self.map.set_ore_picture(cell.0, cell.1, picture);
            self.queue_growth(kind, cell);
            return true;
        }
        if self.ore_kind_at(cell) != Some(kind) || !self.ore_can_grow(cell) {
            return false;
        }
        let held = self.map.ore_at(cell.0, cell.1).map_or(0, |(_, amount)| amount);
        self.map.put_ore(cell.0, cell.1, kind as u8, (held + amount).min(11));
        self.queue_spread(kind, cell);
        true
    }

    /// Ore from a cell goes onto one of the eight beside it. `forced` is the
    /// ore drill, which needs no ore of its own.
    // From a direction at random, the first
    // neighbour that can take it gets four bales.
    pub(crate) fn spread_ore_from(&mut self, cell: (i32, i32), forced: bool) -> bool {
        if !forced && !self.ore_can_spread(cell) {
            return false;
        }
        let kind = match self.ore_kind_at(cell) {
            Some(kind) => kind,
            None if forced => 0,
            None => return false,
        };
        let start = (self.next_random() % 8) as usize;
        for i in 0..8 {
            let step = DIRECTIONS[(i + start) & 7];
            let next = (cell.0 + step.0, cell.1 + step.1);
            if self.ore_can_accept(next) {
                return self.place_ore(next, kind, 3);
            }
        }
        false
    }

    /// A cell has just been mined out.
    // It leaves every kind's spread queue, and
    // its neighbours that are not queued are asked to spread.
    pub(crate) fn ore_emptied(&mut self, cell: (i32, i32), kind: usize) {
        if let Some(index) = self.map.index(cell.0, cell.1) {
            for queues in &mut self.ore_queues {
                if let Some(queued) = queues.spread_queued.get_mut(index) {
                    *queued = false;
                }
            }
        }
        for step in DIRECTIONS {
            self.queue_spread(kind, (cell.0 + step.0, cell.1 + step.1));
        }
    }

    /// The growth and spread passes.
    // Each kind has
    // a pass of each sort when its timer runs out, both on the first frame;
    // growth every `Growth` x 0.3 frames (a skirmish sets the flag that
    // shortens it), spread every `Spread`.
    pub(crate) fn run_ore(&mut self) {
        if !self.ore_ready {
            // At the start each queue
            // is built afresh.
            self.ore_ready = true;
            let cells = self.map.width as usize * self.map.height as usize;
            for queues in &mut self.ore_queues {
                *queues = Queues { spread_queued: vec![false; cells], ..Queues::default() };
            }
            for kind in 0..self.ore_queues.len() {
                self.rebuild_growth(kind);
                self.rebuild_spread(kind);
            }
        }
        for kind in 0..self.ore_types.len().min(self.ore_queues.len()) {
            let rules = self.ore_types[kind];
            if self.tick >= self.ore_queues[kind].growth_at {
                self.ore_queues[kind].growth_at = self.tick + (rules.growth as f64 * 0.3) as u64;
                self.growth_pass(kind, rules);
            }
            if self.tick >= self.ore_queues[kind].spread_at {
                self.ore_queues[kind].spread_at = self.tick + rules.spread as u64;
                self.spread_pass(kind, rules);
            }
        }
    }

    // Up to a random number of cells, itself up
    // to the queue's size times `GrowthPercentage` kept between 5 and 50,
    // each gain a step; one not yet full goes back in the queue.
    fn growth_pass(&mut self, kind: usize, rules: OreType) {
        let count = self.ore_queues[kind].growth.len();
        if count == 0 || rules.growth_percentage <= 1e-5 {
            return;
        }
        let most = ((count as f64 * rules.growth_percentage) as u32).clamp(5, 50);
        let turns = self.next_random() % most + 1;
        // A heap nearly as big as the node array
        // is rebuilt first.
        if self.ore_capacity() < count + 2 * turns as usize {
            self.rebuild_growth(kind);
        }
        for _ in 0..turns {
            let Some(Reverse((_, _, cell))) = self.ore_queues[kind].growth.pop() else {
                break;
            };
            if self.ore_kind_at(cell) != Some(kind) {
                continue;
            }
            self.place_ore(cell, kind, 1);
            if self.map.ore_at(cell.0, cell.1).is_some_and(|(_, amount)| amount < 11) {
                // (Pushed directly: the engine does not check for a rebuild here.)
                let due = self.tick + (self.next_random() % 50) as u64;
                let queues = &mut self.ore_queues[kind];
                queues.serial += 1;
                queues.growth.push(Reverse((due, queues.serial, cell)));
                queues.growth_nodes += 1;
                self.queue_spread(kind, cell);
            }
        }
    }

    // Up to a random number of spreads, itself up
    // to the queue's size times `SpreadPercentage` kept between 5 and 25;
    // a cell with nowhere to spread to leaves the queue and does not count;
    // one with more than one open neighbour goes back in at once.
    fn spread_pass(&mut self, kind: usize, rules: OreType) {
        let count = self.ore_queues[kind].spread.len();
        if count == 0 || rules.spread_percentage <= 1e-5 {
            return;
        }
        let most = ((count as f64 * rules.spread_percentage) as u32).clamp(5, 25);
        let turns = self.next_random() % most + 1;
        if self.ore_capacity() < count + 20 {
            self.rebuild_spread(kind);
        }
        let mut spreads = 0;
        while spreads < turns {
            let Some(Reverse((_, _, cell))) = self.ore_queues[kind].spread.pop() else {
                break;
            };
            let free = DIRECTIONS.iter().filter(|step| self.ore_can_accept((cell.0 + step.0, cell.1 + step.1))).count();
            let index = self.map.index(cell.0, cell.1).unwrap_or(0);
            if free == 0 {
                self.ore_queues[kind].spread_queued[index] = false;
                continue;
            }
            self.spread_ore_from(cell, false);
            spreads += 1;
            if free > 1 {
                let queues = &mut self.ore_queues[kind];
                queues.serial += 1;
                queues.spread.push(Reverse((0, queues.serial, cell)));
                queues.spread_nodes += 1;
                queues.spread_queued[index] = true;
            }
        }
    }
}
