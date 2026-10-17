//! The meeting-place server for Internet games: the list of games waiting for
//! players, and the passing of messages between the players of each.
//!
//! Not original behaviour (the original used Westwood Online). It holds no
//! game code and no game files: see `ra2_net::directory`.
//!
//! Usage: `ra2-server [port]`, or the port in `PORT` (default 47100). It
//! listens on every address and logs to standard output.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use ra2_net::TcpLink;
use ra2_net::directory::Directory;

fn main() {
    let port: u16 = std::env::args().nth(1).or_else(|| std::env::var("PORT").ok()).and_then(|port| port.trim().parse().ok()).unwrap_or(47100);
    let listener = TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|error| {
        eprintln!("cannot listen on port {port}: {error}");
        std::process::exit(1);
    });
    listener.set_nonblocking(true).expect("non-blocking listener");
    println!("ra2-server listening on port {port}");
    let mut directory = Directory::new();
    let mut last = (0, 0);
    let mut reported = Instant::now();
    loop {
        // Everybody new, then everything said.
        while let Ok((stream, from)) = listener.accept() {
            let _ = stream.set_nonblocking(false);
            match TcpLink::new(stream) {
                Ok(link) => directory.add(Box::new(link)),
                Err(error) => eprintln!("{from}: {error}"),
            }
        }
        let handled = directory.pump();
        let now = directory.counts();
        if now != last && reported.elapsed() > Duration::from_secs(1) {
            println!("{} connections, {} games", now.0, now.1);
            last = now;
            reported = Instant::now();
        }
        // Busy while there is traffic, easy on the processor when there is none.
        if handled == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
