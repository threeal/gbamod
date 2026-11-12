//! Records of which bytes of working RAM a game wrote, and the runs of bytes
//! it left alone, where a hook can keep its own values.

use crate::memory::{self, RAM_REGIONS};
use std::fmt;

/// The bytes of working RAM written while writes were tracked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The frames run while writes were tracked.
    pub frames: u32,
    /// One bit per byte of a RAM dump, laid out as
    /// [`memory::ram_addresses`], the low bit of each byte first, set if the
    /// byte was written.
    bits: Vec<u8>,
}

/// A run of bytes in one region of working RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// The address of the first byte.
    pub start: u32,
    /// The number of bytes.
    pub size: u32,
}

impl Written {
    /// Creates a record of `frames` frames from a bitmap with one bit per
    /// byte of a RAM dump, laid out as [`memory::ram_addresses`], the low bit
    /// of each byte first; bytes the bitmap doesn't reach count as written.
    #[must_use]
    pub const fn new(frames: u32, bits: Vec<u8>) -> Self {
        Self { frames, bits }
    }

    /// Returns whether the byte at `index` in a RAM dump was written.
    fn was_written(&self, index: usize) -> bool {
        self.bits
            .get(index / 8)
            .is_none_or(|byte| byte & (1 << (index % 8)) != 0)
    }

    /// Returns every run of bytes never written, longest first, then by
    /// address. A run never crosses from one region into the next.
    #[must_use]
    pub fn unwritten(&self) -> Vec<Span> {
        let mut spans = Vec::new();
        let mut index = 0;
        for region in RAM_REGIONS {
            let mut run: Option<Span> = None;
            for address in region.start..region.start + region.size {
                if self.was_written(index) {
                    spans.extend(run.take());
                } else if let Some(span) = &mut run {
                    span.size += 1;
                } else {
                    run = Some(Span {
                        start: address,
                        size: 1,
                    });
                }
                index += 1;
            }
            spans.extend(run);
        }
        spans.sort_by_key(|span| (std::cmp::Reverse(span.size), span.start));
        spans
    }
}

impl fmt::Display for Span {
    /// Writes the first and last addresses and the size, such as
    /// `0x0203FF00-0x0203FFFF (256 bytes)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let last = self.start + self.size - 1;
        let unit = if self.size == 1 { "byte" } else { "bytes" };
        write!(
            f,
            "0x{:08X}-0x{last:08X} ({} {unit})",
            self.start, self.size
        )
    }
}

/// The size in bytes of a bitmap with one bit per byte of a RAM dump.
pub const BITMAP_SIZE: usize = memory::RAM_DUMP_SIZE as usize / 8;

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a bitmap with every byte written but those in `free`, given
    /// as dump offsets and sizes.
    fn bitmap(free: &[(usize, usize)]) -> Vec<u8> {
        let mut bits = vec![0xFF; BITMAP_SIZE];
        for &(offset, size) in free {
            for index in offset..offset + size {
                bits[index / 8] &= !(1 << (index % 8));
            }
        }
        bits
    }

    #[test]
    fn find_unwritten_runs_longest_first() {
        let written = Written::new(60, bitmap(&[(0x10, 3), (0x3_FF00, 0x100), (0x20, 3)]));
        assert_eq!(
            written.unwritten(),
            [
                Span {
                    start: 0x0203_FF00,
                    size: 0x100
                },
                Span {
                    start: 0x0200_0010,
                    size: 3
                },
                Span {
                    start: 0x0200_0020,
                    size: 3
                },
            ]
        );
    }

    #[test]
    fn split_runs_at_region_ends() {
        let written = Written::new(1, bitmap(&[(0x3_FFFE, 4), (0x4_7FFF, 1)]));
        assert_eq!(
            written.unwritten(),
            [
                Span {
                    start: 0x0203_FFFE,
                    size: 2
                },
                Span {
                    start: 0x0300_0000,
                    size: 2
                },
                Span {
                    start: 0x0300_7FFF,
                    size: 1
                },
            ]
        );
    }

    #[test]
    fn count_missing_bits_as_written() {
        assert_eq!(Written::new(0, Vec::new()).unwritten(), []);
        assert_eq!(
            Written::new(0, vec![0b1111_1101]).unwritten(),
            [Span {
                start: 0x0200_0001,
                size: 1
            }]
        );
    }

    #[test]
    fn display_spans() {
        let span = Span {
            start: 0x0203_FF00,
            size: 0x100,
        };
        assert_eq!(span.to_string(), "0x0203FF00-0x0203FFFF (256 bytes)");
        let span = Span {
            start: 0x0300_0000,
            size: 1,
        };
        assert_eq!(span.to_string(), "0x03000000-0x03000000 (1 byte)");
    }
}
