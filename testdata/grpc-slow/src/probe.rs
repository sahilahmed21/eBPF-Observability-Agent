//! h2c client for `grpc-slow`.
//!
//! Two `write()`s: preface marks BPF INFLIGHT (Q8 first-iovec 256 B); HEADERS
//! go on the second write so sticky INFLIGHT copies them. The probe then reads
//! frames until response HEADERS — a single `read()` can return only SETTINGS
//! and miss the 50 ms RPC. The **server** is still tonic.

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const TYPE_DATA: u8 = 0x0;
const TYPE_HEADERS: u8 = 0x1;
const TYPE_SETTINGS: u8 = 0x4;
const TYPE_GOAWAY: u8 = 0x7;
const FLAG_END_STREAM: u8 = 0x1;
const FLAG_END_HEADERS: u8 = 0x4;

fn frame(ty: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let n = payload.len() as u32;
    let mut b = vec![
        (n >> 16) as u8,
        (n >> 8) as u8,
        n as u8,
        ty,
        flags,
        (stream >> 24) as u8,
        (stream >> 16) as u8,
        (stream >> 8) as u8,
        stream as u8,
    ];
    b.extend_from_slice(payload);
    b
}

fn lit_inc(name_idx: u8, value: &[u8]) -> Vec<u8> {
    let mut v = vec![0x40 | name_idx, value.len() as u8];
    v.extend_from_slice(value);
    v
}

fn hpack_grpc(path: &[u8], authority: &[u8]) -> Vec<u8> {
    let mut b = vec![0x83, 0x86];
    b.push(0x04);
    b.push(path.len() as u8);
    b.extend_from_slice(path);
    b.extend(lit_inc(1, authority));
    b.extend(lit_inc(31, b"application/grpc"));
    b.extend_from_slice(&[0x00, 0x02, b't', b'e', 0x08]);
    b.extend_from_slice(b"trailers");
    b
}

fn grpc_data(delay_ms: u64) -> Vec<u8> {
    let mut proto = vec![0x08];
    let mut n = delay_ms;
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
        }
        proto.push(b);
        if n == 0 {
            break;
        }
    }
    let mut out = vec![0];
    out.extend_from_slice(&(proto.len() as u32).to_be_bytes());
    out.extend_from_slice(&proto);
    out
}

fn read_exact(stream: &mut TcpStream, n: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn read_frame(stream: &mut TcpStream) -> Result<(u8, u8, u32), String> {
    let hdr = read_exact(stream, 9)?;
    let length = ((hdr[0] as usize) << 16) | ((hdr[1] as usize) << 8) | hdr[2] as usize;
    let ty = hdr[3];
    let flags = hdr[4];
    let stream_id = u32::from_be_bytes([hdr[5], hdr[6], hdr[7], hdr[8]]) & 0x7fff_ffff;
    if length > 0 {
        let _ = read_exact(stream, length)?;
    }
    Ok((ty, flags, stream_id))
}

fn once(port: u16, delay_ms: u64) -> Result<(), String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();
    let auth = format!("127.0.0.1:{port}");
    let path = b"/slow.Slow/Sleep";
    let mut open = PREFACE.to_vec();
    open.extend(frame(TYPE_SETTINGS, 0, 0, &[]));
    stream.write_all(&open).map_err(|e| e.to_string())?;
    let mut req = frame(
        TYPE_HEADERS,
        FLAG_END_HEADERS,
        1,
        &hpack_grpc(path, auth.as_bytes()),
    );
    req.extend(frame(TYPE_DATA, FLAG_END_STREAM, 1, &grpc_data(delay_ms)));
    stream.write_all(&req).map_err(|e| e.to_string())?;
    loop {
        let (ty, flags, sid) = read_frame(&mut stream)?;
        if ty == TYPE_HEADERS && sid == 1 && flags & FLAG_END_HEADERS != 0 {
            return Ok(());
        }
        if ty == TYPE_GOAWAY {
            return Err("goaway".into());
        }
    }
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut port = 18097u16;
    let mut repeat = 1usize;
    let mut delay_ms = 50u64;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().expect("--port").parse().expect("port"),
            "--repeat" => repeat = args.next().expect("--repeat").parse().expect("n"),
            "--delay-ms" => delay_ms = args.next().expect("--delay-ms").parse().expect("ms"),
            other => {
                eprintln!("unknown arg {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let mut rc = ExitCode::SUCCESS;
    for _ in 0..repeat {
        if let Err(e) = once(port, delay_ms) {
            eprintln!("{e}");
            rc = ExitCode::FAILURE;
        }
    }
    rc
}
