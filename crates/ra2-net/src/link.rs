//! Not original behaviour: the transport. The original spoke IPX / UDP with its own
//! retransmission; all the lockstep rules need is reliable, ordered, whole messages.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A reliable, ordered, message-framed byte transport to one other end.
pub trait Link {
    /// Queue one whole message. Never blocks for long; errors show up as `closed()`.
    fn send(&mut self, bytes: &[u8]);
    /// The next whole message that has arrived, if any. Never blocks.
    fn poll(&mut self) -> Option<Vec<u8>>;
    /// The other end is gone (messages already received can still be polled).
    fn closed(&self) -> bool;
}

/// A clock the caller moves by hand, in milliseconds. `MemoryLink` reads it to decide what
/// has "arrived"; nothing in this crate reads wall time.
#[derive(Clone, Default)]
pub struct Clock(Arc<AtomicU64>);

impl Clock {
    pub fn new() -> Clock {
        Clock::default()
    }
    pub fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

/// One direction of a `MemoryLink`.
#[derive(Default)]
struct Pipe {
    /// (time it may be delivered, message), in sending order.
    queue: VecDeque<(u64, Vec<u8>)>,
    /// Delivery time of the newest message: later ones never arrive before it (ordered).
    last: u64,
    delay: u64,
    jitter: u64,
    random: u64,
    /// Nothing is delivered while the clock is in [from, until).
    stall: Option<(u64, u64)>,
    closed: bool,
}

/// An in-process connected pair for tests, with optional scripted latency, jitter and stalls
/// driven by a `Clock`.
pub struct MemoryLink {
    out: Arc<Mutex<Pipe>>,
    back: Arc<Mutex<Pipe>>,
    clock: Clock,
}

impl MemoryLink {
    /// A pair with no delay.
    pub fn pair() -> (MemoryLink, MemoryLink) {
        MemoryLink::pair_on(&Clock::new())
    }

    /// A pair whose delays and stalls are measured on `clock`.
    pub fn pair_on(clock: &Clock) -> (MemoryLink, MemoryLink) {
        let a = Arc::new(Mutex::new(Pipe::default()));
        let b = Arc::new(Mutex::new(Pipe::default()));
        (
            MemoryLink {
                out: a.clone(),
                back: b.clone(),
                clock: clock.clone(),
            },
            MemoryLink {
                out: b,
                back: a,
                clock: clock.clone(),
            },
        )
    }

    /// Messages sent from this end from now on take `delay` ms plus a pseudo-random
    /// 0..=`jitter` ms (drawn from `seed`), without ever overtaking each other.
    pub fn set_delay(&self, delay: u64, jitter: u64, seed: u64) {
        let mut pipe = self.out.lock().unwrap();
        pipe.delay = delay;
        pipe.jitter = jitter;
        pipe.random = seed | 1;
    }

    /// Nothing sent from this end is delivered while the clock is in [from, until); it all
    /// arrives, in order, afterwards.
    pub fn script_stall(&self, from: u64, until: u64) {
        self.out.lock().unwrap().stall = Some((from, until));
    }

    pub fn close(&self) {
        self.out.lock().unwrap().closed = true;
        self.back.lock().unwrap().closed = true;
    }
}

impl Link for MemoryLink {
    fn send(&mut self, bytes: &[u8]) {
        let now = self.clock.now();
        let mut pipe = self.out.lock().unwrap();
        if pipe.closed {
            return;
        }
        let mut at = now + pipe.delay;
        if pipe.jitter > 0 {
            // xorshift64
            let mut x = pipe.random;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            pipe.random = x;
            at += x % (pipe.jitter + 1);
        }
        at = at.max(pipe.last);
        pipe.last = at;
        pipe.queue.push_back((at, bytes.to_vec()));
    }

    fn poll(&mut self) -> Option<Vec<u8>> {
        let now = self.clock.now();
        let mut pipe = self.back.lock().unwrap();
        if let Some((from, until)) = pipe.stall
            && now >= from
            && now < until
        {
            return None;
        }
        if pipe.queue.front()?.0 > now {
            return None;
        }
        pipe.queue.pop_front().map(|(_, bytes)| bytes)
    }

    fn closed(&self) -> bool {
        self.back.lock().unwrap().closed
    }
}

impl Drop for MemoryLink {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_and_ordered() {
        let (mut a, mut b) = MemoryLink::pair();
        a.send(b"one");
        a.send(b"two");
        b.send(b"back");
        assert_eq!(b.poll().as_deref(), Some(&b"one"[..]));
        assert_eq!(b.poll().as_deref(), Some(&b"two"[..]));
        assert_eq!(b.poll(), None);
        assert_eq!(a.poll().as_deref(), Some(&b"back"[..]));
        assert!(!a.closed());
        drop(b);
        assert!(a.closed());
    }

    #[test]
    fn delay_jitter_keep_order() {
        let clock = Clock::new();
        let (mut a, mut b) = MemoryLink::pair_on(&clock);
        a.set_delay(50, 40, 7);
        for i in 0..100u8 {
            a.send(&[i]);
            clock.advance(3);
        }
        // The first message was sent at 0, so nothing can have arrived before 50.
        let mut got = Vec::new();
        clock.set(49);
        assert_eq!(b.poll(), None);
        while got.len() < 100 {
            clock.advance(1);
            while let Some(m) = b.poll() {
                // Never earlier than the delay after it was sent.
                assert!(clock.now() >= u64::from(m[0]) * 3 + 50);
                got.push(m[0]);
            }
            assert!(clock.now() < 1000);
        }
        assert_eq!(got, (0..100).collect::<Vec<u8>>());
    }

    #[test]
    fn scripted_stall() {
        let clock = Clock::new();
        let (mut a, mut b) = MemoryLink::pair_on(&clock);
        a.script_stall(100, 200);
        a.send(b"x");
        assert!(b.poll().is_some());
        clock.set(100);
        a.send(b"y");
        assert_eq!(b.poll(), None);
        clock.set(199);
        assert_eq!(b.poll(), None);
        clock.set(200);
        assert!(b.poll().is_some());
    }
}
