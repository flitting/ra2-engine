# ra2-net

The lockstep core of network play. Std only: no `ra2-sim`, no Bevy, no async, no external
crates. Orders are opaque `Vec<u8>`; the game is whatever implements `Game`.

The timing, stall, sync and drop rules follow the original game's. The transport is ours.

## Pieces

| File | What |
|---|---|
| `link.rs` | `Link` (reliable, ordered, whole messages), `MemoryLink` pair with scripted delay / jitter / stall on a hand-moved `Clock` |
| `tcp.rs` | `TcpLink`: std TCP, length-prefixed frames, `set_nodelay`, a reader thread feeding a channel |
| `msg.rs` | `Msg` and `Order` with a hand-written binary codec |
| `rules.rs` | The original's numbers and formulas: start table, stamping, TIMING, time-outs |
| `relay.rs` | `Relay`: fans every message out to the other machines, sender's house in front |
| `session.rs` | `Session`: the per-machine state machine |

## Driving a session

```text
trait Game {
    fn frame(&self) -> u32;                      // frame about to run; +1 per tick
    fn apply(&mut self, house: u8, order: &[u8]);
    fn tick(&mut self);
    fn check_value(&self) -> u32;                // hash at the top of the frame
    fn destroy_house(&mut self, house: u8);      // DESTRUCT / REMOVEPLAYER
}

// Host only: one Relay in process. The host joins it through a MemoryLink like a client.
relay.add(house, Box::new(link_to_that_machine));

// Every machine, after Hello / Start / Loaded / Go (not driven by this crate):
let config = Config { local, players, kind, game_id, speed, max_ahead, frame_send_rate };
let mut session = Session::new(config, Box::new(link_to_relay), now_ms);

// Once per rendered frame:
relay.pump();                                    // host only
for order in input_this_frame { session.issue(encode(order)); }   // never World::order directly
match session.step(now_ms, &mut game) {
    Step::Idle => {}                             // next frame not due yet
    Step::Run => {}                              // one frame ran: apply()s then tick()
    Step::Wait { house, dialog, seconds_left } => {} // stalled; if dialog, show the waiting box
    Step::Dropped(house) => {}                   // "connection lost"; play goes on
    Step::OutOfSync { frame } => {}              // write the dump, tell the player, end
    Step::Ended(reason) => {}                    // Left / TimedOut / GameIdMismatch
}
session.note_process_ms(ms_spent_on_that_frame); // optional, feeds PROCESS_TIME
for notice in session.take_notices() {}          // speed / latency changed, left, late order...
```

`step` runs at most one frame per call and paces itself at `session.fps()`, the rate the host
publishes; keep drawing and taking input whatever it returns. Other calls: `set_latency_level`,
`set_game_speed`, `set_game_speed_cooled`, `quit` (DESTRUCT then EXIT), `resign`, `leave_now`,
`vote_drop`.

## The original's rules

- Stamping: an order sent on frame F runs on `roundup(F + MaxAhead, FrameSendRate)`; packets
  only on multiples of FrameSendRate; header = frame, check value, running order count, delay.
  Start values by speed. Out list of 128, silently dropping when full.
- Execution: house order, then issue order; an order whose frame has passed is skipped.
- Stall rule: every peer's announced orders in, and `Frame < lowest peer frame + MaxAhead`;
  nothing runs until every peer has been heard from.
- Adaptive timing: PROCESS_TIME every 128 frames; the host's TIMING every 256 frames once more
  than 4 orders are sent; the formula; applied on the order's frame; in-flight orders moved to
  the new boundary on a raise; shrinking immediate.
- Latency level and game speed orders by any player; the 900-tick cool-down.
- Sync: check value in every header, compared against a 256-entry history when the header's
  frame comes up, not in the first 256 frames; a mismatch ends the session.
- Time-outs: table per game type (LAN 7 s / 40 s, Internet 12 s / 120 s, 15 s / 30 s at the
  start), FRAMESYNC resend, drop of the lowest-frame peer, time-out on frame 0 ends the game,
  vote count (all but one, 3+ players), REMOVEPLAYER queued by every remaining machine,
  DESTRUCT then EXIT on quitting, game id check.

## Ours ("Not original behaviour:")

- The transport, the framing and every byte layout; the order count is u32 on the wire.
- The star through `Relay`, players identified by house number, and the TIMING host being the
  lowest-numbered connected player (so it passes on by itself).
- `Step::Idle` and the pacing arithmetic (steady cadence, no catching up after a stall).
- Response time measured with `Ping` / `Pong` once a second.
- When a house's EXIT runs, its orders stamped for later frames are discarded.

## Not done

- The handshake driver: `Hello`, `Refuse`, `Lobby`, `Start`, `Loaded`, `Go` exist and
  round-trip, but nothing sends or checks them yet. Chat. The WebSocket relay. Host
  migration (if the relay host goes, everyone times out). Saving network games.
- A closed link is not reported to the others: by the original's rules they stall and time
  out. The relay could say so sooner.
- A slow-but-alive peer dropped by time-out can still have packets in flight that one machine
  took and another did not (the original has the same hazard); the check value catches it.
- The latency level starting at 2 when lobby ping is over 1000 ms; the relay-server +20
  MaxAhead; the "+10 with Scenario flag 0x1000" in the TIMING payload; types 1 / 2 (modem)
  retry dialog; the do list size limit (error 3); RESPONSE_TIME.
- The original's check draws twice per frame from the game's random stream; that is the
  game side's to decide.
- One oddity kept: when TIMING shrinks MaxAhead by more than one FrameSendRate, an order sent
  just after the change runs before one sent just before it (same on every machine).
