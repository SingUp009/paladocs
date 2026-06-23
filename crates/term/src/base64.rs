//! 標準 base64（RFC 4648, `+/` アルファベット・`=` パディング）の手書き実装。
//!
//! Kitty graphics protocol のペイロードは base64 で運ばれる。依存最小主義のため
//! 外部クレートを足さず、本モジュールに必要十分なエンコード（と検証/テスト用の
//! デコード）だけを持つ。いずれも純粋関数でパニックしない。

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `input` を標準 base64（パディング付き）でエンコードする。
///
/// 出力長は常に `input.len().div_ceil(3) * 4` で、4 の倍数。
pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize]);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3f) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3f) as usize]
        } else {
            b'='
        });
    }
    out
}

/// 標準 base64 をデコードする。検証・テスト用。
///
/// 長さが 4 の倍数でない、または不正な文字を含む場合は `None`。
#[cfg(test)]
pub fn decode(input: &[u8]) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    if !input.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    for chunk in input.chunks(4) {
        let c0 = val(chunk[0])?;
        let c1 = val(chunk[1])?;
        let (c2, has2) = if chunk[2] == b'=' {
            (0, false)
        } else {
            (val(chunk[2])?, true)
        };
        let (c3, has3) = if chunk[3] == b'=' {
            (0, false)
        } else {
            (val(chunk[3])?, true)
        };
        let n = (c0 << 18) | (c1 << 12) | (c2 << 6) | c3;
        out.push((n >> 16) as u8);
        if has2 {
            out.push((n >> 8) as u8);
        }
        if has3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // RFC 4648 test vectors.
        assert_eq!(encode(b""), b"");
        assert_eq!(encode(b"f"), b"Zg==");
        assert_eq!(encode(b"fo"), b"Zm8=");
        assert_eq!(encode(b"foo"), b"Zm9v");
        assert_eq!(encode(b"foob"), b"Zm9vYg==");
        assert_eq!(encode(b"fooba"), b"Zm9vYmE=");
        assert_eq!(encode(b"foobar"), b"Zm9vYmFy");
    }

    #[test]
    fn roundtrip_all_padding_classes() {
        // len % 3 == 0, 1, 2 をそれぞれ網羅。
        for len in 0..200usize {
            let input: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let enc = encode(&input);
            assert!(enc.len().is_multiple_of(4), "len {len}");
            let dec = decode(&enc).expect("decode");
            assert_eq!(dec, input, "roundtrip len {len}");
        }
    }

    #[test]
    fn decode_rejects_bad_length() {
        assert_eq!(decode(b"Zg="), None);
        assert_eq!(decode(b"Z"), None);
    }
}
