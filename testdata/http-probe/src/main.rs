//! Minimal HTTP client using write(2)/read(2) for Phase 2 sock probes.

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::process::ExitCode;
use std::thread;
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

/// Two write(2) syscalls before headers complete — Phase 6 reassembly gate.
fn split_header(host: &str, port: u16, path: &str) -> Result<(), String> {
    let addr = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&addr).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();
    let line1 = format!("GET {path} HTTP/1.1\r\n");
    let line2 = format!("Host: {host}:{port}\r\nConnection: close\r\n\r\n");
    let fd = stream.as_raw_fd();
    let n1 = unsafe {
        libc::write(
            fd,
            line1.as_ptr() as *const libc::c_void,
            line1.len(),
        )
    };
    if n1 < 0 {
        return Err("write1 failed".into());
    }
    thread::sleep(Duration::from_millis(50));
    let n2 = unsafe {
        libc::write(
            fd,
            line2.as_ptr() as *const libc::c_void,
            line2.len(),
        )
    };
    if n2 < 0 {
        return Err("write2 failed".into());
    }
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if !buf.windows(7).any(|w| w == b"HTTP/1.") {
        return Err(format!(
            "bad split-header response: {:?}",
            &buf[..buf.len().min(80)]
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut host = "127.0.0.1".to_string();
    let mut port = 18080u16;
    let mut paths: Vec<String> = Vec::new();
    let mut repeat = 1usize;
    let mut split = false;
    let mut correctness6 = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--host" => host = args.next().expect("--host value"),
            "--port" => port = args.next().expect("--port").parse().expect("port"),
            "--path" => paths.push(args.next().expect("--path value")),
            "--repeat" => repeat = args.next().expect("--repeat").parse().expect("n"),
            "--split-header" => split = true,
            "--correctness6" => correctness6 = true,
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

    // One tgid for allow-list refresh: wait so ALLOWED_TGID can pick us up, then
    // 5 full probes + 1 split-header (Phase 6 gate).
    if correctness6 {
        thread::sleep(Duration::from_secs(2));
        let path = &paths[0];
        for _ in 0..5 {
            if let Err(e) = once(&host, port, path) {
                eprintln!("{e}");
                rc = ExitCode::FAILURE;
            }
        }
        if let Err(e) = split_header(&host, port, path) {
            eprintln!("{e}");
            rc = ExitCode::FAILURE;
        }
        return rc;
    }

    if split {
        for path in &paths {
            if let Err(e) = split_header(&host, port, path) {
                eprintln!("{e}");
                rc = ExitCode::FAILURE;
            }
        }
        return rc;
    }
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
