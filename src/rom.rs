//! Validation and header parsing of GBA ROM images.

/// The size of the cartridge header at the start of every GBA ROM.
pub const HEADER_SIZE: usize = 0xC0;

/// The largest ROM the GBA can map into its address space.
pub const MAX_SIZE: usize = 32 * 1024 * 1024;

const TITLE: std::ops::Range<usize> = 0xA0..0xAC;
const GAME_CODE: std::ops::Range<usize> = 0xAC..0xB0;
const MAKER_CODE: std::ops::Range<usize> = 0xB0..0xB2;
const FIXED_VALUE_OFFSET: usize = 0xB2;
const FIXED_VALUE: u8 = 0x96;
const VERSION_OFFSET: usize = 0xBC;
const CHECKSUM_OFFSET: usize = 0xBD;

/// A GBA ROM image whose size and header have been validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rom {
    /// The raw bytes of the image.
    pub bytes: Vec<u8>,
    /// The fields read from the cartridge header.
    pub header: Header,
}

/// The identifying fields of a GBA cartridge header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The game title, up to 12 uppercase ASCII characters.
    pub title: String,
    /// The 4-character game code, such as `AXVE`.
    pub game_code: String,
    /// The 2-character maker code, such as `01`.
    pub maker_code: String,
    /// The software version number.
    pub version: u8,
}

/// The reason bytes aren't a valid GBA ROM image.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RomError {
    /// The image is too small to hold a cartridge header.
    #[error("size of {0} bytes is smaller than the {header}-byte header", header = HEADER_SIZE)]
    TooSmall(usize),
    /// The image is larger than the GBA can map.
    #[error("size of {0} bytes is larger than the {max}-byte maximum", max = MAX_SIZE)]
    TooLarge(usize),
    /// The header's fixed byte doesn't hold its required value.
    #[error("header byte 0xB2 is 0x{0:02X} instead of the fixed 0x96")]
    BadFixedValue(u8),
    /// The header's complement checksum doesn't match its contents.
    #[error("header checksum is 0x{found:02X} instead of 0x{expected:02X}")]
    BadChecksum {
        /// The checksum stored in the header.
        found: u8,
        /// The checksum computed from the header's contents.
        expected: u8,
    },
}

impl Rom {
    /// Validates the bytes of a GBA ROM image and reads its header.
    ///
    /// # Errors
    ///
    /// Returns a [`RomError`] if the image's size is out of range, or if its
    /// header's fixed byte or checksum is wrong.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, RomError> {
        if bytes.len() < HEADER_SIZE {
            return Err(RomError::TooSmall(bytes.len()));
        }
        if bytes.len() > MAX_SIZE {
            return Err(RomError::TooLarge(bytes.len()));
        }
        if bytes[FIXED_VALUE_OFFSET] != FIXED_VALUE {
            return Err(RomError::BadFixedValue(bytes[FIXED_VALUE_OFFSET]));
        }
        let expected = header_checksum(&bytes);
        if bytes[CHECKSUM_OFFSET] != expected {
            return Err(RomError::BadChecksum {
                found: bytes[CHECKSUM_OFFSET],
                expected,
            });
        }

        let header = Header {
            title: ascii(&bytes[TITLE]),
            game_code: ascii(&bytes[GAME_CODE]),
            maker_code: ascii(&bytes[MAKER_CODE]),
            version: bytes[VERSION_OFFSET],
        };
        Ok(Self { bytes, header })
    }
}

/// Computes the complement checksum over header bytes 0xA0 to 0xBC.
fn header_checksum(bytes: &[u8]) -> u8 {
    bytes[TITLE.start..CHECKSUM_OFFSET]
        .iter()
        .fold(0_u8, |sum, byte| sum.wrapping_sub(*byte))
        .wrapping_sub(0x19)
}

/// Reads a header text field, which is padded with trailing zero bytes.
fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| char::from(*byte))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal valid ROM image with the given title.
    pub fn valid_rom(title: &str) -> Vec<u8> {
        let mut bytes = vec![0; 0x200];
        bytes[TITLE.start..TITLE.start + title.len()].copy_from_slice(title.as_bytes());
        bytes[GAME_CODE].copy_from_slice(b"ATSE");
        bytes[MAKER_CODE].copy_from_slice(b"01");
        bytes[FIXED_VALUE_OFFSET] = FIXED_VALUE;
        bytes[VERSION_OFFSET] = 2;
        bytes[CHECKSUM_OFFSET] = header_checksum(&bytes);
        bytes
    }

    #[test]
    fn parse_valid_rom() {
        let bytes = valid_rom("TEST");
        let rom = Rom::parse(bytes.clone()).unwrap();
        assert_eq!(rom.bytes, bytes);
        assert_eq!(
            rom.header,
            Header {
                title: "TEST".into(),
                game_code: "ATSE".into(),
                maker_code: "01".into(),
                version: 2,
            }
        );
    }

    #[test]
    fn parse_rom_with_full_length_title() {
        let rom = Rom::parse(valid_rom("TWELVECHARSX")).unwrap();
        assert_eq!(rom.header.title, "TWELVECHARSX");
    }

    #[test]
    fn reject_too_small_rom() {
        let err = Rom::parse(vec![0; HEADER_SIZE - 1]).unwrap_err();
        assert_eq!(err, RomError::TooSmall(HEADER_SIZE - 1));
        assert_eq!(
            err.to_string(),
            "size of 191 bytes is smaller than the 192-byte header"
        );
    }

    #[test]
    fn reject_too_large_rom() {
        let mut bytes = valid_rom("TEST");
        bytes.resize(MAX_SIZE + 1, 0);
        let err = Rom::parse(bytes).unwrap_err();
        assert_eq!(err, RomError::TooLarge(MAX_SIZE + 1));
        assert_eq!(
            err.to_string(),
            "size of 33554433 bytes is larger than the 33554432-byte maximum"
        );
    }

    #[test]
    fn reject_bad_fixed_value() {
        let mut bytes = valid_rom("TEST");
        bytes[FIXED_VALUE_OFFSET] = 0x12;
        let err = Rom::parse(bytes).unwrap_err();
        assert_eq!(err, RomError::BadFixedValue(0x12));
        assert_eq!(
            err.to_string(),
            "header byte 0xB2 is 0x12 instead of the fixed 0x96"
        );
    }

    #[test]
    fn reject_bad_checksum() {
        let mut bytes = valid_rom("TEST");
        let expected = bytes[CHECKSUM_OFFSET];
        bytes[CHECKSUM_OFFSET] = expected.wrapping_add(1);
        let err = Rom::parse(bytes).unwrap_err();
        assert_eq!(
            err,
            RomError::BadChecksum {
                found: expected.wrapping_add(1),
                expected,
            }
        );
        assert_eq!(
            err.to_string(),
            format!(
                "header checksum is 0x{:02X} instead of 0x{expected:02X}",
                expected.wrapping_add(1)
            )
        );
    }
}
