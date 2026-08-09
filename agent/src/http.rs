//! HTTP parse + path normalize (Phase 2 Q9/Q10).
//!
//! Redaction (Q14) lives in [`redact_headers`] for export sinks — not on the
//! parse hot path (CLI is metrics-only).

use crate::correlate::Exchange;

/// Aggregation key: `METHOD + normalized_path` (Q9).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HttpEndpoint {
    pub method: String,
    pub path: String,
}

impl HttpEndpoint {
    pub fn label(&self) -> String {
        format!("{} {}", self.method, self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedExchange {
    pub endpoint: HttpEndpoint,
    pub status: u16,
    pub latency_ns: u64,
}

/// Strip sensitive headers in-place (Q14). Call before off-node export / logging.
#[allow(dead_code)] // ponytail: no export sink yet; keep off hot path until there is one.
pub fn redact_headers(buf: &mut [u8]) {
    for header in [b"authorization:" as &[u8], b"cookie:", b"set-cookie:"] {
        redact_header_line(buf, header);
    }
}

fn redact_header_line(buf: &mut [u8], name_lc: &[u8]) {
    let lower: Vec<u8> = buf.iter().map(u8::to_ascii_lowercase).collect();
    let Some(mut i) = find_subslice(&lower, name_lc) else {
        return;
    };
    i += name_lc.len();
    while i < buf.len() && (buf[i] == b' ' || buf[i] == b'\t') {
        i += 1;
    }
    while i < buf.len() && buf[i] != b'\r' && buf[i] != b'\n' {
        buf[i] = b'*';
        i += 1;
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Replace pure-digit path segments with `:id` (Q10).
pub fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    let parts: Vec<&str> = path
        .split('/')
        .map(|seg| {
            if is_numeric_id(seg) {
                ":id"
            } else {
                seg
            }
        })
        .collect();
    let joined = parts.join("/");
    if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    }
}

fn is_numeric_id(seg: &str) -> bool {
    !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())
}

/// Parse an exchange into endpoint + status. Truncated headers → partial OK.
pub fn parse_exchange(ex: &Exchange) -> Option<ParsedExchange> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    match request.parse(&ex.req_prefix) {
        Ok(httparse::Status::Complete(_)) | Ok(httparse::Status::Partial) => {}
        Err(_) => return None,
    }
    let method = request.method?.to_string();
    let path_raw = request.path?;
    let path_only = path_raw.split('?').next().unwrap_or(path_raw);
    let path = normalize_path(path_only);

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut response = httparse::Response::new(&mut headers);
    match response.parse(&ex.resp_prefix) {
        Ok(httparse::Status::Complete(_)) | Ok(httparse::Status::Partial) => {}
        Err(_) => return None,
    }
    let status = response.code?;

    Some(ParsedExchange {
        endpoint: HttpEndpoint { method, path },
        status,
        latency_ns: ex.latency_ns(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correlate::Exchange;

    #[test]
    fn normalize_numeric_segments() {
        assert_eq!(normalize_path("/users/123"), "/users/:id");
        assert_eq!(normalize_path("/users/123/posts/456"), "/users/:id/posts/:id");
        assert_eq!(normalize_path("/health"), "/health");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn redacts_authorization() {
        let mut buf = b"GET / HTTP/1.1\r\nAuthorization: Bearer secret\r\n\r\n".to_vec();
        redact_headers(&mut buf);
        let s = String::from_utf8_lossy(&buf);
        assert!(!s.contains("secret"));
        assert!(s.contains("Authorization:"));
    }

    #[test]
    fn parses_get_200() {
        let ex = Exchange {
            tgid: 1,
            fd: 3,
            req_prefix: b"GET /users/42 HTTP/1.1\r\nHost: x\r\n\r\n".to_vec(),
            resp_prefix: b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            t_start_ns: 0,
            t_end_ns: 1_000_000,
        };
        let p = parse_exchange(&ex).expect("parsed");
        assert_eq!(p.endpoint.label(), "GET /users/:id");
        assert_eq!(p.status, 200);
        assert_eq!(p.latency_ns, 1_000_000);
    }

    #[test]
    fn truncated_request_still_partial_ok() {
        let ex = Exchange {
            tgid: 1,
            fd: 3,
            req_prefix: b"GET /fast HTTP/1.1\r\nHost: localhost".to_vec(),
            resp_prefix: b"HTTP/1.1 404 Not Found\r\n".to_vec(),
            t_start_ns: 0,
            t_end_ns: 10,
        };
        let p = parse_exchange(&ex).expect("partial");
        assert_eq!(p.endpoint.method, "GET");
        assert_eq!(p.endpoint.path, "/fast");
        assert_eq!(p.status, 404);
    }
}
