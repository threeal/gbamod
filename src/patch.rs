//! Byte patches to a ROM, and the IPS files that carry them.

use crate::rom::MAX_SIZE;
use std::ops::Range;

/// The address the ROM is mapped at.
pub const ROM_BASE: u32 = 0x0800_0000;

/// The size of the part of a ROM an IPS patch can reach with its 24-bit
/// offsets.
pub const IPS_REACH: usize = 1 << 24;

/// The most bytes one IPS record holds.
const IPS_MAX_RECORD: usize = 0xFFFF;

/// The offset that would read as the `EOF` marker if a record started there.
const IPS_EOF_OFFSET: usize = 0x45_4F46;

/// Bytes to write over a ROM at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// What the patch is called in messages, such as `patch 1`.
    pub name: String,
    /// The address of the first byte to write.
    pub at: u32,
    /// The bytes to write.
    pub bytes: Vec<u8>,
    /// What the ROM must hold there before the patch.
    pub expect: Expect,
}

/// What a patch expects the ROM to hold where it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// Anything.
    Any,
    /// These bytes, as many as the patch writes.
    Bytes(Vec<u8>),
    /// Free space: bytes that are all `0xFF` or all `0x00`.
    Free,
}

/// The reason text isn't a list of hex bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` isn't hex bytes; expected pairs of hex digits such as `79 6A 00 08`")]
pub struct HexError(pub String);

/// Parses bytes written as pairs of hex digits, which spaces may separate,
/// such as `79 6A 00 08` or `796A0008`.
///
/// # Errors
///
/// Returns a [`HexError`] if the text holds anything but pairs of hex digits
/// and spaces, or holds no bytes at all.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, HexError> {
    let error = || HexError(text.to_string());
    let mut bytes = Vec::new();
    for word in text.split_whitespace() {
        let digits: Vec<u32> = word
            .chars()
            .map(|c| c.to_digit(16))
            .collect::<Option<_>>()
            .ok_or_else(error)?;
        if !digits.len().is_multiple_of(2) {
            return Err(error());
        }
        bytes.extend(
            digits
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[high, low]| u8::try_from(high << 4 | low).unwrap_or_default()),
        );
    }
    if bytes.is_empty() {
        return Err(error());
    }
    Ok(bytes)
}

/// The reason a patch can't be applied to a ROM.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// The patch doesn't start in the ROM's address range.
    #[error("the address isn't in the ROM; expected 0x08000000 to 0x09FFFFFF")]
    Outside,
    /// The patch runs past the end of the ROM.
    #[error("it runs past the end of the ROM at 0x{0:08X}")]
    PastEnd(usize),
    /// The patch runs past the part of the ROM an IPS patch can reach.
    #[error("it runs past 0x{:08X}, the furthest an IPS patch reaches", ROM_BASE as usize + IPS_REACH)]
    PastIps,
    /// The patch expects a different number of bytes than it writes.
    #[error("it expects {expect} bytes but writes {bytes}")]
    Length {
        /// The number of bytes it expects.
        expect: usize,
        /// The number of bytes it writes.
        bytes: usize,
    },
    /// The patch expects free space where the ROM holds something else.
    #[error("the byte at 0x{address:08X} is 0x{found:02X}, but free space is all 0xFF or all 0x00")]
    NotFree {
        /// The address of the first byte that isn't free.
        address: usize,
        /// The byte the ROM holds there.
        found: u8,
    },
    /// The ROM doesn't hold the bytes the patch expects.
    #[error("the byte at 0x{address:08X} is 0x{found:02X}, not the expected 0x{expected:02X}")]
    Mismatch {
        /// The address of the first byte that differs.
        address: usize,
        /// The byte the ROM holds there.
        found: u8,
        /// The byte the patch expects there.
        expected: u8,
    },
    /// The patch writes over bytes an earlier patch writes.
    #[error("it overlaps {0}")]
    Overlap(String),
}

/// A problem with one patch.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{name} at 0x{at:08X}: {problem}")]
pub struct PatchProblem {
    /// The patch's name.
    pub name: String,
    /// The patch's address.
    pub at: u32,
    /// What's wrong with it.
    pub problem: Problem,
}

/// The reason patches can't be applied to a ROM: one problem per patch that
/// can't be.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct PatchError(pub Vec<PatchProblem>);

/// Returns a copy of `rom` with `patches` written over it, after checking
/// that each one lies in the ROM, within an IPS patch's reach, over the bytes
/// it expects, and clear of every other patch.
///
/// # Errors
///
/// Returns a [`PatchError`] listing every patch that fails a check, in
/// order, in which case nothing is written.
pub fn apply(rom: &[u8], patches: &[Patch]) -> Result<Vec<u8>, PatchError> {
    let mut problems = Vec::new();
    let mut spans: Vec<(&str, Range<usize>)> = Vec::new();
    for patch in patches {
        match check(rom, patch, &spans) {
            Ok(span) => spans.push((&patch.name, span)),
            Err(problem) => problems.push(PatchProblem {
                name: patch.name.clone(),
                at: patch.at,
                problem,
            }),
        }
    }
    if !problems.is_empty() {
        return Err(PatchError(problems));
    }
    let mut result = rom.to_vec();
    for (patch, (_, span)) in patches.iter().zip(spans) {
        result[span].copy_from_slice(&patch.bytes);
    }
    Ok(result)
}

/// Checks one patch against the ROM and the spans of the valid patches
/// before it, returning the span of the ROM it writes.
fn check(
    rom: &[u8],
    patch: &Patch,
    spans: &[(&str, Range<usize>)],
) -> Result<Range<usize>, Problem> {
    let start = offset(patch.at).ok_or(Problem::Outside)?;
    let span = start..start + patch.bytes.len();
    if span.end > rom.len() {
        return Err(Problem::PastEnd(ROM_BASE as usize + rom.len()));
    }
    if span.end > IPS_REACH {
        return Err(Problem::PastIps);
    }
    let found = &rom[span.clone()];
    let address = |index: usize| ROM_BASE as usize + start + index;
    match &patch.expect {
        Expect::Any => {}
        Expect::Bytes(expect) => {
            if expect.len() != patch.bytes.len() {
                return Err(Problem::Length {
                    expect: expect.len(),
                    bytes: patch.bytes.len(),
                });
            }
            if let Some((index, (&found, &expected))) = (0..)
                .zip(found.iter().zip(expect))
                .find(|(_, (found, expected))| found != expected)
            {
                return Err(Problem::Mismatch {
                    address: address(index),
                    found,
                    expected,
                });
            }
        }
        Expect::Free => {
            if let Some((index, &found)) = (0..)
                .zip(found)
                .find(|&(_, &byte)| byte != found[0] || (byte != 0x00 && byte != 0xFF))
            {
                return Err(Problem::NotFree {
                    address: address(index),
                    found,
                });
            }
        }
    }
    if let Some((name, _)) = spans
        .iter()
        .find(|(_, other)| other.start < span.end && span.start < other.end)
    {
        return Err(Problem::Overlap((*name).to_string()));
    }
    Ok(span)
}

/// Returns the offset into the ROM of an address in its address range.
fn offset(address: u32) -> Option<usize> {
    let offset = address.checked_sub(ROM_BASE)? as usize;
    (offset < MAX_SIZE).then_some(offset)
}

/// Writes an IPS patch that turns the original ROM into the patched `rom`,
/// with records for the bytes each of `patches` wrote, in order. The patches
/// must have been applied by [`apply`].
///
/// A record that would start at the offset that reads as the `EOF` marker
/// starts one byte earlier instead, and a patch longer than a record holds is
/// split across several.
#[must_use]
pub fn ips(rom: &[u8], patches: &[Patch]) -> Vec<u8> {
    let mut ips = b"PATCH".to_vec();
    for patch in patches {
        let start = offset(patch.at).unwrap_or_default();
        let end = start + patch.bytes.len();
        let mut offset = start;
        while offset < end {
            if offset == IPS_EOF_OFFSET {
                offset -= 1;
            }
            let size = (end - offset).min(IPS_MAX_RECORD);
            ips.extend_from_slice(&be_bytes(offset)[1..]);
            ips.extend_from_slice(&be_bytes(size)[2..]);
            ips.extend_from_slice(&rom[offset..offset + size]);
            offset += size;
        }
    }
    ips.extend_from_slice(b"EOF");
    ips
}

/// Returns the low 32 bits of a number as big-endian bytes.
#[allow(clippy::cast_possible_truncation)]
const fn be_bytes(number: usize) -> [u8; 4] {
    (number as u32).to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(at: u32, bytes: &[u8], expect: Option<&[u8]>) -> Patch {
        Patch {
            name: format!("patch at {at:X}"),
            at,
            bytes: bytes.to_vec(),
            expect: expect.map_or(Expect::Any, |expect| Expect::Bytes(expect.to_vec())),
        }
    }

    fn problems(err: &PatchError) -> Vec<(String, Problem)> {
        err.0
            .iter()
            .map(|p| (p.name.clone(), p.problem.clone()))
            .collect()
    }

    #[test]
    fn parse_hex_bytes() {
        assert_eq!(parse_hex("79 6A 00 08"), Ok(vec![0x79, 0x6A, 0x00, 0x08]));
        assert_eq!(parse_hex("796a0008"), Ok(vec![0x79, 0x6A, 0x00, 0x08]));
        assert_eq!(parse_hex(" 01F0\n CB08 "), Ok(vec![0x01, 0xF0, 0xCB, 0x08]));
    }

    #[test]
    fn reject_bad_hex() {
        for text in ["", "  ", "7", "79 6", "0x79", "+1", "GG"] {
            assert_eq!(parse_hex(text), Err(HexError(text.into())), "{text}");
        }
        assert_eq!(
            HexError("7".into()).to_string(),
            "`7` isn't hex bytes; expected pairs of hex digits such as `79 6A 00 08`"
        );
    }

    #[test]
    fn apply_patches() {
        let rom = [0xFF; 16];
        let patches = [
            patch(0x0800_0004, &[1, 2], Some(&[0xFF, 0xFF])),
            patch(0x0800_0006, &[3], None),
            patch(0x0800_000F, &[4], Some(&[0xFF])),
        ];
        let modded = apply(&rom, &patches).unwrap();
        assert_eq!(
            modded,
            [
                0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 4
            ]
        );
        assert_eq!(apply(&rom, &[]).unwrap(), rom);
    }

    /// Names each patch by its number, from 1.
    fn numbered<const N: usize>(mut patches: [Patch; N]) -> [Patch; N] {
        for (number, patch) in (1..).zip(&mut patches) {
            patch.name = format!("patch {number}");
        }
        patches
    }

    fn free(at: u32, bytes: &[u8]) -> Patch {
        Patch {
            expect: Expect::Free,
            ..patch(at, bytes, None)
        }
    }

    #[test]
    fn reject_bad_patches() {
        let rom = [0, 1, 2, 3, 4, 5, 6, 7];
        let patches = numbered([
            patch(0x0800_0000, &[9, 9], Some(&[0, 1])),
            patch(0x07FF_FFFF, &[9], None),
            patch(0x0A00_0000, &[9], None),
            patch(0x0800_0006, &[9, 9, 9], None),
            patch(0x0800_0002, &[9, 9], Some(&[2])),
            patch(0x0800_0002, &[9, 9], Some(&[2, 4])),
            patch(0x0800_0001, &[9, 9], None),
            patch(0x0800_0003, &[9], Some(&[3])),
        ]);
        let err = apply(&rom, &patches).unwrap_err();
        assert_eq!(
            problems(&err),
            [
                ("patch 2".into(), Problem::Outside),
                ("patch 3".into(), Problem::Outside),
                ("patch 4".into(), Problem::PastEnd(0x0800_0008)),
                (
                    "patch 5".into(),
                    Problem::Length {
                        expect: 1,
                        bytes: 2
                    }
                ),
                (
                    "patch 6".into(),
                    Problem::Mismatch {
                        address: 0x0800_0003,
                        found: 3,
                        expected: 4
                    }
                ),
                ("patch 7".into(), Problem::Overlap("patch 1".into())),
            ]
        );
        assert_eq!(
            err.to_string(),
            "patch 2 at 0x07FFFFFF: the address isn't in the ROM; expected 0x08000000 to 0x09FFFFFF\n\
             patch 3 at 0x0A000000: the address isn't in the ROM; expected 0x08000000 to 0x09FFFFFF\n\
             patch 4 at 0x08000006: it runs past the end of the ROM at 0x08000008\n\
             patch 5 at 0x08000002: it expects 1 bytes but writes 2\n\
             patch 6 at 0x08000002: the byte at 0x08000003 is 0x03, not the expected 0x04\n\
             patch 7 at 0x08000001: it overlaps patch 1"
        );
    }

    #[test]
    fn apply_patches_over_free_space() {
        let rom = [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x7F];
        let patches = [
            free(0x0800_0000, &[1, 2, 3]),
            free(0x0800_0003, &[4, 5]),
            free(0x0800_0006, &[]),
        ];
        assert_eq!(apply(&rom, &patches).unwrap(), [1, 2, 3, 4, 5, 0x7F]);
    }

    #[test]
    fn reject_patches_over_used_space() {
        let rom = [0xFF, 0xFF, 0x00, 0x7F, 0x7F];
        let patches = numbered([free(0x0800_0000, &[9, 9, 9]), free(0x0800_0003, &[9, 9])]);
        let err = apply(&rom, &patches).unwrap_err();
        assert_eq!(
            problems(&err),
            [
                (
                    "patch 1".into(),
                    Problem::NotFree {
                        address: 0x0800_0002,
                        found: 0x00
                    }
                ),
                (
                    "patch 2".into(),
                    Problem::NotFree {
                        address: 0x0800_0003,
                        found: 0x7F
                    }
                ),
            ]
        );
        assert_eq!(
            err.0[0].to_string(),
            "patch 1 at 0x08000000: the byte at 0x08000002 is 0x00, but free space is all 0xFF or all 0x00"
        );
    }

    #[test]
    fn reject_patch_past_ips_reach() {
        let rom = vec![0; IPS_REACH + 4];
        let err = apply(&rom, &numbered([patch(0x08FF_FFFE, &[1, 2, 3], None)])).unwrap_err();
        assert_eq!(problems(&err), [("patch 1".into(), Problem::PastIps)]);
        assert_eq!(
            err.to_string(),
            "patch 1 at 0x08FFFFFE: it runs past 0x09000000, the furthest an IPS patch reaches"
        );
        assert!(apply(&rom, &[patch(0x08FF_FFFE, &[1, 2], None)]).is_ok());
    }

    #[test]
    fn write_ips_records() {
        let rom = [0xFF; 16];
        let patches = [
            patch(0x0800_000C, &[1, 2, 3], None),
            patch(0x0800_0001, &[4], None),
        ];
        let modded = apply(&rom, &patches).unwrap();
        assert_eq!(
            ips(&modded, &patches),
            b"PATCH\x00\x00\x0C\x00\x03\x01\x02\x03\x00\x00\x01\x00\x01\x04EOF"
        );
        assert_eq!(ips(&rom, &[]), b"PATCHEOF");
    }

    #[test]
    fn split_long_records_and_avoid_eof_offset() {
        let rom = vec![0; IPS_EOF_OFFSET + 8];
        let start = IPS_EOF_OFFSET - IPS_MAX_RECORD;
        let bytes = vec![7; IPS_MAX_RECORD + 2];
        let patches = [patch(
            ROM_BASE + u32::try_from(start).unwrap(),
            &bytes,
            None,
        )];
        let modded = apply(&rom, &patches).unwrap();
        let ips = ips(&modded, &patches);

        let mut expected = b"PATCH\x44\x4F\x47\xFF\xFF".to_vec();
        expected.extend(vec![7; IPS_MAX_RECORD]);
        expected.extend_from_slice(b"\x45\x4F\x45\x00\x03\x07\x07\x07EOF");
        assert_eq!(ips, expected);
    }
}
