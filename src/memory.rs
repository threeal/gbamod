//! Addresses, values, and layouts for reading and writing the GBA's memory.

use std::fmt::Write;

/// A region of the GBA's address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// The region's usual name.
    pub name: &'static str,
    /// The address of the region's first byte.
    pub start: u32,
    /// The size of the region in bytes.
    pub size: u32,
}

/// The GBA's working RAM, in the order a RAM dump holds it: the 256 KiB
/// external RAM, then the 32 KiB internal RAM.
pub const RAM_REGIONS: [Region; 2] = [
    Region {
        name: "EWRAM",
        start: 0x0200_0000,
        size: 0x4_0000,
    },
    Region {
        name: "IWRAM",
        start: 0x0300_0000,
        size: 0x8000,
    },
];

/// The number of bytes a peek prints when no count is given.
pub const DEFAULT_PEEK_COUNT: u32 = 16;

/// How many bytes a hex dump prints per line.
const BYTES_PER_LINE: u32 = 16;

/// The width of a value written to memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Width {
    /// A single byte.
    U8,
    /// A 16-bit halfword.
    U16,
    /// A 32-bit word.
    U32,
}

/// The reason a number or value isn't usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemoryError {
    /// The text isn't a decimal or `0x`-prefixed hexadecimal number that fits
    /// in 32 bits.
    #[error("`{0}` isn't a 32-bit number in decimal or 0x-prefixed hex")]
    InvalidNumber(String),
    /// The value doesn't fit in the width it's written with.
    #[error("value 0x{value:X} doesn't fit in {bits} bits")]
    TooWide {
        /// The value.
        value: u32,
        /// The number of bits it was meant to fit in.
        bits: u32,
    },
}

impl Width {
    /// Returns the number of bytes in a value of this width.
    #[must_use]
    pub const fn bytes(self) -> u32 {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }

    /// Returns `value` if it fits in this width.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::TooWide`] if the value has bits set beyond this
    /// width.
    pub const fn check(self, value: u32) -> Result<u32, MemoryError> {
        let bits = self.bytes() * 8;
        if bits < 32 && value >> bits != 0 {
            return Err(MemoryError::TooWide { value, bits });
        }
        Ok(value)
    }

    /// Formats `value` as hex padded to this width, such as `0x0120`.
    #[must_use]
    pub fn format(self, value: u32) -> String {
        format!("0x{value:0digits$X}", digits = self.bytes() as usize * 2)
    }
}

/// Parses a 32-bit number written in decimal, or in hex with a `0x` prefix.
///
/// # Errors
///
/// Returns [`MemoryError::InvalidNumber`] if the text is neither, or doesn't
/// fit in 32 bits.
pub fn parse_number(text: &str) -> Result<u32, MemoryError> {
    let parsed = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .map_or_else(|| text.parse(), |hex| u32::from_str_radix(hex, 16));
    parsed.map_err(|_| MemoryError::InvalidNumber(text.to_string()))
}

/// Formats bytes read from `address` onward as lines of an address followed
/// by up to 16 hex bytes.
#[must_use]
pub fn hex_dump(address: u32, bytes: &[u8]) -> String {
    let mut dump = String::new();
    for (line, chunk) in (0_u32..).zip(bytes.chunks(BYTES_PER_LINE as usize)) {
        if line > 0 {
            dump.push('\n');
        }
        let line_address = address.wrapping_add(line * BYTES_PER_LINE);
        let _ = write!(dump, "{line_address:08X}:");
        for byte in chunk {
            let _ = write!(dump, " {byte:02X}");
        }
    }
    dump
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_numbers() {
        assert_eq!(parse_number("288"), Ok(288));
        assert_eq!(parse_number("0x0200316C"), Ok(0x0200_316C));
        assert_eq!(parse_number("0XffFFffFF"), Ok(u32::MAX));
    }

    #[test]
    fn reject_invalid_numbers() {
        for text in ["", "0x", "12a", "0xG", "-1", "4294967296", "0x100000000"] {
            assert_eq!(
                parse_number(text),
                Err(MemoryError::InvalidNumber(text.into())),
                "{text}"
            );
        }
        assert_eq!(
            parse_number("x1").unwrap_err().to_string(),
            "`x1` isn't a 32-bit number in decimal or 0x-prefixed hex"
        );
    }

    #[test]
    fn measure_widths() {
        assert_eq!(Width::U8.bytes(), 1);
        assert_eq!(Width::U16.bytes(), 2);
        assert_eq!(Width::U32.bytes(), 4);
    }

    #[test]
    fn check_values_fit() {
        assert_eq!(Width::U8.check(0xFF), Ok(0xFF));
        assert_eq!(Width::U16.check(0xFFFF), Ok(0xFFFF));
        assert_eq!(Width::U32.check(u32::MAX), Ok(u32::MAX));

        let err = Width::U8.check(0x100).unwrap_err();
        assert_eq!(
            err,
            MemoryError::TooWide {
                value: 0x100,
                bits: 8
            }
        );
        assert_eq!(err.to_string(), "value 0x100 doesn't fit in 8 bits");
        assert!(Width::U16.check(0x1_0000).is_err());
    }

    #[test]
    fn format_values() {
        assert_eq!(Width::U8.format(0xA), "0x0A");
        assert_eq!(Width::U16.format(0x120), "0x0120");
        assert_eq!(Width::U32.format(0x0200_316C), "0x0200316C");
    }

    #[test]
    fn dump_hex() {
        let bytes: Vec<u8> = (0..20).collect();
        assert_eq!(
            hex_dump(0x0200_316C, &bytes),
            "0200316C: 00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F\n0200317C: 10 11 12 13"
        );
        assert_eq!(hex_dump(0, &[]), "");
    }

    #[test]
    fn dump_hex_across_end_of_address_space() {
        assert_eq!(
            hex_dump(0xFFFF_FFF8, &[0; 17]),
            "FFFFFFF8: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00\n00000008: 00"
        );
    }

    #[test]
    fn describe_ram_layout() {
        let total: u32 = RAM_REGIONS.iter().map(|region| region.size).sum();
        assert_eq!(total, 288 * 1024);
        assert_eq!(RAM_REGIONS[0].name, "EWRAM");
        assert_eq!(RAM_REGIONS[1].start, 0x0300_0000);
    }
}
