//! The map's division into regions, zones and subzones, and the search over
//! subzones that picks the corridor a route must keep to.
//!
//! Behaves as the original game does. The engine
//! floods the map into regions of one passability class, joins regions into zones
//! for each movement zone, and cuts the map into blocks of 2x2, 4x4 and 8x8 cells
//! whose connected pieces are the subzones. A path request first finds a chain of
//! 8x8 pieces, then 4x4 pieces within it, then 2x2 pieces within that; the cell
//! search may only use cells on the last chain.
//!

/// Passability classes.
pub mod class {
    pub const CLEAR: u8 = 0;
    pub const CRUSHABLE: u8 = 1;
    pub const WALL: u8 = 2;
    pub const BEACH: u8 = 3;
    pub const WATER: u8 = 4;
    pub const TREE: u8 = 5;
    pub const IMPASSABLE: u8 = 6;
    pub const OUTSIDE: u8 = 7;
}

/// Whether each movement zone can cross each class: 1 yes, 2 no, 3 outside.
const CROSSES: [[u8; 8]; 12] = [
    [1, 2, 2, 2, 2, 2, 2, 3],
    [1, 1, 2, 2, 2, 2, 2, 3],
    [1, 1, 1, 2, 2, 2, 2, 3],
    [1, 1, 1, 1, 1, 1, 2, 3],
    [1, 1, 2, 1, 1, 2, 2, 3],
    [1, 2, 2, 1, 1, 2, 2, 3],
    [1, 1, 1, 2, 2, 2, 1, 3],
    [1, 2, 2, 2, 2, 1, 2, 3],
    [1, 1, 1, 2, 2, 1, 2, 3],
    [1, 1, 1, 1, 1, 1, 1, 3],
    [2, 2, 2, 2, 1, 2, 2, 3],
    [2, 2, 2, 1, 1, 2, 2, 3],
];

/// What entering a subzone of each class costs the corridor search.
const CLASS_COST: [f32; 8] = [1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Added for a link between subzones that touch only corner to corner.
const CORNER_LINK: f64 = 0.001;

const HEAP_CAPACITY: usize = 10000;

#[derive(serde::Serialize, serde::Deserialize)]
struct Subzone {
    /// Neighbouring subzones, and whether each link is a corner one.
    neighbours: Vec<(u16, bool)>,
    /// The subzone of the next level up that it lies in.
    parent: u16,
    class: u8,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Zones {
    width: i32,
    height: i32,
    class: Vec<u8>,
    level: Vec<u8>,
    playfield: Vec<bool>,
    region: Vec<u16>,
    region_class: Vec<u8>,
    /// For each movement zone, the zone of each region.
    zone: Vec<Vec<u16>>,
    /// For each level, the subzone of each cell.
    sub: [Vec<u16>; 3],
    nodes: [Vec<Subzone>; 3],
}

/// A route's corridor: for each 2x2-block subzone whether a route may use it,
/// and each level's subzones in order from the start.
pub struct Corridor {
    pub on: Vec<bool>,
    pub order: [Vec<u16>; 3],
}

/// The links between subzones barred for the rest of one path request, per
/// level.
#[derive(Default)]
pub struct Barred(pub [Vec<u32>; 3]);

fn link_key(a: u16, b: u16) -> u32 {
    ((a.min(b) as u32) << 16) | a.max(b) as u32
}

/// Pairs of ids in the engine's order: 256 buckets, each in order of insertion.
struct Pairs {
    buckets: Vec<Vec<(u16, u16, bool)>>,
}

impl Pairs {
    fn new() -> Self {
        Self { buckets: vec![Vec::new(); 256] }
    }

    /// Records that `other` touches `id`; the first flag recorded for a pair stays.
    fn add(&mut self, other: u16, id: u16, flag: bool) {
        let bucket = &mut self.buckets[(((other & 0xf) << 4) | (id & 0xf)) as usize];
        if !bucket.iter().any(|&(a, b, _)| (a, b) == (other, id)) {
            bucket.push((other, id, flag));
        }
    }
}

impl Zones {
    /// Builds everything for a map given each cell's class, height and whether it
    /// is inside the playable area. Cells off the grid count as outside.
    pub fn build(width: i32, height: i32, class: Vec<u8>, level: Vec<u8>, playfield: Vec<bool>) -> Self {
        let cells = (width * height) as usize;
        let mut zones = Self {
            width,
            height,
            class,
            level,
            playfield,
            region: vec![0; cells],
            region_class: vec![class::OUTSIDE],
            zone: Vec::new(),
            sub: [vec![0; cells], vec![0; cells], vec![0; cells]],
            nodes: [Vec::new(), Vec::new(), Vec::new()],
        };
        zones.build_regions();
        for level in [2, 1, 0] {
            zones.build_subzones(level);
        }
        zones
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.width && y < self.height).then(|| (y * self.width + x) as usize)
    }

    /// The ground class the zones were built with at a cell.
    pub(crate) fn class_at(&self, x: i32, y: i32) -> u8 {
        self.index(x, y).map_or(class::OUTSIDE, |index| self.class[index])
    }

    fn level_at(&self, x: i32, y: i32) -> i32 {
        self.index(x, y).map_or(0, |index| self.level[index] as i32)
    }

    fn region_at(&self, x: i32, y: i32) -> u16 {
        self.index(x, y).map_or(0, |index| self.region[index])
    }

    fn sub_at(&self, level: usize, x: i32, y: i32) -> u16 {
        self.index(x, y).map_or(0, |index| self.sub[level][index])
    }

    fn in_playfield(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.playfield[index])
    }

    // Regions are numbered from 1 in the order their
    // first cell is met, row by row; each movement zone then labels the regions it
    // cannot cross 1 and joins the rest into connected groups numbered from 2.
    fn build_regions(&mut self) {
        let mut links: Vec<(u16, u16)> = Vec::new();
        let mut id: u16 = 1;
        for y in 0..self.height {
            let mut x = 0;
            while x < self.width {
                let index = (y * self.width + x) as usize;
                if self.class[index] == class::OUTSIDE || self.region[index] != 0 {
                    x += 1;
                    continue;
                }
                let mut last = 0;
                let advance = self.flood_region(x, y, id, &mut last, &mut links);
                self.region_class.push(self.class[index]);
                id += 1;
                x += advance.max(0) + 1;
            }
        }
        let regions = self.region_class.len();
        let mut touching: Vec<Vec<u16>> = vec![Vec::new(); regions];
        for &(a, b) in &links {
            touching[b as usize].push(a);
            touching[a as usize].push(b);
        }
        for crosses in &CROSSES {
            let mut zone: Vec<u16> =
                (0..regions).map(|r| u16::from(crosses[self.region_class[r] as usize] != 1)).collect();
            let mut next = 2;
            for region in 0..regions {
                if zone[region] != 0 {
                    continue;
                }
                let mut stack = vec![region];
                zone[region] = next;
                while let Some(at) = stack.pop() {
                    for &other in touching[at].iter().rev() {
                        let other = other as usize;
                        if crosses[self.region_class[other] as usize] == 1 && zone[other] == 0 {
                            zone[other] = next;
                            stack.push(other);
                        }
                    }
                }
                next += 1;
            }
            zone[0] = 0xffff;
            self.zone.push(zone);
        }
    }

    /// Fills one row's run of a region and everything reachable from it. Returns
    /// how far right of the seed the run went.
    // As in the original, including its lopsided rules: going
    // left a step of two levels ends the run, going right it takes four, measured
    // from the cell before; and the cells above and below are compared with the
    // run cell one to their right.
    fn flood_region(&mut self, seed: i32, y: i32, id: u16, last: &mut u16, links: &mut Vec<(u16, u16)>) -> i32 {
        let cls = self.class_at(seed, y);
        let rock = cls == class::IMPASSABLE;
        let mut record = |other: u16, last: &mut u16| {
            if !links.contains(&(other, id)) {
                links.push((other, id));
            }
            *last = other;
        };
        let mut prev = self.level_at(seed, y);
        let mut left = seed;
        loop {
            if (self.level_at(left, y) - prev).abs() >= 2 {
                break;
            }
            let index = (y * self.width + left) as usize;
            self.region[index] = id;
            prev = self.level[index] as i32;
            left -= 1;
            if self.class_at(left, y) != cls {
                break;
            }
        }
        let other = self.region_at(left, y);
        if other != 0 && ((self.level_at(left, y) - prev).abs() < 2 || rock) && other != *last && other != id {
            record(other, last);
        }
        let mut right = seed;
        loop {
            if (self.level_at(right, y) - prev).abs() >= 4 {
                break;
            }
            let index = (y * self.width + right) as usize;
            self.region[index] = id;
            prev = self.level[index] as i32;
            right += 1;
            if self.class_at(right, y) != cls {
                break;
            }
        }
        let other = self.region_at(right, y);
        if other != 0 && ((self.level_at(right, y) - prev).abs() < 2 || rock) && other != *last && other != id {
            record(other, last);
        }
        let advance = right - seed - 1;
        let (first, end) = (left + 1, right - 1);
        for row in [y - 1, y + 1] {
            let mut x = first - 1;
            while x <= end + 1 {
                let beside = self.level_at(if x < end { x + 1 } else { end }, y);
                let near = (self.level_at(x, row) - beside).abs() < 2;
                let other = self.region_at(x, row);
                if other == 0 {
                    if self.class_at(x, row) == cls && near {
                        x += self.flood_region(x, row, id, last, links).max(0);
                    } else {
                        x += 1;
                    }
                } else {
                    if other != id && other != *last && (near || rock) {
                        if !links.contains(&(other, id)) {
                            links.push((other, id));
                        }
                        *last = other;
                    }
                    x += 1;
                }
            }
        }
        advance
    }

    // Blocks of 2, 4 or 8 cells a side; subzones are
    // numbered from 1 in the order their first cell is met, row by row; links are
    // gathered into 256 buckets and then handed out to both ends in bucket order.
    fn build_subzones(&mut self, level: usize) {
        let size = 1 << (level + 1);
        let mut pairs = Pairs::new();
        self.nodes[level].push(Subzone { neighbours: Vec::new(), parent: 0, class: class::OUTSIDE });
        let mut id: u16 = 1;
        for y in 0..self.height {
            let mut x = 0;
            while x < self.width {
                let index = (y * self.width + x) as usize;
                if self.class[index] == class::OUTSIDE || self.sub[level][index] != 0 {
                    x += 1;
                    continue;
                }
                let parent = if level < 2 { self.sub[level + 1][index] } else { 0 };
                let block = (x - x % size, y - y % size, size);
                let advance = self.flood_subzone(x, y, level, id, block, &mut pairs);
                self.nodes[level].push(Subzone { neighbours: Vec::new(), parent, class: self.class[index] });
                id += 1;
                x += advance.max(0) + 1;
            }
        }
        for bucket in &pairs.buckets {
            for &(other, own, flag) in bucket {
                self.nodes[level][own as usize].neighbours.push((other, flag));
                self.nodes[level][other as usize].neighbours.push((own, flag));
            }
        }
    }

    /// Fills one row's run of a subzone and what it reaches within its block,
    /// noting the subzones it touches. Returns how far right of the seed the run
    /// went.
    fn flood_subzone(&mut self, seed: i32, y: i32, level: usize, id: u16, block: (i32, i32, i32), pairs: &mut Pairs) -> i32 {
        let (x_min, x_max) = (block.0, block.0 + block.2 - 1);
        let (y_min, y_max) = (block.1, block.1 + block.2 - 1);
        let region = self.region_at(seed, y);
        let mut last: i32 = -1;

        let mut prev = self.level_at(seed, y);
        let mut left = seed;
        loop {
            if left < x_min || (self.level_at(left, y) - prev).abs() >= 2 {
                break;
            }
            let index = (y * self.width + left) as usize;
            self.sub[level][index] = id;
            prev = self.level[index] as i32;
            left -= 1;
            if self.region_at(left, y) != region {
                break;
            }
        }
        let other = self.sub_at(level, left, y);
        if other > 0
            && (self.level_at(left, y) - prev).abs() < 2
            && self.in_playfield(left, y)
            && self.in_playfield(left + 1, y)
        {
            pairs.add(other, id, false);
            last = other as i32;
        }

        prev = self.level_at(seed, y);
        let mut right = seed;
        loop {
            if right > x_max || (self.level_at(right, y) - prev).abs() >= 2 {
                break;
            }
            let index = (y * self.width + right) as usize;
            self.sub[level][index] = id;
            prev = self.level[index] as i32;
            right += 1;
            if self.region_at(right, y) != region {
                break;
            }
        }
        let other = self.sub_at(level, right, y);
        if other > 0
            && (self.level_at(right, y) - prev).abs() < 2
            && other as i32 != last
            && self.in_playfield(right, y)
            && self.in_playfield(right - 1, y)
        {
            pairs.add(other, id, false);
            last = other as i32;
        }
        let advance = right - seed - 1;

        for (row, allowed) in [(y - 1, y > y_min), (y + 1, y < y_max)] {
            let mut x = left;
            while x <= right {
                let beside = if x == left {
                    left + 1
                } else if x < right {
                    x
                } else {
                    x - 1
                };
                let near = (self.level_at(x, row) - self.level_at(beside, y)).abs() < 2;
                let other = self.sub_at(level, x, row);
                if other == 0 && allowed && x >= x_min && x <= x_max {
                    if self.region_at(x, row) == region && near && self.index(x, row).is_some() {
                        // Not stepped past: the next look finds it filled.
                        self.flood_subzone(x, row, level, id, block, pairs);
                    } else {
                        x += 1;
                    }
                } else {
                    if other != id
                        && other as i32 != last
                        && other != 0
                        && near
                        && self.in_playfield(beside, y)
                        && self.in_playfield(x, row)
                    {
                        pairs.add(other, id, x < x_min || x > x_max);
                        last = other as i32;
                    }
                    x += 1;
                }
            }
        }
        advance
    }

    /// Joins the zones of two cells, for every movement zone that can stand on
    /// both (the two ends of a bridge).
    // The subzones at a bridge's ends are neighbours, so a corridor can run
    // over the bridge.
    pub fn link(&mut self, a: (i32, i32), b: (i32, i32)) {
        if let (Some(ia), Some(ib)) = (self.index(a.0, a.1), self.index(b.0, b.1)) {
            for level in 0..3 {
                let (sa, sb) = (self.sub[level][ia], self.sub[level][ib]);
                if sa != sb && sa != 0 && sb != 0 {
                    for (from, to) in [(sa, sb), (sb, sa)] {
                        let neighbours = &mut self.nodes[level][from as usize].neighbours;
                        if !neighbours.iter().any(|&(other, _)| other == to) {
                            neighbours.push((to, false));
                        }
                    }
                }
            }
        }
        let (ra, rb) = (self.region_at(a.0, a.1) as usize, self.region_at(b.0, b.1) as usize);
        for table in &mut self.zone {
            let (za, zb) = (table[ra], table[rb]);
            if za >= 2 && zb >= 2 && za != zb && za != 0xffff && zb != 0xffff {
                for zone in table.iter_mut().filter(|zone| **zone == zb) {
                    *zone = za;
                }
            }
        }
    }

    /// The zone a cell is in for a movement zone: 1 where that movement zone cannot
    /// go, otherwise the number of its connected area.
    pub fn zone_of(&self, cell: (i32, i32), movement_zone: usize) -> u16 {
        self.zone[movement_zone.min(11)][self.region_at(cell.0, cell.1) as usize]
    }

    /// Regions, zones a ground vehicle can use, and subzones at each level, for
    /// diagnostics.
    pub fn counts(&self) -> (usize, usize, [usize; 3]) {
        let zones = self.zone[0].iter().filter(|&&zone| zone >= 2 && zone != 0xffff).max().map_or(0, |&top| top as usize - 1);
        (self.region_class.len() - 1, zones, [self.nodes[0].len() - 1, self.nodes[1].len() - 1, self.nodes[2].len() - 1])
    }

    /// Whether a cell is inside the playable area.
    pub fn playable(&self, cell: (i32, i32)) -> bool {
        self.in_playfield(cell.0, cell.1)
    }

    /// The 2x2-block subzone a cell is in.
    pub fn subzone(&self, cell: (i32, i32)) -> u16 {
        self.sub_at(0, cell.0, cell.1)
    }

    /// Picks the corridor from one cell to another: for each 2x2-block subzone,
    /// whether a route may use it. `None` if the subzones do not connect.
    // For 8x8, then 4x4, then 2x2 blocks, a search by
    // cost alone (no estimate) from the start's subzone to the destination's, over
    // neighbour lists in their stored order; below the top level a subzone is only
    // taken if its parent is on the corridor above or it is of the crushable class.
    pub fn corridor(&self, start: (i32, i32), dest: (i32, i32), movement_zone: usize) -> Option<Vec<bool>> {
        self.corridor_barred(start, dest, movement_zone, &Barred::default()).map(|corridor| corridor.on)
    }

    /// As `corridor`, leaving out the links barred after failed searches, and
    /// keeping each level's subzones in order from the start.
    // A barred link is passed over once it has met
    // every other test; each level has its own list.
    pub fn corridor_barred(&self, start: (i32, i32), dest: (i32, i32), movement_zone: usize, barred: &Barred) -> Option<Corridor> {
        let crosses = &CROSSES[movement_zone.min(11)];
        let mut above: Vec<bool> = Vec::new();
        let mut order: [Vec<u16>; 3] = Default::default();
        for level in [2usize, 1, 0] {
            let nodes = &self.nodes[level];
            let (from, to) = (self.sub_at(level, start.0, start.1), self.sub_at(level, dest.0, dest.1));
            let mut on = vec![false; nodes.len()];
            on[from as usize] = true;
            on[to as usize] = true;
            if from != to {
                // Search records: parent record, subzone, cost so far.
                let mut pool: Vec<(u32, u16, f32)> = vec![(u32::MAX, from, 0.0)];
                let mut heap: Vec<u32> = vec![0];
                let mut seen = vec![false; nodes.len()];
                let mut cost = vec![0f32; nodes.len()];
                seen[from as usize] = true;
                let mut current = heap_pop(&mut heap, &pool);
                let found = loop {
                    let at = current?;
                    let (_, here, here_cost) = pool[at as usize];
                    if here == to {
                        break at;
                    }
                    for &(next, corner) in &nodes[here as usize].neighbours {
                        let node = &nodes[next as usize];
                        let g = (CLASS_COST[node.class as usize] as f64
                            + here_cost as f64
                            + if corner { CORNER_LINK } else { 0.0 }) as f32;
                        if seen[next as usize] && cost[next as usize] <= g {
                            continue;
                        }
                        if level != 2 && !above[node.parent as usize] && node.class != class::CRUSHABLE {
                            continue;
                        }
                        if crosses[node.class as usize] != 1 {
                            continue;
                        }
                        if barred.0[level].contains(&link_key(here, next)) {
                            continue;
                        }
                        pool.push((at, next, g));
                        if heap.len() + 2 < HEAP_CAPACITY {
                            heap_push(&mut heap, &pool, pool.len() as u32 - 1);
                        }
                        seen[next as usize] = true;
                        cost[next as usize] = g;
                    }
                    current = heap_pop(&mut heap, &pool);
                };
                let mut at = found;
                let mut back = vec![to];
                while pool[at as usize].0 != u32::MAX {
                    on[pool[at as usize].1 as usize] = true;
                    at = pool[at as usize].0;
                    back.push(pool[at as usize].1);
                }
                back.reverse();
                order[level] = back;
            } else {
                order[level] = vec![from];
            }
            above = on;
        }
        Some(Corridor { on: above, order })
    }

    /// After a search kept to a corridor failed at a cell: bars links so the
    /// next corridor goes another way. Returns whether the corridor may still be
    /// used.
    // For each level, the
    // stuck cell's subzone is flooded from the cell over the cells a mover can
    // enter (an occupied cell of a crossable kind stops it). If the subzone
    // is judged split, the corridor's link out of it is barred, and the links
    // from the subzone before it to the other neighbours of the next one;
    // otherwise its links to the neighbours the flood never touched are.
    // The split test looks at one cell only, the stuck cell moved on by its
    // own offset in its block, and is true if that cell is in the subzone and
    // any of the block's cells was not flooded.
    pub fn bar_after_failure(&self, corridor: &Corridor, stuck: (i32, i32), movement_zone: usize, blocked: &dyn Fn((i32, i32), usize, (i32, i32)) -> bool, barred: &mut Barred) -> bool {
        let crosses = &CROSSES[movement_zone.min(11)];
        let mut usable = true;
        for level in 0..3 {
            let n = 2i32 << level;
            let mask = n - 1;
            let s = self.sub_at(level, stuck.0, stuck.1);
            let mut visited = vec![false; (n * n) as usize];
            let slot = |cell: (i32, i32)| ((cell.0 & mask) * n + (cell.1 & mask)) as usize;
            let mut touched: Vec<u16> = Vec::new();
            let mut last = 0u16;
            let mut stack = vec![stuck];
            visited[slot(stuck)] = true;
            while let Some(cell) = stack.pop() {
                for (dir, step) in crate::path::DIRECTIONS.iter().enumerate() {
                    let next = (cell.0 + step.0, cell.1 + step.1);
                    let Some(index) = self.index(next.0, next.1) else {
                        continue;
                    };
                    let t = self.sub[level][index];
                    if crosses[self.class[index] as usize] == 1 && blocked(next, dir, cell) {
                        continue;
                    }
                    if t == s {
                        if !visited[slot(next)] {
                            visited[slot(next)] = true;
                            stack.push(next);
                        }
                    } else if t != last && t != 0 {
                        if !touched.contains(&t) {
                            touched.push(t);
                        }
                        last = t;
                    }
                }
            }
            let probe = (stuck.0 + (stuck.0 & mask), stuck.1 + (stuck.1 & mask));
            let split = self.in_playfield(probe.0, probe.1) && self.sub_at(level, probe.0, probe.1) == s && visited.iter().any(|v| !v);
            if split {
                let path = &corridor.order[level];
                match path.iter().position(|&id| id == s) {
                    Some(k) if path.len() >= 2 => {
                        let (a, b) = if k + 1 < path.len() { (s, path[k + 1]) } else { (path[k - 1], s) };
                        barred.0[level].push(link_key(a, b));
                        let nodes = &self.nodes[level];
                        for &(other, _) in &nodes[b as usize].neighbours {
                            if other != a && nodes[a as usize].neighbours.iter().any(|&(x, _)| x == other) {
                                barred.0[level].push(link_key(a, other));
                            }
                        }
                    }
                    _ => usable = false,
                }
            } else {
                for &(other, _) in self.nodes[level][s as usize].neighbours.iter().rev() {
                    if !touched.contains(&other) && other != s {
                        barred.0[level].push(link_key(s, other));
                    }
                }
            }
        }
        usable
    }

    /// The subzone of a cell at a level of the corridor search (0: 2x2 blocks).
    pub fn subzone_at(&self, level: usize, cell: (i32, i32)) -> u16 {
        self.sub_at(level, cell.0, cell.1)
    }
}

fn heap_push(heap: &mut Vec<u32>, pool: &[(u32, u16, f32)], record: u32) {
    heap.push(record);
    let mut at = heap.len() - 1;
    while at > 0 {
        let parent = (at - 1) / 2;
        if pool[heap[parent] as usize].2 > pool[record as usize].2 {
            heap[at] = heap[parent];
            at = parent;
        } else {
            break;
        }
    }
    heap[at] = record;
}

fn heap_pop(heap: &mut Vec<u32>, pool: &[(u32, u16, f32)]) -> Option<u32> {
    let top = *heap.first()?;
    let last = heap.pop()?;
    if !heap.is_empty() {
        heap[0] = last;
        let mut at = 0;
        loop {
            let (left, right) = (2 * at + 1, 2 * at + 2);
            let mut least = at;
            if left < heap.len() && pool[heap[least] as usize].2 > pool[heap[left] as usize].2 {
                least = left;
            }
            if right < heap.len() && pool[heap[least] as usize].2 > pool[heap[right] as usize].2 {
                least = right;
            }
            if least == at {
                break;
            }
            heap.swap(at, least);
            at = least;
        }
    }
    Some(top)
}
