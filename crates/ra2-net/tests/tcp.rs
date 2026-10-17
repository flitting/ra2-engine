//! `TcpLink` over the loopback interface: two threads, a few hundred framed messages each
//! way, of every size from empty to tens of kilobytes.

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ra2_net::{Link, Msg, TcpLink};

fn message(index: usize) -> Vec<u8> {
    let length = match index % 7 {
        0 => 0,
        1 => 1,
        2 => 13,
        3 => 255,
        4 => 256,
        5 => 4096 + index,
        _ => 40_000 + index,
    };
    (0..length).map(|i| (i * 31 + index) as u8).collect()
}

/// Wait for the next message (the test's only use of wall time: a give-up limit).
fn next(link: &mut TcpLink) -> Vec<u8> {
    let start = Instant::now();
    loop {
        if let Some(bytes) = link.poll() {
            return bytes;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out");
        assert!(!link.closed(), "closed early");
        thread::sleep(Duration::from_micros(200));
    }
}

#[test]
fn loopback() {
    const COUNT: usize = 400;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();

    // The far end answers every message with the same bytes reversed.
    let far = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut link = TcpLink::new(stream).unwrap();
        for index in 0..COUNT {
            let mut bytes = next(&mut link);
            assert_eq!(bytes, message(index));
            bytes.reverse();
            link.send(&bytes);
        }
        // Stay until the near end hangs up.
        let start = Instant::now();
        while !link.closed() {
            assert!(start.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(1));
        }
    });

    let mut link = TcpLink::connect(address).unwrap();
    // Send everything first, so frames queue up and are split and joined by the stream.
    for index in 0..COUNT {
        link.send(&message(index));
    }
    for index in 0..COUNT {
        let mut expected = message(index);
        expected.reverse();
        assert_eq!(next(&mut link), expected, "message {index}");
    }
    assert_eq!(link.poll(), None);

    // A real message survives the trip too.
    assert!(!link.closed());
    drop(link);
    far.join().unwrap();
}

#[test]
fn carries_encoded_messages_and_reports_closing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let far = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut link = TcpLink::new(stream).unwrap();
        let hello = Msg::decode(&next(&mut link)).unwrap();
        link.send(
            &Msg::Refuse {
                code: 1,
                text: format!("no: {hello:?}"),
            }
            .encode(),
        );
        // Dropping the link closes the connection.
    });
    let mut link = TcpLink::connect(address).unwrap();
    link.send(
        &Msg::Hello {
            protocol: 1,
            build: "abc".into(),
            name: "Tanya".into(),
        }
        .encode(),
    );
    let reply = Msg::decode(&next(&mut link)).unwrap();
    assert!(matches!(reply, Msg::Refuse { code: 1, .. }));
    far.join().unwrap();
    let start = Instant::now();
    while !link.closed() {
        assert!(start.elapsed() < Duration::from_secs(20));
        thread::sleep(Duration::from_millis(1));
    }
    // Sending into a closed link is harmless.
    link.send(b"x");
}
