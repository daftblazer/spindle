// SPDX-License-Identifier: GPL-3.0-or-later

//! PG/IG run-length encoding of 8-bit indexed bitmaps.

pub fn encode(indexes: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(indexes.len() / 4);
    for line in indexes.chunks(width).take(height) {
        let mut i = 0;
        while i < line.len() {
            let c = line[i];
            let mut len = 1;
            while i + len < line.len() && line[i + len] == c && len < 16383 {
                len += 1;
            }
            emit(&mut out, c, len);
            i += len;
        }
        out.extend_from_slice(&[0, 0]); // end of line
    }
    out
}

fn emit(out: &mut Vec<u8>, color: u8, len: usize) {
    if color == 0 {
        if len < 64 {
            out.extend_from_slice(&[0, len as u8]);
        } else {
            out.extend_from_slice(&[0, 0x40 | (len >> 8) as u8, len as u8]);
        }
    } else if len <= 2 {
        out.extend(std::iter::repeat_n(color, len));
    } else if len < 64 {
        out.extend_from_slice(&[0, 0x80 | len as u8, color]);
    } else {
        out.extend_from_slice(&[0, 0xC0 | (len >> 8) as u8, len as u8, color]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decoder mirroring libbluray's `_decode_rle`.
    fn decode(mut d: &[u8], width: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut line = Vec::new();
        while !d.is_empty() {
            let (color, len);
            if d[0] != 0 {
                color = d[0];
                len = 1;
                d = &d[1..];
            } else {
                let f = d[1];
                match f >> 6 {
                    0 => (color, len, d) = (0, (f & 0x3F) as usize, &d[2..]),
                    1 => (color, len, d) = (0, (((f & 0x3F) as usize) << 8) | d[2] as usize, &d[3..]),
                    2 => (color, len, d) = (d[2], (f & 0x3F) as usize, &d[3..]),
                    _ => (color, len, d) = (d[3], (((f & 0x3F) as usize) << 8) | d[2] as usize, &d[4..]),
                }
            }
            if len == 0 {
                assert_eq!(line.len(), width);
                out.append(&mut line);
            } else {
                line.extend(std::iter::repeat_n(color, len));
            }
        }
        out
    }

    #[test]
    fn roundtrip() {
        let w = 300;
        let mut img = vec![0u8; w * 3];
        img[5..9].fill(7);
        img[10] = 3;
        img[11] = 3;
        img[w + 1..w + 200].fill(200);
        img[2 * w..3 * w].fill(1);
        assert_eq!(decode(&encode(&img, w, 3), w), img);
    }
}
