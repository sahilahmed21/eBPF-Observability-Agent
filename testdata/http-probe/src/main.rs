//! Minimal HTTP client using write(2)/read(2) for Phase 2 sock probes.

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

fn once(host: &str, port: u16, path: &str) -> Result<(), String> {
    let addr = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&addr).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if !buf.windows(7).any(|w| w == b"HTTP/1.") {
        return Err(format!("bad response for {path}: {:?}", &buf[..buf.len().min(80)]));
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut host = "127.0.0.1".to_string();
    let mut port = 18080u16;
    let mut paths: Vec<String> = Vec::new();
    let mut repeat = 1usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--host" => host = args.next().expect("--host value"),
            "--port" => port = args.next().expect("--port").parse().expect("port"),
            "--path" => paths.push(args.next().expect("--path value")),
            "--repeat" => repeat = args.next().expect("--repeat").parse().expect("n"),
            other => {
                eprintln!("unknown arg {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    if paths.is_empty() {
        paths.push("/fast".into());
    }
    let mut rc = ExitCode::SUCCESS;
    for _ in 0..repeat {
        for path in &paths {
            if let Err(e) = once(&host, port, path) {
                eprintln!("{e}");
                rc = ExitCode::FAILURE;
            }
        }
    }
    rc
}
