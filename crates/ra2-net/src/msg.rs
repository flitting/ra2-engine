//! Messages and their binary form.
//!
//! Not original behaviour: the framing and every byte layout here are ours (the original
//! packs 0x6f-byte event records, optionally compressed). What is the original's is the
//! *content* of the in-game stream: a header with frame, check value, running order count and
//! the delay used, followed by orders that each carry their execution frame and house, and the
//! timing / leave orders travelling inside that same stream as ordinary events.
//!
//! All integers are little endian. A message is one tag byte followed by its fields.

/// Bumped whenever a layout in this file changes. Checked in `Hello`.
pub const PROTOCOL: u16 = 1;

/// The original's event type
/// numbers, used as the order tags on the wire.
pub mod event {
    pub const DESTRUCT: u8 = 0x08;
    pub const GAMESPEED: u8 = 0x0d;
    pub const EXIT: u8 = 0x13;
    pub const FRAMESYNC: u8 = 0x19;
    pub const FRAMEINFO: u8 = 0x1c;
    pub const TIMING: u8 = 0x20;
    pub const PROCESS_TIME: u8 = 0x21;
    pub const REMOVEPLAYER: u8 = 0x23;
    pub const LATENCYFUDGE: u8 = 0x24;
    /// Not original behaviour: every game order (move, build, ...) is one opaque blob to
    /// this crate, under a tag the original does not use.
    pub const GAME: u8 = 0xff;
}

/// What an order does. Everything but `Game` is handled by the session itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Body {
    /// An order for the simulation; opaque here.
    Game(Vec<u8>),
    /// PROCESS_TIME: the sender's average processing time per frame.
    ProcessTime(u16),
    /// TIMING: the host's new pace. Fps u16, MaxAhead u16,
    /// FrameSendRate u8.
    Timing {
        fps: u16,
        max_ahead: u16,
        frame_send_rate: u8,
    },
    /// LATENCYFUDGE: the new latency level, 0..3.
    LatencyFudge(u8),
    /// GAMESPEED: the new game speed, 0 (fastest) .. 6.
    GameSpeed(u8),
    /// DESTRUCT: the sender's house is destroyed (resign).
    Destruct,
    /// EXIT: the sender leaves.
    Exit,
    /// REMOVEPLAYER: destroy the named house (queued by everyone left after a drop).
    RemovePlayer(u8),
}

/// One order as it travels: type, house and
/// execution frame, then the payload.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Order {
    pub frame: u32,
    pub house: u8,
    pub body: Body,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Msg {
    /// First message on a new connection. A different protocol or build is refused.
    Hello {
        protocol: u16,
        build: String,
        name: String,
    },
    /// The host will not let this peer in (or start): a code for the program, text for the
    /// player. The codes are the caller's.
    Refuse {
        code: u8,
        text: String,
    },
    /// Lobby state (slots, options, ready flags): an opaque blob owned by the game side.
    Lobby {
        setup: Vec<u8>,
    },
    /// The host seals the lobby. `setup` is the opaque match setup (seed, slots, options).
    /// `internet` picks the Internet time-outs and send-rate rule instead of the LAN ones.
    Start {
        setup: Vec<u8>,
        game_id: u32,
        internet: bool,
        speed: u8,
        max_ahead: u16,
        frame_send_rate: u8,
    },
    /// A peer has built its world; the hash of it at tick 0.
    Loaded {
        setup_hash: u64,
    },
    /// Every `Loaded` matched: frame 0 may begin.
    Go,
    /// The in-game packet, sent on every FrameSendRate boundary.
    /// The FRAMEINFO header is
    /// frame = roundup(Frame + MaxAhead, FrameSendRate), the sender's check value for the
    /// frame it is on, its running count of orders sent, and the delay used.
    /// Not original behaviour: the count is u32 here (u16 in the original).
    Packet {
        frame: u32,
        check: u32,
        sent: u32,
        delay: u8,
        orders: Vec<Order>,
    },
    /// Resent on a timer while stalled:
    /// frame, order count, game id; no orders.
    FrameSync {
        frame: u32,
        sent: u32,
        game_id: u32,
    },
    /// Not original behaviour: response-time measurement. Every peer answers a `Ping` with a
    /// `Pong` naming the house that asked and echoing its stamp.
    Ping {
        stamp: u32,
    },
    Pong {
        to: u8,
        stamp: u32,
    },
    /// The original's kick-vote packet flow is "Not determined"; only the counting
    /// rule is known.
    Vote {
        target: u8,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    /// Ran out of bytes.
    Short,
    /// Unknown message or order tag.
    Tag(u8),
    /// Bytes left over after a whole message.
    Trailing,
    /// A string was not UTF-8.
    Text,
}

const HELLO: u8 = 1;
const REFUSE: u8 = 2;
const LOBBY: u8 = 3;
const START: u8 = 4;
const LOADED: u8 = 5;
const GO: u8 = 6;
const PING: u8 = 7;
const PONG: u8 = 8;
const VOTE: u8 = 9;

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// Length (u32) then the bytes.
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    fn text(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], DecodeError> {
        if self.0.len() < count {
            return Err(DecodeError::Short);
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let length = self.u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }
    fn text(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.bytes()?).map_err(|_| DecodeError::Text)
    }
}

impl Order {
    fn write(&self, w: &mut Writer) {
        let tag = match &self.body {
            Body::Game(_) => event::GAME,
            Body::ProcessTime(_) => event::PROCESS_TIME,
            Body::Timing { .. } => event::TIMING,
            Body::LatencyFudge(_) => event::LATENCYFUDGE,
            Body::GameSpeed(_) => event::GAMESPEED,
            Body::Destruct => event::DESTRUCT,
            Body::Exit => event::EXIT,
            Body::RemovePlayer(_) => event::REMOVEPLAYER,
        };
        w.u8(tag);
        w.u8(self.house);
        w.u32(self.frame);
        match &self.body {
            Body::Game(bytes) => w.bytes(bytes),
            Body::ProcessTime(v) => w.u16(*v),
            Body::Timing {
                fps,
                max_ahead,
                frame_send_rate,
            } => {
                w.u16(*fps);
                w.u16(*max_ahead);
                w.u8(*frame_send_rate);
            }
            Body::LatencyFudge(v) | Body::GameSpeed(v) | Body::RemovePlayer(v) => w.u8(*v),
            Body::Destruct | Body::Exit => {}
        }
    }

    fn read(r: &mut Reader) -> Result<Order, DecodeError> {
        let tag = r.u8()?;
        let house = r.u8()?;
        let frame = r.u32()?;
        let body = match tag {
            event::GAME => Body::Game(r.bytes()?),
            event::PROCESS_TIME => Body::ProcessTime(r.u16()?),
            event::TIMING => Body::Timing {
                fps: r.u16()?,
                max_ahead: r.u16()?,
                frame_send_rate: r.u8()?,
            },
            event::LATENCYFUDGE => Body::LatencyFudge(r.u8()?),
            event::GAMESPEED => Body::GameSpeed(r.u8()?),
            event::DESTRUCT => Body::Destruct,
            event::EXIT => Body::Exit,
            event::REMOVEPLAYER => Body::RemovePlayer(r.u8()?),
            other => return Err(DecodeError::Tag(other)),
        };
        Ok(Order { frame, house, body })
    }
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        match self {
            Msg::Hello {
                protocol,
                build,
                name,
            } => {
                w.u8(HELLO);
                w.u16(*protocol);
                w.text(build);
                w.text(name);
            }
            Msg::Refuse { code, text } => {
                w.u8(REFUSE);
                w.u8(*code);
                w.text(text);
            }
            Msg::Lobby { setup } => {
                w.u8(LOBBY);
                w.bytes(setup);
            }
            Msg::Start {
                setup,
                game_id,
                internet,
                speed,
                max_ahead,
                frame_send_rate,
            } => {
                w.u8(START);
                w.u32(*game_id);
                w.u8(u8::from(*internet));
                w.u8(*speed);
                w.u16(*max_ahead);
                w.u8(*frame_send_rate);
                w.bytes(setup);
            }
            Msg::Loaded { setup_hash } => {
                w.u8(LOADED);
                w.u64(*setup_hash);
            }
            Msg::Go => w.u8(GO),
            Msg::Packet {
                frame,
                check,
                sent,
                delay,
                orders,
            } => {
                w.u8(event::FRAMEINFO);
                w.u32(*frame);
                w.u32(*check);
                w.u32(*sent);
                w.u8(*delay);
                w.u16(orders.len() as u16);
                for order in orders {
                    order.write(&mut w);
                }
            }
            Msg::FrameSync {
                frame,
                sent,
                game_id,
            } => {
                w.u8(event::FRAMESYNC);
                w.u32(*frame);
                w.u32(*sent);
                w.u32(*game_id);
            }
            Msg::Ping { stamp } => {
                w.u8(PING);
                w.u32(*stamp);
            }
            Msg::Pong { to, stamp } => {
                w.u8(PONG);
                w.u8(*to);
                w.u32(*stamp);
            }
            Msg::Vote { target } => {
                w.u8(VOTE);
                w.u8(*target);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Msg, DecodeError> {
        let mut r = Reader(bytes);
        let msg = match r.u8()? {
            HELLO => Msg::Hello {
                protocol: r.u16()?,
                build: r.text()?,
                name: r.text()?,
            },
            REFUSE => Msg::Refuse {
                code: r.u8()?,
                text: r.text()?,
            },
            LOBBY => Msg::Lobby { setup: r.bytes()? },
            START => {
                let game_id = r.u32()?;
                let internet = r.u8()? != 0;
                let speed = r.u8()?;
                let max_ahead = r.u16()?;
                let frame_send_rate = r.u8()?;
                Msg::Start {
                    setup: r.bytes()?,
                    game_id,
                    internet,
                    speed,
                    max_ahead,
                    frame_send_rate,
                }
            }
            LOADED => Msg::Loaded {
                setup_hash: r.u64()?,
            },
            GO => Msg::Go,
            event::FRAMEINFO => {
                let frame = r.u32()?;
                let check = r.u32()?;
                let sent = r.u32()?;
                let delay = r.u8()?;
                let count = r.u16()?;
                let mut orders = Vec::new();
                for _ in 0..count {
                    orders.push(Order::read(&mut r)?);
                }
                Msg::Packet {
                    frame,
                    check,
                    sent,
                    delay,
                    orders,
                }
            }
            event::FRAMESYNC => Msg::FrameSync {
                frame: r.u32()?,
                sent: r.u32()?,
                game_id: r.u32()?,
            },
            PING => Msg::Ping { stamp: r.u32()? },
            PONG => Msg::Pong {
                to: r.u8()?,
                stamp: r.u32()?,
            },
            VOTE => Msg::Vote { target: r.u8()? },
            other => return Err(DecodeError::Tag(other)),
        };
        if !r.0.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Msg> {
        let bodies = vec![
            Body::Game(vec![]),
            Body::Game((0..=255).collect()),
            Body::ProcessTime(17),
            Body::Timing {
                fps: 45,
                max_ahead: 250,
                frame_send_rate: 10,
            },
            Body::LatencyFudge(3),
            Body::GameSpeed(6),
            Body::Destruct,
            Body::Exit,
            Body::RemovePlayer(7),
        ];
        let orders: Vec<Order> = bodies
            .into_iter()
            .enumerate()
            .map(|(i, body)| Order {
                frame: 1000 + i as u32 * 10,
                house: i as u8,
                body,
            })
            .collect();
        vec![
            Msg::Hello {
                protocol: PROTOCOL,
                build: "b40d5fb".into(),
                name: "Тест ☭".into(),
            },
            Msg::Hello {
                protocol: 0xffff,
                build: String::new(),
                name: String::new(),
            },
            Msg::Refuse {
                code: 2,
                text: "rulesmd.ini differs".into(),
            },
            Msg::Lobby {
                setup: vec![1, 2, 3],
            },
            Msg::Start {
                setup: vec![9; 5000],
                game_id: 0xdead_beef,
                internet: true,
                speed: 2,
                max_ahead: 30,
                frame_send_rate: 5,
            },
            Msg::Loaded {
                setup_hash: 0x0123_4567_89ab_cdef,
            },
            Msg::Go,
            Msg::Packet {
                frame: 40,
                check: 0,
                sent: 0,
                delay: 40,
                orders: vec![],
            },
            Msg::Packet {
                frame: u32::MAX,
                check: 0xffff_fffe,
                sent: 70000,
                delay: 250,
                orders,
            },
            Msg::FrameSync {
                frame: 123,
                sent: 45,
                game_id: 678,
            },
            Msg::Ping { stamp: 99 },
            Msg::Pong { to: 3, stamp: 99 },
            Msg::Vote { target: 2 },
        ]
    }

    #[test]
    fn round_trip() {
        for msg in samples() {
            let bytes = msg.encode();
            assert_eq!(Msg::decode(&bytes), Ok(msg));
        }
    }

    #[test]
    fn truncated_and_trailing_are_refused() {
        for msg in samples() {
            let mut bytes = msg.encode();
            for cut in 0..bytes.len() {
                assert!(Msg::decode(&bytes[..cut]).is_err(), "{msg:?} cut at {cut}");
            }
            bytes.push(0);
            assert_eq!(Msg::decode(&bytes), Err(DecodeError::Trailing));
        }
    }

    #[test]
    fn unknown_tags() {
        assert_eq!(Msg::decode(&[200]), Err(DecodeError::Tag(200)));
        // A packet with one order of an unknown type.
        let mut bytes = Msg::Packet {
            frame: 1,
            check: 2,
            sent: 3,
            delay: 4,
            orders: vec![],
        }
        .encode();
        let at = bytes.len() - 2;
        bytes[at] = 1;
        bytes.extend_from_slice(&[0x7e, 0, 0, 0, 0, 0]);
        assert_eq!(Msg::decode(&bytes), Err(DecodeError::Tag(0x7e)));
        assert_eq!(
            Msg::decode(&[HELLO, 1, 0, 1, 0, 0, 0, 0xff, 0, 0, 0, 0]),
            Err(DecodeError::Text)
        );
    }
}
