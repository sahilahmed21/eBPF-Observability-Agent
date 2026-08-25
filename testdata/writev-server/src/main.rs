//! HTTP/1.1 server that **must** answer via writev(2). Status line is iov[0].

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::fd::AsRawFd;
use std::thread;
use std::time::Duration;

fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18086);
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    eprintln!("writev-server listening on http://127.0.0.1:{port}");
    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(s) => s,
            Err(_) => continue,
        };
        thread::spawn(move || handle(&mut stream));
    }
}

fn handle(stream: &mut std::net::TcpStream) {
    // Read until end of headers so split-header clients (two write syscalls)
    // are answered as one exchange — a single read races the 20ms gap.
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 512];
    loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return,
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= 2048 {
            break;
        }
    }
    if buf.is_empty() {
        return;
    }
    let req = String::from_utf8_lossy(&buf);
    let delay_ms = req
        .split("delay_ms=")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    if req.contains("/slow") && delay_ms > 0 {
        thread::sleep(Duration::from_millis(delay_ms));
    } else if req.contains("/slow") {
        thread::sleep(Duration::from_millis(50));
    }
    let hdr = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n";
    let body = b"ok";
    let fd = stream.as_raw_fd();
    let iov = [
        libc::iovec {
            iov_base: hdr.as_ptr() as *mut libc::c_void,
            iov_len: hdr.len(),
        },
        libc::iovec {
            iov_base: body.as_ptr() as *mut libc::c_void,
            iov_len: body.len(),
        },
    ];
    let _ = unsafe { libc::writev(fd, iov.as_ptr(), 2) };
    let _ = stream.flush();
}
