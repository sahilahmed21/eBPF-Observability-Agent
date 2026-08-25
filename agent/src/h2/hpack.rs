//! HPACK decoder: static table + Huffman + bounded dynamic table (32 entries).
//!
//! Dynamic eviction follows RFC 7541 §4.1 (32 + name + value octets) with a
//! 4096-byte table (RFC default) and a 32-entry ceiling. Table-size updates
//! larger than 4096 fail the block (fail closed). Huffman EOS in a string or
//! invalid padding fails the string (RFC 7541 §5.2).

const DYNAMIC_CAP: usize = 32;
const DEFAULT_TABLE_SIZE: usize = 4096;
const ENTRY_OVERHEAD: usize = 32;

/// RFC 7541 Appendix A (1-based). Name/value; empty value means name only.
const STATIC: &[(&str, &str)] = &[
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

#[derive(Clone, Debug, Default)]
pub struct Pseudo {
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<u16>,
    pub authority: Option<String>,
}

pub struct Decoder {
    dynamic: Vec<(String, String)>,
    max_size: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            dynamic: Vec::new(),
            max_size: DEFAULT_TABLE_SIZE,
        }
    }
}

impl Decoder {
    pub fn reset(&mut self) {
        self.dynamic.clear();
        self.max_size = DEFAULT_TABLE_SIZE;
    }

    pub fn decode_block(&mut self, src: &[u8]) -> Option<Pseudo> {
        match self.try_decode(src) {
            Some(p) => Some(p),
            None => {
                self.reset();
                None
            }
        }
    }

    fn try_decode(&mut self, mut src: &[u8]) -> Option<Pseudo> {
        let mut out = Pseudo::default();
        while !src.is_empty() {
            let b0 = src[0];
            if b0 & 0x80 != 0 {
                let (idx, rest) = decode_int(src, 7)?;
                src = rest;
                let (n, v) = self.lookup(idx)?;
                apply(&mut out, &n, &v);
            } else if b0 & 0x40 != 0 {
                let (name, value, rest) = self.decode_literal(src, 6)?;
                src = rest;
                apply(&mut out, &name, &value);
                self.push(name, value);
            } else if b0 & 0x20 != 0 {
                let (size, rest) = decode_int(src, 5)?;
                src = rest;
                self.set_max_size(size)?;
            } else {
                let (name, value, rest) = self.decode_literal(src, 4)?;
                src = rest;
                apply(&mut out, &name, &value);
            }
        }
        Some(out)
    }

    fn set_max_size(&mut self, size: u64) -> Option<()> {
        if size > DEFAULT_TABLE_SIZE as u64 {
            return None;
        }
        self.max_size = size as usize;
        self.evict();
        Some(())
    }

    fn table_octets(&self) -> usize {
        self.dynamic
            .iter()
            .map(|(n, v)| ENTRY_OVERHEAD + n.len() + v.len())
            .sum()
    }

    fn evict(&mut self) {
        while self.dynamic.len() > DYNAMIC_CAP || self.table_octets() > self.max_size {
            if self.dynamic.pop().is_none() {
                break;
            }
        }
    }

    fn decode_literal<'a>(
        &self,
        src: &'a [u8],
        prefix: u8,
    ) -> Option<(String, String, &'a [u8])> {
        let (idx, rest) = decode_int(src, prefix)?;
        let (name, rest) = if idx == 0 {
            decode_string(rest)?
        } else {
            let (n, _) = self.lookup(idx)?;
            (n, rest)
        };
        let (value, rest) = decode_string(rest)?;
        Some((name, value, rest))
    }

    fn lookup(&self, idx: u64) -> Option<(String, String)> {
        if idx == 0 {
            return None;
        }
        let i = idx as usize;
        if i <= STATIC.len() {
            let (n, v) = STATIC[i - 1];
            return Some((n.to_string(), v.to_string()));
        }
        let d = i - STATIC.len() - 1;
        self.dynamic.get(d).cloned()
    }

    fn push(&mut self, name: String, value: String) {
        self.dynamic.insert(0, (name, value));
        self.evict();
    }
}

fn apply(out: &mut Pseudo, name: &str, value: &str) {
    match name {
        ":method" => out.method = Some(value.to_string()),
        ":path" => out.path = Some(value.to_string()),
        ":authority" => out.authority = Some(value.to_string()),
        ":status" => out.status = value.parse().ok(),
        _ => {}
    }
}

fn decode_int(src: &[u8], prefix_bits: u8) -> Option<(u64, &[u8])> {
    if src.is_empty() {
        return None;
    }
    let mask = (1u8 << prefix_bits) - 1;
    let mut v = (src[0] & mask) as u64;
    if v < mask as u64 {
        return Some((v, &src[1..]));
    }
    let mut shift = 0u32;
    let mut i = 1usize;
    loop {
        let b = *src.get(i)?;
        i += 1;
        v += ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((v, &src[i..]));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
}

fn decode_string(src: &[u8]) -> Option<(String, &[u8])> {
    if src.is_empty() {
        return None;
    }
    let huffman = src[0] & 0x80 != 0;
    let (len, rest) = decode_int(src, 7)?;
    let n = len as usize;
    if rest.len() < n {
        return None;
    }
    let raw = &rest[..n];
    let bytes = if huffman {
        decode_huffman(raw)?
    } else {
        raw.to_vec()
    };
    let s = String::from_utf8(bytes).ok()?;
    Some((s, &rest[n..]))
}

/// RFC 7541 Appendix B: (code, bit length) for symbols 0..=255; EOS = 256.
const HUFF: [(u32, u8); 257] = [
    (0x1ff8, 13), (0x7fffd8, 23), (0xfffffe2, 28), (0xfffffe3, 28),
    (0xfffffe4, 28), (0xfffffe5, 28), (0xfffffe6, 28), (0xfffffe7, 28),
    (0xfffffe8, 28), (0xffffea, 24), (0x3ffffffc, 30), (0xfffffe9, 28),
    (0xfffffea, 28), (0x3ffffffd, 30), (0xfffffeb, 28), (0xfffffec, 28),
    (0xfffffed, 28), (0xfffffee, 28), (0xfffffef, 28), (0xffffff0, 28),
    (0xffffff1, 28), (0xffffff2, 28), (0x3ffffffe, 30), (0xffffff3, 28),
    (0xffffff4, 28), (0xffffff5, 28), (0xffffff6, 28), (0xffffff7, 28),
    (0xffffff8, 28), (0xffffff9, 28), (0xffffffa, 28), (0xffffffb, 28),
    (0x14, 6), (0x3f8, 10), (0x3f9, 10), (0xffa, 12),
    (0x1ff9, 13), (0x15, 6), (0xf8, 8), (0x7fa, 11),
    (0x3fa, 10), (0x3fb, 10), (0xf9, 8), (0x7fb, 11),
    (0xfa, 8), (0x16, 6), (0x17, 6), (0x18, 6),
    (0x0, 5), (0x1, 5), (0x2, 5), (0x19, 6),
    (0x1a, 6), (0x1b, 6), (0x1c, 6), (0x1d, 6),
    (0x1e, 6), (0x1f, 6), (0x5c, 7), (0xfb, 8),
    (0x7ffc, 15), (0x20, 6), (0xffb, 12), (0x3fc, 10),
    (0x1ffa, 13), (0x21, 6), (0x5d, 7), (0x5e, 7),
    (0x5f, 7), (0x60, 7), (0x61, 7), (0x62, 7),
    (0x63, 7), (0x64, 7), (0x65, 7), (0x66, 7),
    (0x67, 7), (0x68, 7), (0x69, 7), (0x6a, 7),
    (0x6b, 7), (0x6c, 7), (0x6d, 7), (0x6e, 7),
    (0x6f, 7), (0x70, 7), (0x71, 7), (0x72, 7),
    (0xfc, 8), (0x73, 7), (0xfd, 8), (0x1ffb, 13),
    (0x7fff0, 19), (0x1ffc, 13), (0x3ffc, 14), (0x22, 6),
    (0x7ffd, 15), (0x3, 5), (0x23, 6), (0x4, 5),
    (0x24, 6), (0x5, 5), (0x25, 6), (0x26, 6),
    (0x27, 6), (0x6, 5), (0x74, 7), (0x75, 7),
    (0x28, 6), (0x29, 6), (0x2a, 6), (0x7, 5),
    (0x2b, 6), (0x76, 7), (0x2c, 6), (0x8, 5),
    (0x9, 5), (0x2d, 6), (0x77, 7), (0x78, 7),
    (0x79, 7), (0x7a, 7), (0x7b, 7), (0x7ffe, 15),
    (0x7fc, 11), (0x3ffd, 14), (0x1ffd, 13), (0xffffffc, 28),
    (0xfffe6, 20), (0x3fffd2, 22), (0xfffe7, 20), (0xfffe8, 20),
    (0x3fffd3, 22), (0x3fffd4, 22), (0x3fffd5, 22), (0x7fffd9, 23),
    (0x3fffd6, 22), (0x7fffda, 23), (0x7fffdb, 23), (0x7fffdc, 23),
    (0x7fffdd, 23), (0x7fffde, 23), (0xffffeb, 24), (0x7fffdf, 23),
    (0xffffec, 24), (0xffffed, 24), (0x3fffd7, 22), (0x7fffe0, 23),
    (0xffffee, 24), (0x7fffe1, 23), (0x7fffe2, 23), (0x7fffe3, 23),
    (0x7fffe4, 23), (0x1fffdc, 21), (0x3fffd8, 22), (0x7fffe5, 23),
    (0x3fffd9, 22), (0x7fffe6, 23), (0x7fffe7, 23), (0xffffef, 24),
    (0x3fffda, 22), (0x1fffdd, 21), (0xfffe9, 20), (0x3fffdb, 22),
    (0x3fffdc, 22), (0x7fffe8, 23), (0x7fffe9, 23), (0x1fffde, 21),
    (0x7fffea, 23), (0x3fffdd, 22), (0x3fffde, 22), (0xfffff0, 24),
    (0x1fffdf, 21), (0x3fffdf, 22), (0x7fffeb, 23), (0x7fffec, 23),
    (0x1fffe0, 21), (0x1fffe1, 21), (0x3fffe0, 22), (0x1fffe2, 21),
    (0x7fffed, 23), (0x3fffe1, 22), (0x7fffee, 23), (0x7fffef, 23),
    (0xfffea, 20), (0x3fffe2, 22), (0x3fffe3, 22), (0x3fffe4, 22),
    (0x7ffff0, 23), (0x3fffe5, 22), (0x3fffe6, 22), (0x7ffff1, 23),
    (0x3ffffe0, 26), (0x3ffffe1, 26), (0xfffeb, 20), (0x7fff1, 19),
    (0x3fffe7, 22), (0x7ffff2, 23), (0x3fffe8, 22), (0x1ffffec, 25),
    (0x3ffffe2, 26), (0x3ffffe3, 26), (0x3ffffe4, 26), (0x7ffffde, 27),
    (0x7ffffdf, 27), (0x3ffffe5, 26), (0xfffff1, 24), (0x1ffffed, 25),
    (0x7fff2, 19), (0x1fffe3, 21), (0x3ffffe6, 26), (0x7ffffe0, 27),
    (0x7ffffe1, 27), (0x3ffffe7, 26), (0x7ffffe2, 27), (0xfffff2, 24),
    (0x1fffe4, 21), (0x1fffe5, 21), (0x3ffffe8, 26), (0x3ffffe9, 26),
    (0xffffffd, 28), (0x7ffffe3, 27), (0x7ffffe4, 27), (0x7ffffe5, 27),
    (0xfffec, 20), (0xfffff3, 24), (0xfffed, 20), (0x1fffe6, 21),
    (0x3fffe9, 22), (0x1fffe7, 21), (0x1fffe8, 21), (0x7ffff3, 23),
    (0x3fffea, 22), (0x3fffeb, 22), (0x1ffffee, 25), (0x1ffffef, 25),
    (0xfffff4, 24), (0xfffff5, 24), (0x3ffffea, 26), (0x7ffff4, 23),
    (0x3ffffeb, 26), (0x7ffffe6, 27), (0x3ffffec, 26), (0x3ffffed, 26),
    (0x7ffffe7, 27), (0x7ffffe8, 27), (0x7ffffe9, 27), (0x7ffffea, 27),
    (0x7ffffeb, 27), (0xffffffe, 28), (0x7ffffec, 27), (0x7ffffed, 27),
    (0x7ffffee, 27), (0x7ffffef, 27), (0x7fffff0, 27), (0x3ffffee, 26),
    (0x3fffffff, 30),
];

fn decode_huffman(src: &[u8]) -> Option<Vec<u8>> {
    let mut acc: u64 = 0;
    let mut nbits: u32 = 0;
    let mut out = Vec::new();
    for &b in src {
        acc = (acc << 8) | b as u64;
        nbits += 8;
        while let Some((sym, used)) = match_huff(acc, nbits) {
            nbits -= used;
            acc &= (1u64 << nbits).wrapping_sub(1);
            if sym == 256 {
                return None;
            }
            if sym > 255 {
                return None;
            }
            out.push(sym as u8);
        }
    }
    if nbits > 7 {
        return None;
    }
    if nbits > 0 {
        let pad = (1u64 << nbits) - 1;
        if acc != pad {
            return None;
        }
    }
    Some(out)
}

fn match_huff(acc: u64, nbits: u32) -> Option<(u16, u32)> {
    if nbits < 5 {
        return None;
    }
    for (sym, &(code, bits)) in HUFF.iter().enumerate() {
        let bits = bits as u32;
        if nbits < bits {
            continue;
        }
        let shift = nbits - bits;
        if (acc >> shift) == code as u64 {
            return Some((sym as u16, bits));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_post_and_literal_path() {
        // 0x83 = indexed :method POST (static 3)
        // 0x04 = literal without indexing, name index 4 (:path)
        // 0x18 = 24-byte raw string
        let path = b"/hello.Greeter/SayHello";
        assert_eq!(path.len(), 23);
        let mut blk = vec![0x83, 0x04, 0x17];
        blk.extend_from_slice(path);
        let mut d = Decoder::default();
        let p = d.decode_block(&blk).expect("decode");
        assert_eq!(p.method.as_deref(), Some("POST"));
        assert_eq!(p.path.as_deref(), Some("/hello.Greeter/SayHello"));
    }

    #[test]
    fn second_request_uses_dynamic_index() {
        let path = b"/hello.Greeter/SayHello";
        // incremental indexing of :path (index 4) → 0x44
        let mut first = vec![0x83, 0x44, path.len() as u8];
        first.extend_from_slice(path);
        let mut d = Decoder::default();
        d.decode_block(&first).unwrap();
        // static 61 + first dynamic at index 62 → 0x80 | 62 = 0xBE
        let second = vec![0x83, 0xBE];
        let p = d.decode_block(&second).expect("dyn");
        assert_eq!(p.method.as_deref(), Some("POST"));
        assert_eq!(p.path.as_deref(), Some("/hello.Greeter/SayHello"));
    }

    #[test]
    fn indexed_status_200() {
        let mut d = Decoder::default();
        let p = d.decode_block(&[0x88]).unwrap();
        assert_eq!(p.status, Some(200));
    }

    #[test]
    fn rfc7541_c4_authority_huffman() {
        // RFC 7541 C.4.1 first request (Huffman :authority).
        let blk = [
            0x82, 0x86, 0x84, 0x41, 0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab,
            0x90, 0xf4, 0xff,
        ];
        let mut d = Decoder::default();
        let p = d.decode_block(&blk).expect("c4");
        assert_eq!(p.method.as_deref(), Some("GET"));
        assert_eq!(p.path.as_deref(), Some("/"));
        assert_eq!(p.authority.as_deref(), Some("www.example.com"));
    }

    #[test]
    fn huffman_literal_www_dot() {
        // RFC 7541 C.1.2-ish: Huffman for "www.example.com" is a known vector.
        // 0x00 = literal without indexing, new name
        // name "x" uncompressed, value Huffman "www"
        // Keep this small: decode Huffman of 'w' (code 0x77, 7 bits from table...
        // Use uncompressed to avoid vector mistakes; Huffman is covered via
        // encode-roundtrip of a single ASCII run we build from HUFF.
        let bytes = huffman_encode(b"abc");
        let mut blk = vec![0x00, 0x01, b'x', 0x80 | bytes.len() as u8];
        blk.extend_from_slice(&bytes);
        let mut d = Decoder::default();
        let p = d.decode_block(&blk).expect("huff");
        assert!(p.method.is_none());
        // 'x' is not a pseudo; just ensure we didn't panic and Huffman consumed.
        let _ = p;
        let decoded = decode_huffman(&bytes).expect("round");
        assert_eq!(decoded, b"abc");
    }

    #[test]
    fn table_size_zero_drops_dynamic() {
        let path = b"/hello.Greeter/SayHello";
        let mut first = vec![0x83, 0x44, path.len() as u8];
        first.extend_from_slice(path);
        let mut d = Decoder::default();
        d.decode_block(&first).unwrap();
        assert!(d.decode_block(&[0x20]).is_some());
        assert!(d.decode_block(&[0x83, 0xBE]).is_none());
        let p = d.decode_block(&[0x88]).expect("static after reset");
        assert_eq!(p.status, Some(200));
    }

    #[test]
    fn table_size_over_default_fails_closed() {
        let mut d = Decoder::default();
        // 0x3f = 5-bit prefix all ones → extra integer bytes; 0x80 0x40 → large size.
        assert!(d.decode_block(&[0x3f, 0x80, 0x40]).is_none());
        let p = d.decode_block(&[0x88]).expect("reset");
        assert_eq!(p.status, Some(200));
    }

    #[test]
    fn huffman_eos_in_string_is_error() {
        assert!(decode_huffman(&[0xff, 0xff, 0xff, 0xff]).is_none());
    }

    #[test]
    fn huffman_bad_padding_is_error() {
        // ASCII '0' is 5 zero bits; byte 0x00 pads with zeros, not EOS ones.
        assert!(decode_huffman(&[0x00]).is_none());
    }
}

#[cfg(test)]
fn huffman_encode(src: &[u8]) -> Vec<u8> {
    let mut acc: u64 = 0;
    let mut nbits: u32 = 0;
    let mut out = Vec::new();
    for &b in src {
        let (code, bits) = HUFF[b as usize];
        acc = (acc << bits) | code as u64;
        nbits += bits as u32;
        while nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
            acc &= (1u64 << nbits).wrapping_sub(1);
        }
    }
    if nbits > 0 {
        let pad = 8 - nbits;
        acc = (acc << pad) | ((1u64 << pad) - 1);
        out.push(acc as u8);
    }
    out
}
