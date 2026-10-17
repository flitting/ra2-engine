//! Not original behaviour: `Link` over a TCP stream, each message prefixed by its length
//! (u32, little endian). One reader thread per connection feeds a channel so `poll` never
//! blocks.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::link::Link;

/// A frame longer than this is treated as a broken stream. In-game packets are tens of
/// bytes; the largest message is the lobby's setup blob.
pub const MAX_FRAME: usize = 1 << 20;

pub struct TcpLink {
    stream: TcpStream,
    incoming: Receiver<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

impl TcpLink {
    pub fn connect(address: impl ToSocketAddrs) -> io::Result<TcpLink> {
        TcpLink::new(TcpStream::connect(address)?)
    }

    /// Wrap a connected stream (from `connect` or a listener's `accept`).
    pub fn new(stream: TcpStream) -> io::Result<TcpLink> {
        // Packets are small and the stall rule waits on them: never batch.
        stream.set_nodelay(true)?;
        let mut reader = stream.try_clone()?;
        let (sender, incoming) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        thread::Builder::new()
            .name("ra2-net tcp reader".into())
            .spawn(move || {
                loop {
                    let mut length = [0u8; 4];
                    if reader.read_exact(&mut length).is_err() {
                        break;
                    }
                    let length = u32::from_le_bytes(length) as usize;
                    if length > MAX_FRAME {
                        break;
                    }
                    let mut frame = vec![0u8; length];
                    if reader.read_exact(&mut frame).is_err() || sender.send(frame).is_err() {
                        break;
                    }
                }
                flag.store(true, Ordering::SeqCst);
                let _ = reader.shutdown(Shutdown::Both);
            })?;
        Ok(TcpLink {
            stream,
            incoming,
            closed,
        })
    }
}

impl Link for TcpLink {
    fn send(&mut self, bytes: &[u8]) {
        if bytes.len() > MAX_FRAME || self.closed() {
            return;
        }
        // One write per frame, so a frame is never split across two small segments.
        let mut frame = Vec::with_capacity(bytes.len() + 4);
        frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        frame.extend_from_slice(bytes);
        if self.stream.write_all(&frame).is_err() {
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    fn poll(&mut self) -> Option<Vec<u8>> {
        self.incoming.try_recv().ok()
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Drop for TcpLink {
    fn drop(&mut self) {
        // Wakes the reader thread out of its blocking read.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}
