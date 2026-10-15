//! Not original behaviour: the star. The original talks peer to peer (IPX broadcast, or UDP
//! with a host that can migrate). Here every machine holds one `Link` to a forwarder, which
//! copies each message to all the other machines with the sender's house number put in front.
//! Each `Session` therefore sees every other peer's packets exactly as it would in a mesh,
//! and the lockstep rules do not know the difference.
//!
//! Framing on a session's link:
//! - session -> relay: the encoded `Msg`;
//! - relay -> session: one byte, the sending house, then the encoded `Msg`.
//!
//! The relay never looks inside a message. The host runs one in its own process and joins
//! it through a `MemoryLink` like everybody else; a stand-alone relay server is the same
//! loop.

use crate::link::Link;

#[derive(Default)]
pub struct Relay {
    seats: Vec<(u8, Box<dyn Link>)>,
}

impl Relay {
    pub fn new() -> Relay {
        Relay::default()
    }

    /// Attach the link that leads to the machine playing `house`.
    pub fn add(&mut self, house: u8, link: Box<dyn Link>) {
        self.seats.retain(|(h, _)| *h != house);
        self.seats.push((house, link));
    }

    /// Detach a house (its link is dropped).
    pub fn remove(&mut self, house: u8) {
        self.seats.retain(|(h, _)| *h != house);
    }

    pub fn houses(&self) -> Vec<u8> {
        self.seats.iter().map(|(h, _)| *h).collect()
    }

    /// Forward everything that has arrived. Call it often (every rendered frame on a host,
    /// a short sleep loop on a server). Returns the number of messages forwarded.
    ///
    /// A closed link is detached once it has been read dry. The other machines are not told:
    /// by the original's rules they find out by stalling and timing out.
    pub fn pump(&mut self) -> usize {
        let mut forwarded = 0;
        for from in 0..self.seats.len() {
            while let Some(message) = self.seats[from].1.poll() {
                let house = self.seats[from].0;
                let mut framed = Vec::with_capacity(message.len() + 1);
                framed.push(house);
                framed.extend_from_slice(&message);
                for (to, seat) in self.seats.iter_mut().enumerate() {
                    if to != from {
                        seat.1.send(&framed);
                    }
                }
                forwarded += 1;
            }
        }
        self.seats.retain(|(_, link)| !link.closed());
        forwarded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::MemoryLink;

    #[test]
    fn fans_out_with_the_sender_in_front() {
        let mut relay = Relay::new();
        let mut ends = Vec::new();
        for house in [2u8, 5, 6] {
            let (near, far) = MemoryLink::pair();
            relay.add(house, Box::new(far));
            ends.push(near);
        }
        ends[0].send(b"a");
        ends[2].send(b"c1");
        ends[2].send(b"c2");
        assert_eq!(relay.pump(), 3);
        assert_eq!(ends[0].poll().unwrap(), b"\x06c1");
        assert_eq!(ends[0].poll().unwrap(), b"\x06c2");
        assert_eq!(ends[0].poll(), None);
        assert_eq!(ends[1].poll().unwrap(), b"\x02a");
        assert_eq!(ends[1].poll().unwrap(), b"\x06c1");
        assert_eq!(ends[2].poll().unwrap(), b"\x02a");
        assert_eq!(ends[2].poll(), None);
        // A machine that goes away is detached.
        ends.pop();
        relay.pump();
        assert_eq!(relay.houses(), [2, 5]);
    }
}
