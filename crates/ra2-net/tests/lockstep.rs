//! Whole sessions against each other: a toy game per machine, `MemoryLink`s through the
//! relay, and a clock moved by hand. Nothing here reads wall time or real randomness.

use ra2_net::rules::{self, NetKind};
use ra2_net::{Clock, Config, End, Game, MemoryLink, Notice, Relay, Session, Step};

/// Milliseconds the clock moves per pass of the test loop ("one rendered frame").
const STEP: u64 = 4;

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A stand-in simulation: a counter per house that orders stir, and a pseudo-random stream
/// advanced every tick. The check value is a hash of all of it.
#[derive(Clone)]
struct Toy {
    frame: u32,
    counters: [u32; 8],
    dead: [bool; 8],
    random: u64,
    /// (frame, house, order) for every order applied.
    log: Vec<(u32, u8, Vec<u8>)>,
    /// (frame, house) for every house destroyed.
    destroyed: Vec<(u32, u8)>,
    /// The check value at the top of every frame run.
    checks: Vec<u32>,
}

impl Toy {
    fn new() -> Toy {
        Toy {
            frame: 0,
            counters: [0; 8],
            dead: [false; 8],
            random: 0x9e37_79b9_7f4a_7c15,
            log: Vec::new(),
            destroyed: Vec::new(),
            checks: Vec::new(),
        }
    }
}

impl Game for Toy {
    fn frame(&self) -> u32 {
        self.frame
    }
    fn apply(&mut self, house: u8, order: &[u8]) {
        self.log.push((self.frame, house, order.to_vec()));
        let slot = &mut self.counters[house as usize % 8];
        for byte in order {
            // Order-sensitive on purpose: the same orders in another order give another value.
            *slot = slot.wrapping_mul(31).wrapping_add(u32::from(*byte) + 1);
        }
    }
    fn tick(&mut self) {
        self.checks.push(self.check_value());
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        let slot = (self.random % 8) as usize;
        if !self.dead[slot] {
            self.counters[slot] = self.counters[slot].wrapping_add(self.random as u32 >> 20);
        }
        self.frame += 1;
    }
    fn check_value(&self) -> u32 {
        let mut hash = 0x811c_9dc5u32 ^ self.frame;
        for (counter, dead) in self.counters.iter().zip(self.dead) {
            hash = (hash ^ counter ^ u32::from(dead))
                .wrapping_mul(0x0100_0193)
                .rotate_left(5);
        }
        hash ^ (self.random as u32) ^ ((self.random >> 32) as u32)
    }
    fn destroy_house(&mut self, house: u8) {
        self.destroyed.push((self.frame, house));
        self.dead[house as usize % 8] = true;
        self.counters[house as usize % 8] = 0;
    }
}

/// One long stall as a seat saw it, ending in a drop.
#[derive(Clone, Debug)]
struct Drop {
    house: u8,
    stall_start: u64,
    dialog_at: Option<u64>,
    /// `seconds_left` on the first call that had the dialog up.
    dialog_left: Option<u32>,
    at: u64,
    frame: u32,
}

struct Seat {
    house: u8,
    session: Session,
    game: Toy,
    random: Random,
    /// Orders issued so far; also the sequence number put in each.
    issued: u32,
    /// This machine has frozen: it is no longer stepped.
    stopped: bool,
    runs: u32,
    waits: u32,
    stall_start: Option<u64>,
    dialog_at: Option<u64>,
    dialog_left: Option<u32>,
    waited_for: Option<u8>,
    drops: Vec<Drop>,
    out_of_sync: Option<u32>,
    ended: Option<End>,
    notices: Vec<Notice>,
}

struct Net {
    clock: Clock,
    relay: Relay,
    seats: Vec<Seat>,
    /// Chance in 1000, per seat and pass, of issuing an order.
    order_rate: u64,
    /// Chance in 1000, per seat and pass, that the machine misses the pass (uneven frames).
    skip_rate: u64,
}

impl Net {
    /// `shape(seat)` gives (delay, jitter) in ms for each direction of that seat's link.
    fn new(
        houses: &[u8],
        kind: NetKind,
        speed: u8,
        seed: u64,
        shape: impl Fn(usize) -> (u64, u64),
    ) -> Net {
        Net::with_stall(houses, kind, speed, seed, shape, None)
    }

    /// The same, with seat `stall.0`'s link delivering nothing in either direction while
    /// the clock is in [stall.1, stall.2).
    fn with_stall(
        houses: &[u8],
        kind: NetKind,
        speed: u8,
        seed: u64,
        shape: impl Fn(usize) -> (u64, u64),
        stall: Option<(usize, u64, u64)>,
    ) -> Net {
        let clock = Clock::new();
        let mut relay = Relay::new();
        let mut seats = Vec::new();
        for (index, &house) in houses.iter().enumerate() {
            let (near, far) = MemoryLink::pair_on(&clock);
            let (delay, jitter) = shape(index);
            near.set_delay(delay, jitter, seed + index as u64 * 2 + 1);
            far.set_delay(delay, jitter, seed + index as u64 * 2 + 2);
            if let Some((seat, from, until)) = stall
                && seat == index
            {
                near.script_stall(from, until);
                far.script_stall(from, until);
            }
            relay.add(house, Box::new(far));
            let config = Config::new(house, houses, kind, speed, 0x5eed);
            seats.push(Seat {
                house,
                session: Session::new(config, Box::new(near), clock.now()),
                game: Toy::new(),
                random: Random(seed * 77 + index as u64 + 1),
                issued: 0,
                stopped: false,
                runs: 0,
                waits: 0,
                stall_start: None,
                dialog_at: None,
                dialog_left: None,
                waited_for: None,
                drops: Vec::new(),
                out_of_sync: None,
                ended: None,
                notices: Vec::new(),
            });
        }
        Net {
            clock,
            relay,
            seats,
            order_rate: 60,
            skip_rate: 0,
        }
    }

    fn now(&self) -> u64 {
        self.clock.now()
    }

    /// One pass: move the clock, forward, and step every machine once.
    fn pass(&mut self) {
        self.clock.advance(STEP);
        let now = self.clock.now();
        self.relay.pump();
        for seat in &mut self.seats {
            if seat.stopped || seat.random.below(1000) < self.skip_rate {
                continue;
            }
            if seat.random.below(1000) < self.order_rate
                && seat.out_of_sync.is_none()
                && seat.ended.is_none()
            {
                seat.issue();
            }
            match seat.session.step(now, &mut seat.game) {
                Step::Idle => {}
                Step::Run => {
                    seat.runs += 1;
                    seat.stall_start = None;
                    seat.dialog_at = None;
                    seat.dialog_left = None;
                }
                Step::Wait {
                    house,
                    dialog,
                    seconds_left,
                } => {
                    seat.waits += 1;
                    seat.waited_for = Some(house);
                    seat.stall_start.get_or_insert(now);
                    if dialog && seat.dialog_at.is_none() {
                        seat.dialog_at = Some(now);
                        seat.dialog_left = Some(seconds_left);
                    }
                }
                Step::Dropped(house) => seat.drops.push(Drop {
                    house,
                    stall_start: seat.stall_start.unwrap_or(now),
                    dialog_at: seat.dialog_at,
                    dialog_left: seat.dialog_left,
                    at: now,
                    frame: seat.game.frame,
                }),
                Step::OutOfSync { frame } => {
                    seat.out_of_sync.get_or_insert(frame);
                }
                Step::Ended(end) => {
                    seat.ended.get_or_insert(end);
                }
            }
            seat.notices.extend(seat.session.take_notices());
        }
    }

    fn run(&mut self, ms: u64) {
        for _ in 0..ms / STEP {
            self.pass();
        }
    }

    /// The machines still playing.
    fn live(&self) -> Vec<&Seat> {
        self.seats
            .iter()
            .filter(|s| !s.stopped && s.ended.is_none())
            .collect()
    }

    /// Every live machine has the same check value on every frame they have all run, and
    /// applied the same orders on the same frames in the same order. Returns the number of
    /// frames compared.
    fn assert_identical(&self) -> u32 {
        let live = self.live();
        let frames = live.iter().map(|s| s.game.checks.len()).min().unwrap();
        let first = live[0];
        for seat in &live[1..] {
            for frame in 0..frames {
                assert_eq!(
                    first.game.checks[frame], seat.game.checks[frame],
                    "check value differs on frame {frame}: houses {} and {}",
                    first.house, seat.house
                );
            }
            let cut = |s: &Seat| -> Vec<(u32, u8, Vec<u8>)> {
                s.game
                    .log
                    .iter()
                    .filter(|e| (e.0 as usize) < frames)
                    .cloned()
                    .collect()
            };
            assert_eq!(cut(first), cut(seat), "order logs differ");
            let timing = |s: &Seat| -> Vec<Notice> {
                s.notices
                    .iter()
                    .filter(
                        |n| matches!(n, Notice::Timing { frame, .. } if (*frame as usize) < frames),
                    )
                    .cloned()
                    .collect()
            };
            assert_eq!(timing(first), timing(seat), "TIMING ran differently");
        }
        frames as u32
    }

    fn timings(&self, seat: usize) -> Vec<(u32, u32, u32, u32)> {
        self.seats[seat]
            .notices
            .iter()
            .filter_map(|n| match n {
                Notice::Timing {
                    frame,
                    fps,
                    max_ahead,
                    frame_send_rate,
                } => Some((*frame, *fps, *max_ahead, *frame_send_rate)),
                _ => None,
            })
            .collect()
    }
}

impl Seat {
    /// Issue one order. Its bytes: sequence number, the frame the stamping rule says it
    /// must run on (worked out here, at issue time), and a random byte. `None` when the
    /// out list is full (a long stall) and the order was dropped, as in the original.
    fn issue(&mut self) -> Option<Vec<u8>> {
        // The order goes out with the next packet: this frame's if it has not been sent
        // yet, else the following send frame's.
        let rate = self.session.frame_send_rate();
        let next = self.game.frame + u32::from(self.session.waiting());
        let send = rules::roundup(next, rate);
        let expected = rules::stamp(send, self.session.max_ahead(), rate);
        let mut order = self.issued.to_le_bytes().to_vec();
        order.extend_from_slice(&expected.to_le_bytes());
        order.push(self.random.below(256) as u8);
        if !self.session.issue(order.clone()) {
            return None;
        }
        self.issued += 1;
        Some(order)
    }

    fn lates(&self) -> Vec<Notice> {
        self.notices
            .iter()
            .filter(|n| matches!(n, Notice::Late { .. }))
            .cloned()
            .collect()
    }
}

fn sequence(order: &[u8]) -> u32 {
    u32::from_le_bytes(order[0..4].try_into().unwrap())
}

fn expected_frame(order: &[u8]) -> u32 {
    u32::from_le_bytes(order[4..8].try_into().unwrap())
}

/// Within each house, orders ran in the order they were issued and none was lost.
fn assert_in_issue_order(seat: &Seat) {
    let mut next = [0u32; 256];
    for (frame, house, order) in &seat.game.log {
        assert_eq!(
            sequence(order),
            next[*house as usize],
            "house {house} order out of turn on frame {frame}"
        );
        next[*house as usize] += 1;
    }
}

/// Every order of every house ran exactly once (in whatever order).
fn assert_none_lost(seat: &Seat) {
    let mut seen: Vec<(u8, u32)> = seat
        .game
        .log
        .iter()
        .map(|(_, house, order)| (*house, sequence(order)))
        .collect();
    seen.sort_unstable();
    let mut next = [0u32; 256];
    for (house, number) in seen {
        assert_eq!(number, next[house as usize], "house {house}");
        next[house as usize] += 1;
    }
}

// ---- determinism ----------------------------------------------------------------------

fn random_game(
    houses: &[u8],
    kind: NetKind,
    speed: u8,
    seed: u64,
    ms: u64,
    least: u32,
    in_order: bool,
) {
    let mut shapes = Random(seed);
    let shapes: Vec<(u64, u64)> = houses
        .iter()
        .map(|_| (5 + shapes.below(55), shapes.below(30)))
        .collect();
    let mut net = Net::new(houses, kind, speed, seed, |i| shapes[i]);
    net.skip_rate = 120;
    net.run(ms);

    let frames = net.assert_identical();
    assert!(frames >= least, "only {frames} frames in {ms} ms");
    for seat in &net.seats {
        assert_eq!(seat.out_of_sync, None);
        assert_eq!(seat.ended, None);
        assert!(seat.drops.is_empty());
        assert_eq!(seat.lates(), []);
        assert_none_lost(seat);
        if in_order {
            assert_in_issue_order(seat);
        }
        // Nobody ever gets MaxAhead or more ahead of anybody else.
        assert!(seat.game.frame < frames + 250);
    }
    // Everybody's orders ran, and plenty of them.
    let ran = &net.seats[0].game.log;
    assert!(ran.len() > 100 * houses.len(), "{} orders", ran.len());
    for house in houses {
        assert!(ran.iter().any(|e| e.1 == *house));
    }
    // The host set the pace at least once, on the same frame everywhere (checked above).
    assert!(!net.timings(0).is_empty());
    // With latency and jitter the machines do stall on each other now and then.
    assert!(net.seats.iter().any(|s| s.waits > 0));
}

#[test]
fn two_machines_stay_identical() {
    random_game(&[1, 4], NetKind::Lan, 0, 11, 30_000, 1200, true);
}

#[test]
fn three_machines_stay_identical() {
    // Internet rules at a slower speed: FrameSendRate 5, MaxAhead 30, 30 frames a second.
    // The first TIMING shrinks MaxAhead from 30 to 15 at once. "Shrinking is immediate, no
    // window", so an order sent just after the change runs before
    // one sent just before it: the original's behaviour, the same on every machine. Hence
    // no issue-order check here.
    random_game(&[2, 3, 7], NetKind::Internet, 2, 22, 40_000, 800, false);
}

#[test]
fn four_machines_stay_identical() {
    random_game(&[0, 1, 5, 6], NetKind::Lan, 0, 33, 30_000, 1200, true);
}

#[test]
fn four_machines_many_seeds() {
    for seed in 100..110 {
        random_game(&[1, 2, 3, 4], NetKind::Lan, 1, seed, 12_000, 300, true);
    }
}

#[test]
fn a_link_that_stalls_for_two_seconds_only_makes_the_others_wait() {
    let stall = Some((2, 5_000, 7_000));
    let mut net = Net::with_stall(&[1, 2, 3], NetKind::Lan, 0, 5, |_| (10, 5), stall);
    net.run(12_000);
    net.assert_identical();
    for seat in &net.seats {
        assert!(
            seat.waits > 100,
            "house {} waited {} passes",
            seat.house,
            seat.waits
        );
        assert!(seat.drops.is_empty());
        assert!(seat.game.frame > 400);
        assert_eq!(seat.lates(), []);
    }
}

// ---- stamping -------------------------------------------------------------------------

#[test]
fn an_order_runs_on_the_right_boundary_everywhere() {
    let mut net = Net::new(&[1, 2], NetKind::Lan, 0, 3, |_| (15, 10));
    net.order_rate = 150;
    // Under 256 frames, so no TIMING changes MaxAhead between issue and send.
    net.run(4_000);
    let frames = net.assert_identical();
    assert!(frames > 150 && frames < 256);
    assert!(net.timings(0).is_empty());
    for seat in &net.seats {
        assert!(seat.game.log.len() > 50);
        for (frame, _, order) in &seat.game.log {
            assert_eq!(*frame, expected_frame(order));
            assert_eq!(frame % 10, 0);
        }
        assert_in_issue_order(seat);
    }

    // And spelled out once: issued on frame F with nothing in the way, it runs on
    // roundup(F + MaxAhead, FrameSendRate) = roundup(F + 40, 10).
    while net.seats[0].session.waiting() || net.seats[0].game.frame % 10 != 3 {
        net.order_rate = 0;
        net.pass();
    }
    let issued_on = net.seats[0].game.frame;
    let order = net.seats[0].issue().unwrap();
    net.run(2_000);
    for seat in &net.seats {
        let ran: Vec<u32> = seat
            .game
            .log
            .iter()
            .filter(|e| e.2 == order)
            .map(|e| e.0)
            .collect();
        assert_eq!(ran, [rules::roundup(issued_on + 40, 10)]);
    }
}

#[test]
fn an_order_that_arrives_late_is_skipped_everywhere() {
    let mut net = Net::new(&[1, 2, 3], NetKind::Lan, 0, 9, |i| (10 + 10 * i as u64, 8));
    net.run(6_000);
    let marker = vec![0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0x55, 0x55];
    // Stamped for a frame that every machine has long passed.
    net.seats[1].session.issue_stamped(marker.clone(), 7);
    net.run(4_000);
    net.assert_identical();
    for seat in &net.seats {
        assert_eq!(seat.lates(), [Notice::Late { house: 2, frame: 7 }]);
        assert!(seat.game.log.iter().all(|e| e.2 != marker));
        assert_eq!(seat.out_of_sync, None);
    }
}

// ---- stalls and drops -----------------------------------------------------------------

#[test]
fn a_silent_peer_is_waited_for_then_dropped_and_its_house_destroyed() {
    for (kind, dialog, hard) in [
        (NetKind::Lan, 7_000, 40_000),
        (NetKind::Internet, 12_000, 120_000),
    ] {
        let mut net = Net::new(&[1, 2, 3], kind, 0, 4, |_| (10, 5));
        net.run(5_000);
        let frozen_at = net.seats[2].game.frame;
        assert!(frozen_at > 200);
        net.seats[2].stopped = true;
        net.run(hard + 6_000);

        let mut destroyed = Vec::new();
        for seat in &net.seats[..2] {
            assert_eq!(seat.drops.len(), 1, "house {}", seat.house);
            let drop = &seat.drops[0];
            assert_eq!(drop.house, 3);
            assert_eq!(seat.waited_for, Some(3));
            // The waiting state comes up after 7 s (12 s), showing the seconds to the drop.
            let dialog_after = drop.dialog_at.unwrap() - drop.stall_start;
            assert!(
                (dialog..dialog + STEP).contains(&dialog_after),
                "{dialog_after}"
            );
            assert_eq!(
                u64::from(drop.dialog_left.unwrap()),
                (hard - dialog_after).div_ceil(1000)
            );
            // The drop after 40 s (120 s).
            let dropped_after = drop.at - drop.stall_start;
            assert!(
                (hard..hard + STEP).contains(&dropped_after),
                "{dropped_after}"
            );
            // Nobody ran MaxAhead or more past the last frame the silent peer reported.
            assert!(drop.frame < frozen_at + 40);
            assert!(seat.notices.contains(&Notice::ConnectionLost { house: 3 }));
            assert_eq!(seat.session.peers().len(), 1);

            // The REMOVEPLAYER queued during the stall goes out with the next packet and
            // runs MaxAhead later.
            let rate = seat.session.frame_send_rate();
            let expected = rules::stamp(
                rules::roundup(drop.frame + 1, rate),
                seat.session.max_ahead(),
                rate,
            );
            assert_eq!(seat.game.destroyed, [(expected, 3)]);
            destroyed.push(seat.game.destroyed.clone());
            // The game went on.
            assert!(seat.game.frame > expected + 100);
            assert_eq!(seat.lates(), []);
        }
        assert_eq!(destroyed[0], destroyed[1]);
        net.assert_identical();
    }
}

#[test]
fn two_players_the_survivor_plays_on_alone() {
    let mut net = Net::new(&[1, 2], NetKind::Lan, 0, 6, |_| (10, 5));
    net.run(5_000);
    net.seats[0].stopped = true;
    net.run(43_000);
    let seat = &net.seats[1];
    assert_eq!(seat.drops.len(), 1);
    assert_eq!(seat.drops[0].house, 1);
    assert!(seat.session.peers().is_empty());
    assert!(seat.session.is_host());
    assert_eq!(seat.game.destroyed.len(), 1);
    assert_eq!(seat.game.destroyed[0].1, 1);
    let before = seat.game.frame;
    net.run(2_000);
    assert!(net.seats[1].game.frame > before + 100);
    // A vote needs three players.
    assert!(!net.seats[1].session.vote_drop(1));
}

#[test]
fn a_time_out_on_frame_zero_ends_the_game() {
    let mut net = Net::new(&[1, 2], NetKind::Lan, 0, 8, |_| (10, 5));
    net.seats[1].stopped = true;
    net.run(14_996);
    let seat = &net.seats[0];
    assert!(seat.waits > 0 && seat.dialog_at.is_none());
    net.run(8);
    // At the start the waiting state comes after 15 s.
    assert!(net.seats[0].dialog_at.is_some());
    net.run(24_000);
    assert_eq!(net.seats[0].ended, None);
    net.run(1_100);
    let seat = &net.seats[0];
    assert_eq!(seat.ended, Some(End::TimedOut));
    assert_eq!(seat.game.frame, 0);
    assert!(seat.drops.is_empty());
    assert_eq!(seat.runs, 0);
}

#[test]
fn votes_from_all_but_one_drop_a_player_at_once() {
    let mut net = Net::new(&[1, 2, 3, 4], NetKind::Lan, 0, 12, |_| (10, 5));
    net.run(4_000);
    net.seats[3].stopped = true;
    net.run(3_000);
    // No vote before the waiting state is up.
    assert!(!net.seats[0].session.vote_drop(4));
    net.run(6_000);
    assert!(net.seats[..3].iter().all(|s| s.dialog_at.is_some()));
    assert!(net.seats[0].session.vote_drop(4));
    assert!(net.seats[1].session.vote_drop(4));
    net.run(1_000);
    // Two votes of four players are not enough.
    assert!(net.seats[..3].iter().all(|s| s.drops.is_empty()));
    assert!(net.seats[2].session.vote_drop(4));
    net.run(3_000);
    let mut destroyed = Vec::new();
    for seat in &net.seats[..3] {
        assert_eq!(seat.drops.len(), 1);
        assert_eq!(seat.drops[0].house, 4);
        // Long before the 40 s time-out.
        assert!(seat.drops[0].at - seat.drops[0].stall_start < 12_000);
        assert_eq!(seat.game.destroyed.len(), 1);
        destroyed.push(seat.game.destroyed[0]);
    }
    assert!(destroyed.iter().all(|d| *d == destroyed[0] && d.1 == 4));
    net.assert_identical();
}

#[test]
fn quitting_destroys_the_house_and_leaves() {
    let mut net = Net::new(&[1, 2, 3], NetKind::Lan, 0, 14, |_| (10, 5));
    net.run(3_000);
    net.seats[1].session.quit();
    net.run(3_000);
    assert_eq!(net.seats[1].ended, Some(End::Left));
    let left_on = net.seats[1].game.frame;
    assert_eq!(net.seats[1].game.destroyed, [(left_on, 2)]);
    for seat in [&net.seats[0], &net.seats[2]] {
        assert_eq!(seat.game.destroyed, [(left_on, 2)]);
        assert!(seat.notices.contains(&Notice::Left { house: 2 }));
        assert_eq!(seat.session.peers().len(), 1);
        assert!(seat.drops.is_empty());
        assert!(seat.game.frame > left_on + 100);
        // Nothing of the leaver's ran after its EXIT.
        assert!(seat.game.log.iter().all(|e| e.1 != 2 || e.0 <= left_on));
    }
    net.assert_identical();
}

// ---- sync -----------------------------------------------------------------------------

#[test]
fn a_corrupted_game_is_out_of_sync_everywhere_on_the_same_frame() {
    let mut net = Net::new(&[1, 2, 3], NetKind::Lan, 0, 21, |i| {
        (10 + 15 * i as u64, 10)
    });
    net.run(8_000);
    assert!(
        net.seats
            .iter()
            .all(|s| s.game.frame > 300 && s.out_of_sync.is_none())
    );
    let corrupted_on = net.seats[1].game.frame;
    net.seats[1].game.counters[5] ^= 1;
    net.run(4_000);
    let frame = net.seats[0].out_of_sync.expect("not detected");
    // The first check value sent after the damage: the next FrameSendRate boundary.
    assert_eq!(
        frame,
        rules::roundup(corrupted_on, net.seats[0].session.frame_send_rate())
    );
    for seat in &mut net.seats {
        assert_eq!(seat.out_of_sync, Some(frame));
        // Found when the header's frame came up, MaxAhead later; that frame never ran.
        assert_eq!(seat.game.frame, frame + seat.session.max_ahead());
        // It stays over.
        assert_eq!(
            seat.session.step(1 << 40, &mut seat.game),
            Step::OutOfSync { frame }
        );
        assert_eq!(seat.ended, None);
    }
}

#[test]
fn nothing_is_compared_in_the_first_256_frames() {
    let mut net = Net::new(&[1, 2], NetKind::Lan, 0, 23, |_| (10, 5));
    net.run(1_000);
    assert!(net.seats[0].game.frame < 100);
    net.seats[0].game.counters[0] ^= 0x8000;
    net.run(6_000);
    for seat in &net.seats {
        // The first header whose frame is 256 or later is the one for frame 260, sent on
        // frame 220 with MaxAhead 40.
        assert_eq!(seat.out_of_sync, Some(220));
        assert_eq!(seat.game.frame, 260);
    }
}

// ---- timing ---------------------------------------------------------------------------

#[test]
fn raising_latency_raises_max_ahead_by_the_rule_on_the_same_frame_everywhere() {
    let mut net = Net::new(&[1, 2, 3], NetKind::Lan, 0, 31, |_| (20, 5));
    net.order_rate = 120;
    net.run(6_000);
    // The first TIMING, computed on frame 256, sent on 260, run on 300: with a round trip
    // of about 100 ms and latency level 1 the formula gives the minimum, 3 x FrameSendRate.
    assert_eq!(net.timings(0), [(300, 60, 30, 10)]);
    for seat in &net.seats {
        assert_eq!(seat.session.max_ahead(), 30);
        assert_eq!(seat.session.latency_level(), 1);
    }

    // Any player may change the latency level, not only the host...
    let now = net.now();
    assert!(!net.seats[2].session.is_host());
    assert!(net.seats[2].session.set_latency_level(now, 3));
    // ...but only once in 15 seconds, and the speed slider of the same dialog shares that.
    assert!(!net.seats[2].session.set_latency_level(now, 0));
    assert!(!net.seats[2].session.set_game_speed_cooled(now, 4));
    net.run(2_000);
    for seat in &net.seats {
        assert_eq!(seat.session.latency_level(), 3);
        assert!(
            seat.notices
                .contains(&Notice::LatencyChanged { house: 3, level: 3 })
        );
    }
    // The others saw the change and are held off as well.
    let now = net.now();
    assert!(!net.seats[0].session.set_latency_level(now, 0));
    net.run(12_900);
    let now = net.now();
    assert!(!net.seats[2].session.set_latency_level(now, 0));
    net.run(200);
    let now = net.now();
    assert!(net.seats[2].session.set_latency_level(now, 3));

    net.run(4_000);
    let frames = net.assert_identical();
    let timings = net.timings(0);
    // The level took effect at the host's next computation (frame 512, sent 520, run on
    // 520 + 30): x3 and +30 frames wants about 40, and one FrameSendRate step is allowed.
    assert_eq!(timings[1], (550, 60, 40, 10));
    // It never grows by more than one step, is always a multiple of FrameSendRate and
    // never under three times it.
    let mut ahead = 40;
    for (_, _, next, rate) in &timings {
        assert!(*next <= ahead + rate && next % rate == 0 && *next >= 3 * rate);
        ahead = *next;
    }
    for seat in &net.seats {
        assert_eq!(seat.session.max_ahead(), 40);
        // No order in flight across the change was overtaken, lost or late.
        assert_in_issue_order(seat);
        assert_eq!(seat.lates(), []);
        assert!(seat.game.log.iter().any(|e| e.0 > 550));
        // Orders that were stamped for 560..=580 under the old MaxAhead were moved to the
        // new boundary, 550 + 40.
        assert!(seat.game.log.iter().all(|e| e.0 <= 550 || e.0 >= 590));
        assert!(
            seat.game
                .log
                .iter()
                .any(|e| e.0 == 590 && expected_frame(&e.2) < 590)
        );
    }
    assert!(frames > 1200);
}

#[test]
fn any_player_may_change_the_game_speed_and_the_pace_follows_the_host() {
    let mut net = Net::new(&[1, 2], NetKind::Lan, 0, 41, |_| (10, 5));
    net.run(3_000);
    assert!(net.seats[1].session.set_game_speed(3));
    net.run(1_500);
    for seat in &net.seats {
        assert_eq!(seat.session.speed(), 3);
        assert!(
            seat.notices
                .contains(&Notice::SpeedChanged { house: 2, speed: 3 })
        );
        // Not yet: the pace changes with the host's next TIMING.
        assert_eq!(seat.session.fps(), 60);
    }
    net.run(4_000);
    let before = net.seats[0].game.frame;
    assert!(net.seats.iter().all(|s| s.session.fps() == 20));
    net.run(5_000);
    // 20 frames a second now.
    let ran = net.seats[0].game.frame - before;
    assert!((90..=100).contains(&ran), "{ran}");
    net.assert_identical();
}
