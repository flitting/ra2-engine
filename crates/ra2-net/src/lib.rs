//! Lockstep network play: the original's timing, stall, sync and drop rules over a modern transport of our own.
//! This crate knows nothing about the simulation or Bevy. Orders are opaque byte strings and
//! the game is whatever implements [`Game`]. See the README for how a game loop drives a
//! [`Session`].
//! Comments mark `Not original behaviour:` (something of ours: transport, framing, the
//! relay star).

pub mod directory;
pub mod link;
pub mod msg;
pub mod relay;
pub mod rules;
pub mod session;
pub mod tcp;

pub use link::{Clock, Link, MemoryLink};
pub use msg::{Body, DecodeError, Msg, Order, PROTOCOL};
pub use relay::Relay;
pub use rules::NetKind;
pub use session::{Config, End, Notice, Session, Step};
pub use tcp::TcpLink;

/// The game as the session sees it. The integration implements this over the real World.
///
/// The session calls, for each frame and in this order: `check_value` (exactly once),
/// `apply` for every order due on the frame (house order, then issue order), `tick`.
pub trait Game {
    /// The frame about to be run. Starts at 0 and goes up by one per `tick`.
    fn frame(&self) -> u32;
    /// Carry out one order issued by `house`. The bytes are whatever was given to
    /// `Session::issue` on that house's machine. Must be deterministic, and must itself
    /// reject orders the house is not entitled to give.
    fn apply(&mut self, house: u8, order: &[u8]);
    /// Run one frame of the simulation.
    fn tick(&mut self);
    /// A hash of the state at the top of the current frame, before its orders are applied.
    /// Called exactly once per frame on every machine. What goes into it is the game's
    /// business.
    fn check_value(&self) -> u32;
    /// DESTRUCT and REMOVEPLAYER: the house resigns or was dropped; destroy it. Called at most once per house, on the same
    /// frame on every machine.
    fn destroy_house(&mut self, house: u8);
}
