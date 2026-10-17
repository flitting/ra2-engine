//! The engine's cell search: the route a ground unit takes from one cell to another,
//! as a list of directions. It behaves as the original game's regular search does,
//! quirks included. What is missing is listed at `search`.

/// The cell offset for each of the eight directions, clockwise from north.
pub const DIRECTIONS: [(i32, i32); 8] =
    [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

/// What stands in the way of entering a cell. Anything from `NO` up cannot be
/// entered at all.
// The values.
pub mod code {
    pub const OK: u8 = 0;
    pub const CLOAK: u8 = 1;
    pub const MOVING_BLOCK: u8 = 2;
    pub const CLOSED_GATE: u8 = 3;
    pub const FRIENDLY_DESTROYABLE: u8 = 4;
    pub const DESTROYABLE: u8 = 5;
    pub const TEMP: u8 = 6;
    pub const NO: u8 = 7;
}

/// How the search treats cells held by units on the move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Follow the chain of blockers to see whether it will clear.
    Normal,
    /// Moving units cost 4.
    Avoid,
    /// Moving units cost 1000: go round everything.
    AvoidAll,
}

/// What a step into a cell costs for each entry code other than a moving block.
pub fn code_cost(code: u8) -> f32 {
    match code {
        code::CLOAK => 1000.0,
        code::FRIENDLY_DESTROYABLE => 60.0,
        code::DESTROYABLE => 20.0,
        code::TEMP => 8.0,
        code::NO => 10000.0,
        _ => 1.0,
    }
}

/// A small extra cost per direction, which settles ties.
const TIE_BREAK: [f32; 8] = [0.001, 0.005, 0.002, 0.006, 0.003, 0.007, 0.004, 0.008];

/// A cell already reached is not looked at again unless its cost is at least this
/// much more than the cell it would now be reached from.
const REVISIT_MARGIN: f64 = 1.009;

/// Most cells the search will work outward from.
const LOOP_CAP: u32 = 0xfff7;

/// Room in the open list; a cell that does not fit is forgotten.
const HEAP_CAPACITY: usize = 0x10000;

/// The most steps a unit keeps of a route; it asks again when they run low.
// The unit's path holds 24 entries.
pub const PATH_LENGTH: usize = 24;

struct Node {
    cell: u32,
    /// The level it is at: the cell's own, or four more on a bridge's deck.
    level: i32,
    parent: u32,
    g: f32,
    f: f32,
    depth: u32,
}

/// What the search needs to know about the map and the unit.
pub struct Query<'a> {
    pub width: i32,
    pub height: i32,
    /// The entry code for stepping into a cell in a direction from a neighbour,
    /// for a mover at a level (-1: not known).
    pub enter: &'a dyn Fn((i32, i32), usize, (i32, i32), i32) -> u8,
    /// Whether a high bridge passes over a cell.
    pub bridged: &'a dyn Fn((i32, i32)) -> bool,
    /// The levels the mover starts at and is to end at.
    pub start_level: i32,
    pub dest_level: i32,
    /// The cost of a step into a cell with an entry code below `NO`.
    pub cost: &'a dyn Fn(u8, (i32, i32)) -> f32,
    /// Height level of a cell.
    pub level: &'a dyn Fn((i32, i32)) -> i32,
    /// Whether the search may use a cell at all: on the corridor picked for this
    /// route, or let off that rule.
    pub allowed: &'a dyn Fn((i32, i32)) -> bool,
    /// Told of each cell the search reaches, as it reaches it.
    pub opened: &'a dyn Fn((i32, i32)),
}

/// The directions to take from `start` to `dest`, or `None` if there is no way.
// Eight neighbours clockwise from
// north; straight and diagonal steps cost the same; the estimate to the goal is the
// straight-line distance in cells, from the engine's table square root; a diagonal
// step is judged by its target cell alone; a cell once reached is never reached
// again by another way; the cheapest child of a cell is held back from the open
// list and taken next if it beats the list's best; if the destination itself cannot
// be entered, the route stops at the first cell worked from that touches it; sums
// are rounded to single precision where they are stored; cells off the corridor
// picked beforehand (see `zones`) are passed over. The result is then straightened
// by the engine's two passes (`flatten_zigzags`, `straighten`).
pub fn search(query: &Query, start: (i32, i32), dest: (i32, i32)) -> Option<Vec<u8>> {
    let index = |cell: (i32, i32)| -> Option<usize> {
        (cell.0 >= 0 && cell.1 >= 0 && cell.0 < query.width && cell.1 < query.height)
            .then(|| (cell.1 * query.width + cell.0) as usize)
    };
    let (start_index, dest_index) = (index(start)?, index(dest)?);
    if start_index == dest_index && query.start_level == query.dest_level {
        return None;
    }
    let cell_of = |index: u32| (index as i32 % query.width, index as i32 / query.width);
    let estimate = |cell: (i32, i32), g: f32| -> f32 {
        let (dx, dy) = (cell.0 - dest.0, cell.1 - dest.1);
        (engine_sqrt((dx * dx + dy * dy) as f32) as f64 + g as f64) as f32
    };

    let cells = (query.width * query.height) as usize;
    // The search's place is a
    // cell and a layer: the ground, or a bridge's deck, each with its own
    // marks. A neighbour is taken on its deck when it has a bridge and the
    // level being worked from is more than one from its ground; the search is
    // over only at the destination cell on the destination's level.
    let mut visited = vec![false; cells * 2];
    let mut cost = vec![0f32; cells * 2];
    let layer = |cell: (i32, i32), level: i32| usize::from((query.bridged)(cell) && (level - (query.level)(cell)).abs() > 1);
    let mut nodes: Vec<Node> = Vec::new();
    // A binary heap of node numbers, smallest `f` first.
    let mut heap: Vec<u32> = Vec::new();

    nodes.push(Node { cell: start_index as u32, level: query.start_level, parent: u32::MAX, g: 0.0, f: estimate(start, 0.0), depth: 1 });
    visited[start_index + cells * layer(start, query.start_level)] = true;
    let mut current = Some(0u32);
    let mut loops = 0u32;
    let mut found = None;

    'search: while let Some(at) = current {
        if loops >= LOOP_CAP {
            break;
        }
        let (here_index, here_g, here_level) = (nodes[at as usize].cell, nodes[at as usize].g, nodes[at as usize].level);
        if here_index as usize == dest_index && here_level == query.dest_level {
            found = Some(at);
            break;
        }
        let here = cell_of(here_index);
        let mut pending: Option<u32> = None;
        for (dir, offset) in DIRECTIONS.iter().enumerate() {
            let next = (here.0 + offset.0, here.1 + offset.1);
            let Some(next_index) = index(next) else {
                continue;
            };
            let on_deck = layer(next, here_level) == 1;
            // A deck neighbour is never passed over for being off the corridor.
            if !on_deck && !(query.allowed)(next) {
                continue;
            }
            let mark = next_index + if on_deck { cells } else { 0 };
            if visited[mark] && (cost[mark] as f64) < here_g as f64 + REVISIT_MARGIN {
                continue;
            }
            let entry = (query.enter)(next, dir, here, here_level);
            if entry < code::NO {
                if visited[mark] {
                    continue;
                }
                let step = ((query.cost)(entry, next) as f64 + TIE_BREAK[dir] as f64) as f32;
                let g = (step as f64 + here_g as f64) as f32;
                let child = nodes.len() as u32;
                nodes.push(Node {
                    cell: next_index as u32,
                    level: (query.level)(next) + if on_deck { 4 } else { 0 },
                    parent: at,
                    g,
                    f: estimate(next, g),
                    depth: nodes[at as usize].depth + 1,
                });
                // The cheapest child so far is held back; the rest go on the open
                // list. A tie keeps the earlier one held.
                match pending {
                    None => pending = Some(child),
                    Some(held) if nodes[child as usize].f < nodes[held as usize].f => {
                        push(&mut heap, &nodes, held);
                        pending = Some(child);
                    }
                    Some(_) => push(&mut heap, &nodes, child),
                }
                visited[mark] = true;
                (query.opened)(next);
                cost[mark] = g;
            } else if next_index == dest_index && (here_level - query.dest_level).abs() < 2 {
                found = Some(at);
                break 'search;
            }
        }
        current = match pending {
            None => pop(&mut heap, &nodes),
            Some(child) => match heap.first() {
                Some(&top) if nodes[top as usize].f <= nodes[child as usize].f => {
                    heap[0] = child;
                    sift_down(&mut heap, &nodes);
                    Some(top)
                }
                _ => Some(child),
            },
        };
        loops += 1;
    }

    let end = found?;
    if loops == 10000 || loops == LOOP_CAP || nodes[end as usize].depth <= 1 {
        return None;
    }
    let mut steps = Vec::new();
    let mut at = end;
    while nodes[at as usize].parent != u32::MAX {
        let parent = nodes[at as usize].parent;
        let (from, to) = (cell_of(nodes[parent as usize].cell), cell_of(nodes[at as usize].cell));
        let offset = (to.0 - from.0, to.1 - from.1);
        steps.push(DIRECTIONS.iter().position(|&d| d == offset).unwrap_or(0) as u8);
        at = parent;
    }
    steps.reverse();
    let count = steps.len();
    // Room for the end mark and the one slot past it that the engine also clears.
    let mut dirs: Vec<i8> = steps.iter().map(|&step| step as i8).chain([-1, -1]).collect();
    flatten_zigzags(query, &mut dirs, count, start);
    let count = straighten(query, &mut dirs, count, start);
    Some(dirs[..count].iter().map(|&dir| dir as u8 & 7).collect())
}

/// The engine's square root: exact for small perfect squares, otherwise a little under.
// The mantissa comes from a table of
// 16384 entries, each the truncated root of the value at the start of its interval.
// The table is rebuilt here from that rule, which every entry in the executable
// fits.
pub(crate) fn engine_sqrt(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    let bits = x.abs().to_bits();
    let exponent = (bits >> 23) as i32 - 127;
    let mut mantissa = bits & 0x7f_ffff;
    if exponent & 1 != 0 {
        mantissa |= 0x80_0000;
    }
    let slot = (mantissa >> 10) as f64;
    let value = if slot < 8192.0 { 1.0 + slot / 8192.0 } else { 2.0 * (1.0 + (slot - 8192.0) / 8192.0) };
    let entry = ((value.sqrt() - 1.0) * 8_388_608.0) as u32;
    f32::from_bits(entry.wrapping_add((((exponent >> 1) + 127) as u32) << 23))
}

fn offset(dir: i8) -> (i32, i32) {
    DIRECTIONS[dir as usize & 7]
}

fn add(cell: (i32, i32), by: (i32, i32)) -> (i32, i32) {
    (cell.0 + by.0, cell.1 + by.1)
}

fn walk(mut cell: (i32, i32), dirs: &[i8]) -> (i32, i32) {
    for &dir in dirs {
        cell = add(cell, offset(dir));
    }
    cell
}

/// First straightening pass: where a run of one diagonal is followed by a run of
/// the diagonal a quarter turn from it, replaces as much of the pair as it can with
/// the straight direction between them.
// As in the original, including that after a
// replacement the scan carries on with the new straight run as if it were a
// diagonal one, which can turn two straight runs into diagonals and move the end of
// the route.
fn flatten_zigzags(query: &Query, dirs: &mut [i8], n: usize, start: (i32, i32)) {
    if n == 0 {
        return;
    }
    let (mut run, mut a, mut s) = (-1i8, 0usize, 0usize);
    let (mut b, mut b_start, mut in_b) = (0usize, 0usize, false);
    let mut second = -1i8;
    let (mut run_cell, mut cur) = (start, start);
    loop {
        if b_start + b >= n {
            break;
        }
        if !in_b {
            let at = s + a;
            let dir = dirs[at];
            if dir == run {
                a += 1;
            } else if matches!((dir - run) & 7, 2 | 6) && run != -1 {
                in_b = true;
                second = dir;
                b = 1;
                b_start = at;
            } else {
                s = at;
                a = 1;
                run = if dir & 1 != 0 { dir } else { -1 };
                run_cell = cur;
            }
            cur = add(cur, offset(dir));
        } else if dirs[b_start + b] == second {
            b += 1;
        } else {
            s += try_flatten(query, &mut dirs[s..], a, b, &mut run_cell);
            a = 1;
            in_b = false;
            run = dirs[s];
            cur = add(run_cell, offset(dirs[s]));
        }
        if s + a >= n {
            break;
        }
    }
    if in_b {
        try_flatten(query, &mut dirs[s..], a, b, &mut run_cell);
    }
}

fn try_flatten(query: &Query, dirs: &mut [i8], a: usize, b: usize, cell: &mut (i32, i32)) -> usize {
    let (first, second) = (dirs[0], dirs[a]);
    let mut middle = (first + second) >> 1;
    if middle + 1 != second && middle + 1 != first {
        middle = 0;
    }
    let mut k = a.min(b);
    let mut base = walk(*cell, &dirs[..a - k]);
    while k > 0 {
        let (mut from, mut clear) = (base, true);
        for _ in 0..2 * k {
            let to = add(from, offset(middle));
            if (query.enter)(to, middle as usize & 7, from, -1) != code::OK {
                clear = false;
                break;
            }
            from = to;
        }
        if clear {
            let kept = a - k;
            dirs[kept..kept + 2 * k].fill(middle);
            *cell = walk(*cell, &dirs[..kept]);
            return kept;
        }
        base = add(base, offset(first));
        k -= 1;
    }
    *cell = walk(*cell, &dirs[..a]);
    a
}

/// Second straightening pass: where the first twenty steps wander without getting
/// further from where a stretch began, replaces the stretch with the direct way, and
/// closes up the gaps. Returns the new number of steps.
fn straighten(query: &Query, dirs: &mut [i8], n: usize, start: (i32, i32)) -> usize {
    let far = |v: (i32, i32)| v.0.abs().max(v.1.abs());
    let mut cur = start;
    let (mut segment, mut anchor_at, mut best) = (0usize, 0usize, 0i32);
    let (mut whole, mut since) = ((0, 0), (0, 0));
    let mut reach = (0, 0);
    let mut anchor = (0, 0);
    let mut i = 0;
    while i < n && i < 20 {
        let dir = dirs[i];
        if dir == -2 {
            i += 1;
            continue;
        }
        let by = offset(dir);
        let (new_since, new_whole) = (add(since, by), add(whole, by));
        if new_since.0.abs() < reach.0 || new_since.1.abs() < reach.1 {
            // The route has turned back on itself; the step is looked at again.
            if anchor != (0, 0) {
                segment = anchor_at;
                whole = (anchor.0 - cur.0, anchor.1 - cur.1);
                best = far(whole);
            }
            reach = (0, 0);
            since = (0, 0);
            anchor = cur;
            anchor_at = i;
            continue;
        }
        reach = (new_since.0.abs(), new_since.1.abs());
        since = new_since;
        let length = far(new_whole);
        cur = add(cur, by);
        if best < length {
            best = length;
        } else {
            let mut cell = cur;
            let from = stretch_start(dirs, i, segment, &mut cell);
            let delta = (cur.0 - cell.0, cur.1 - cell.1);
            try_direct(query, &mut dirs[from..=i], cell, delta);
        }
        whole = new_whole;
        i += 1;
    }
    if n > 0 && anchor != (0, 0) {
        let length = far((cur.0 - anchor.0, cur.1 - anchor.1));
        if i as i32 - anchor_at as i32 - 1 > length {
            let last = i - 1;
            let mut cell = cur;
            let from = stretch_start(dirs, last, anchor_at, &mut cell);
            let delta = (cur.0 - cell.0, cur.1 - cell.1);
            try_direct(query, &mut dirs[from..=last], cell, delta);
        }
    }
    let mut out = 0;
    if dirs[0] != -1 {
        let mut j = 0;
        while j < n {
            if dirs[j] != -2 {
                dirs[out] = dirs[j];
                out += 1;
            }
            j += 1;
            if dirs[j] == -1 {
                break;
            }
        }
    }
    for slot in &mut dirs[out..=n + 1] {
        *slot = -1;
    }
    out
}

/// Walks back from a step to find where the stretch that stopped gaining ground
/// began. Returns its first step, and leaves the cell before it in `cell`.
fn stretch_start(dirs: &[i8], last: usize, low: usize, cell: &mut (i32, i32)) -> usize {
    let (mut gone, mut best, mut flat) = ((0, 0), 0, false);
    let mut at = *cell;
    let mut j = last as i32;
    while j >= low as i32 {
        let dir = dirs[j as usize];
        if dir == -2 {
            j -= 1;
            continue;
        }
        let back = offset((dir - 4) & 7);
        let (new_gone, new_at) = (add(gone, back), add(at, back));
        let length = new_gone.0.abs().max(new_gone.1.abs());
        if length > best {
            if flat {
                *cell = at;
                return j as usize + 1;
            }
            best = length;
        } else {
            flat = true;
        }
        gone = new_gone;
        at = new_at;
        j -= 1;
    }
    *cell = at;
    low
}

/// Tries to replace some steps with the direct way across the same displacement:
/// the diagonal part then the straight part, or failing that the other way round.
fn try_direct(query: &Query, out: &mut [i8], start: (i32, i32), delta: (i32, i32)) -> bool {
    let (dx, dy) = delta;
    let diagonal: i8 = if dx < 0 { if dy >= 0 { 5 } else { 7 } } else if dy >= 0 { 3 } else { 1 };
    let straight: i8 = if dx - dy > 0 { if dx + dy <= 0 { 0 } else { 2 } } else if dx + dy <= 0 { 6 } else { 4 };
    let mut legs = [
        (diagonal, dx.abs().min(dy.abs()) as usize),
        (straight, (dx.abs().max(dy.abs()) - dx.abs().min(dy.abs())) as usize),
    ];
    for _ in 0..2 {
        // An attempt whose first leg is empty is passed over.
        if legs[0].1 != 0 {
            let mut from = start;
            let clear = legs.iter().flat_map(|&(dir, count)| std::iter::repeat_n(dir, count)).all(|dir| {
                let to = add(from, offset(dir));
                let entry = (query.enter)(to, dir as usize, from, -1);
                from = to;
                entry == code::OK
            });
            if clear {
                let (first, second) = (legs[0].1, legs[1].1);
                out[..first].fill(legs[0].0);
                out[first..first + second].fill(legs[1].0);
                out[first + second..].fill(-2);
                return true;
            }
        }
        legs.swap(0, 1);
    }
    false
}

// A new entry goes at the end and moves up past each
// parent with a larger `f`.
fn push(heap: &mut Vec<u32>, nodes: &[Node], node: u32) {
    if heap.len() + 2 >= HEAP_CAPACITY {
        return;
    }
    heap.push(node);
    let mut at = heap.len() - 1;
    while at > 0 {
        let parent = (at - 1) / 2;
        if nodes[heap[parent] as usize].f > nodes[node as usize].f {
            heap[at] = heap[parent];
            at = parent;
        } else {
            break;
        }
    }
    heap[at] = node;
}

fn pop(heap: &mut Vec<u32>, nodes: &[Node]) -> Option<u32> {
    let top = *heap.first()?;
    let last = heap.pop()?;
    if !heap.is_empty() {
        heap[0] = last;
        sift_down(heap, nodes);
    }
    Some(top)
}

// The smaller child is found with a strict
// comparison, the left one first.
fn sift_down(heap: &mut [u32], nodes: &[Node]) {
    let f = |heap: &[u32], slot: usize| nodes[heap[slot] as usize].f;
    let mut at = 0;
    loop {
        let (left, right) = (2 * at + 1, 2 * at + 2);
        let mut smallest = at;
        if left < heap.len() && f(heap, left) < f(heap, smallest) {
            smallest = left;
        }
        if right < heap.len() && f(heap, right) < f(heap, smallest) {
            smallest = right;
        }
        if smallest == at {
            break;
        }
        heap.swap(at, smallest);
        at = smallest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(blocked: &[(i32, i32)], start: (i32, i32), dest: (i32, i32)) -> Option<Vec<u8>> {
        let enter = |to: (i32, i32), _: usize, _: (i32, i32)| {
            if blocked.contains(&to) || to.0 < 0 || to.1 < 0 || to.0 >= 40 || to.1 >= 40 { code::NO } else { code::OK }
        };
        search(&Query { width: 40, height: 40, enter: &|to, dir, from, _| enter(to, dir, from), cost: &|entry, _| code_cost(entry), level: &|_| 0, allowed: &|_| true, opened: &|_| {}, bridged: &|_| false, start_level: 0, dest_level: 0 }, start, dest)
    }

    fn end(start: (i32, i32), steps: &[u8]) -> (i32, i32) {
        steps.iter().fold(start, |cell, &dir| add(cell, DIRECTIONS[dir as usize]))
    }

    #[test]
    fn open_ground_routes_are_direct() {
        for dest in [(30, 10), (10, 30), (25, 25), (3, 17), (12, 4), (31, 12), (20, 2)] {
            let steps = route(&[], (10, 10), dest).unwrap();
            assert_eq!(end((10, 10), &steps), dest, "{steps:?}");
            let direct = (dest.0 - 10i32).abs().max((dest.1 - 10i32).abs()) as usize;
            assert_eq!(steps.len(), direct, "to {dest:?}: {steps:?}");
        }
    }

    #[test]
    fn a_wall_is_gone_round_and_a_closed_destination_is_stopped_beside() {
        let wall: Vec<(i32, i32)> = (5..=25).map(|y| (20, y)).collect();
        let steps = route(&wall, (10, 15), (30, 15)).unwrap();
        assert_eq!(end((10, 15), &steps), (30, 15));
        let mut cell = (10, 15);
        for &dir in &steps {
            cell = add(cell, DIRECTIONS[dir as usize]);
            assert!(!wall.contains(&cell));
        }
        let steps = route(&[(30, 15)], (10, 15), (30, 15)).unwrap();
        let stop = end((10, 15), &steps);
        assert!((stop.0 - 30).abs() <= 1 && (stop.1 - 15).abs() <= 1 && stop != (30, 15));
        assert!(route(&[], (10, 15), (10, 15)).is_none());
    }

    #[test]
    fn the_square_root_is_exact_for_squares_and_just_under_otherwise() {
        for n in 1..64 {
            assert_eq!(engine_sqrt((n * n) as f32), n as f32);
        }
        let root = engine_sqrt(2.0);
        assert!(root <= 2f32.sqrt() && root > 1.4141);
    }
}
