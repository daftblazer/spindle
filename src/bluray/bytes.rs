// SPDX-License-Identifier: GPL-3.0-or-later

//! Big-endian byte/bit writer used by all BDMV structure writers.

#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    buf: Vec<u8>,
    /// Number of bits already used in the last byte (0 = byte aligned).
    bit_pos: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write the `bits` least significant bits of `value`, MSB first.
    pub fn bits(&mut self, bits: u32, value: u64) -> &mut Self {
        debug_assert!(bits <= 64);
        debug_assert!(bits == 64 || value >> bits == 0, "value {value:#x} overflows {bits} bits");
        for i in (0..bits).rev() {
            let bit = ((value >> i) & 1) as u8;
            if self.bit_pos == 0 {
                self.buf.push(0);
            }
            let last = self.buf.last_mut().unwrap();
            *last |= bit << (7 - self.bit_pos);
            self.bit_pos = (self.bit_pos + 1) % 8;
        }
        self
    }

    pub fn flag(&mut self, value: bool) -> &mut Self {
        self.bits(1, value as u64)
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.bits(8, v as u64)
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.bits(16, v as u64)
    }

    pub fn u24(&mut self, v: u32) -> &mut Self {
        self.bits(24, v as u64)
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.bits(32, v as u64)
    }

    pub fn zeros(&mut self, bits: u32) -> &mut Self {
        for _ in 0..bits {
            self.bits(1, 0);
        }
        self
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        if self.bit_pos == 0 {
            self.buf.extend_from_slice(b);
        } else {
            for &x in b {
                self.u8(x);
            }
        }
        self
    }

    /// Write a fixed-length ASCII string, padded with zero bytes.
    pub fn ascii(&mut self, s: &str, len: usize) -> &mut Self {
        let mut b = s.as_bytes().to_vec();
        b.resize(len, 0);
        self.bytes(&b)
    }

    pub fn is_aligned(&self) -> bool {
        self.bit_pos == 0
    }

    /// Current length in bytes (must be aligned).
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        debug_assert!(self.is_aligned());
        self.buf.len()
    }

    /// Pad with zero bytes up to absolute offset `pos`.
    pub fn pad_to(&mut self, pos: usize) -> &mut Self {
        assert!(self.is_aligned());
        assert!(self.buf.len() <= pos, "pad_to({pos}) but already at {}", self.buf.len());
        self.buf.resize(pos, 0);
        self
    }

    /// Overwrite a big-endian u32 at an absolute byte offset.
    pub fn patch_u32(&mut self, pos: usize, v: u32) {
        self.buf[pos..pos + 4].copy_from_slice(&v.to_be_bytes());
    }

    pub fn patch_u16(&mut self, pos: usize, v: u16) {
        self.buf[pos..pos + 2].copy_from_slice(&v.to_be_bytes());
    }

    pub fn patch_u8(&mut self, pos: usize, v: u8) {
        self.buf[pos] = v;
    }

    /// Reserve a u32 length field; returns its position. Use with [`Self::end_len32`].
    pub fn begin_len32(&mut self) -> usize {
        let pos = self.len();
        self.u32(0);
        pos
    }

    /// Fill a u32 length field with the number of bytes written after it.
    pub fn end_len32(&mut self, pos: usize) {
        let len = self.len() - pos - 4;
        self.patch_u32(pos, len as u32);
    }

    pub fn begin_len16(&mut self) -> usize {
        let pos = self.len();
        self.u16(0);
        pos
    }

    pub fn end_len16(&mut self, pos: usize) {
        let len = self.len() - pos - 2;
        self.patch_u16(pos, u16::try_from(len).expect("length exceeds 16 bits"));
    }

    pub fn begin_len8(&mut self) -> usize {
        let pos = self.len();
        self.u8(0);
        pos
    }

    pub fn end_len8(&mut self, pos: usize) {
        let len = self.len() - pos - 1;
        self.patch_u8(pos, u8::try_from(len).expect("length exceeds 8 bits"));
    }

    pub fn into_bytes(self) -> Vec<u8> {
        assert!(self.is_aligned(), "bit writer finished unaligned");
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_pack_msb_first() {
        let mut w = BitWriter::new();
        w.bits(3, 0b101).bits(5, 0b00011).u16(0xBEEF);
        assert_eq!(w.into_bytes(), vec![0b1010_0011, 0xBE, 0xEF]);
    }

    #[test]
    fn length_fields() {
        let mut w = BitWriter::new();
        let p = w.begin_len32();
        w.u16(1).u8(2);
        w.end_len32(p);
        assert_eq!(w.into_bytes(), vec![0, 0, 0, 3, 0, 1, 2]);
    }
}
