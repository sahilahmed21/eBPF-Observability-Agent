//! HTTP/2 frame header (RFC 9113 §4.1). Hand-rolled; no `h2` crate.

pub const HEADER_LEN: usize = 9;
pub const CLIENT_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

#[allow(dead_code)]
pub const TYPE_DATA: u8 = 0x0;
pub const TYPE_HEADERS: u8 = 0x1;
#[allow(dead_code)]
pub const TYPE_PRIORITY: u8 = 0x2;
pub const TYPE_RST_STREAM: u8 = 0x3;
pub const TYPE_SETTINGS: u8 = 0x4;
pub const TYPE_PING: u8 = 0x6;
pub const TYPE_GOAWAY: u8 = 0x7;
pub const TYPE_WINDOW_UPDATE: u8 = 0x8;
pub const TYPE_CONTINUATION: u8 = 0x9;

#[allow(dead_code)]
pub const FLAG_END_STREAM: u8 = 0x1;
pub const FLAG_END_HEADERS: u8 = 0x4;
pub const FLAG_PADDED: u8 = 0x8;
pub const FLAG_PRIORITY: u8 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub length: u32,
    pub ty: u8,
    pub flags: u8,
    pub stream_id: u32,
}

impl FrameHeader {
    pub fn parse(src: &[u8]) -> Option<Self> {
        if src.len() < HEADER_LEN {
            return None;
        }
        let length = u32::from_be_bytes([0, src[0], src[1], src[2]]);
        let ty = src[3];
        let flags = src[4];
        let stream_id = u32::from_be_bytes([src[5], src[6], src[7], src[8]]) & 0x7fff_ffff;
        Some(Self {
            length,
            ty,
            flags,
            stream_id,
        })
    }

    pub fn total_len(self) -> usize {
        HEADER_LEN + self.length as usize
    }
}

/// True if `b` is the client connection preface or an unambiguous prefix of it.
pub fn looks_like_preface(b: &[u8]) -> bool {
    if b.is_empty() {
        return false;
    }
    if b.len() >= CLIENT_PREFACE.len() {
        return b.starts_with(CLIENT_PREFACE);
    }
    CLIENT_PREFACE.starts_with(b) && b.starts_with(b"PRI")
}

/// First-frame detect. All-zero 9 bytes are DATA length 0 — not a connection start.
/// Preface or SETTINGS/PING/GOAWAY on stream 0, WINDOW_UPDATE, or stream HEADERS/RST/CONTINUATION.
pub fn looks_like_frame(b: &[u8]) -> bool {
    let Some(h) = FrameHeader::parse(b) else {
        return false;
    };
    if h.length > 16 * 1024 {
        return false;
    }
    match h.ty {
        TYPE_SETTINGS | TYPE_PING | TYPE_GOAWAY => h.stream_id == 0,
        TYPE_WINDOW_UPDATE => true,
        TYPE_HEADERS | TYPE_RST_STREAM | TYPE_CONTINUATION => h.stream_id != 0,
        _ => false,
    }
}

pub fn looks_like_h2(b: &[u8]) -> bool {
    looks_like_preface(b) || looks_like_frame(b)
}

/// HTTP/1.1 start-line on a previously h2-marked fd (fd reuse after close).
pub fn looks_like_http11(b: &[u8]) -> bool {
    if b.starts_with(b"PRI ") || looks_like_frame(b) {
        return false;
    }
    let n = b.len().min(u16::MAX as usize) as u16;
    obsagent_common::http_magic(b, n)
}

/// Strip PADDED / PRIORITY from a HEADERS or CONTINUATION payload.
pub fn headers_fragment<'a>(flags: u8, payload: &'a [u8]) -> Option<&'a [u8]> {
    let mut p = payload;
    if flags & FLAG_PADDED != 0 {
        if p.is_empty() {
            return None;
        }
        let pad = p[0] as usize;
        p = p.get(1..)?;
        if p.len() < pad {
            return None;
        }
        p = &p[..p.len() - pad];
    }
    if flags & FLAG_PRIORITY != 0 {
        if p.len() < 5 {
            return None;
        }
        p = &p[5..];
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_settings() {
        // length=0 type=SETTINGS flags=0 stream=0
        let b = [0, 0, 0, TYPE_SETTINGS, 0, 0, 0, 0, 0];
        let h = FrameHeader::parse(&b).expect("hdr");
        assert_eq!(h.length, 0);
        assert_eq!(h.ty, TYPE_SETTINGS);
        assert_eq!(h.stream_id, 0);
        assert_eq!(h.total_len(), 9);
    }

    #[test]
    fn parses_headers_stream_1() {
        let mut b = [0u8; 9];
        b[2] = 4; // length 4
        b[3] = TYPE_HEADERS;
        b[4] = FLAG_END_HEADERS | FLAG_END_STREAM;
        b[8] = 1; // stream 1
        let h = FrameHeader::parse(&b).unwrap();
        assert_eq!(h.length, 4);
        assert_eq!(h.stream_id, 1);
        assert_eq!(h.flags & FLAG_END_HEADERS, FLAG_END_HEADERS);
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(FrameHeader::parse(&[0, 0, 1, 1, 0]).is_none());
    }

    #[test]
    fn preface_detect() {
        assert!(looks_like_preface(CLIENT_PREFACE));
        assert!(looks_like_preface(b"PRI * HTTP/2.0"));
        assert!(looks_like_preface(b"PRI"));
        assert!(!looks_like_preface(b"GET /"));
        assert!(!looks_like_preface(b"PRINTER"));
    }

    #[test]
    fn settings_looks_like_h2_get_does_not() {
        let settings = [0, 0, 0, TYPE_SETTINGS, 0, 0, 0, 0, 0];
        assert!(looks_like_frame(&settings));
        assert!(looks_like_h2(&settings));
        assert!(!looks_like_http11(&settings));
        assert!(looks_like_http11(b"GET /slow HTTP/1.1\r\n"));
        assert!(!looks_like_h2(b"GET /slow HTTP/1.1\r\n"));
    }

    #[test]
    fn zeros_are_not_h2() {
        assert!(!looks_like_frame(&[0u8; 9]));
        assert!(!looks_like_h2(&[0u8; 9]));
    }

    #[test]
    fn headers_stream_zero_is_not_detect() {
        let mut b = [0u8; 9];
        b[3] = TYPE_HEADERS;
        assert!(!looks_like_frame(&b));
        b[8] = 1;
        assert!(looks_like_frame(&b));
    }
}
