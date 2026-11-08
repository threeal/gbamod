//! Disassembling the GBA's ARM and Thumb code.

/// The number of instructions a disassembly prints when no count is given.
pub const DEFAULT_COUNT: u32 = 16;

/// The instruction set code is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 32-bit ARM instructions.
    Arm,
    /// 16-bit Thumb instructions, where a `bl` takes two.
    Thumb,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// The size of the instruction in bytes.
    pub size: u32,
    /// The encoded instruction. A Thumb `bl` holds its first half in the high
    /// 16 bits.
    pub opcode: u32,
    /// The instruction in assembly.
    pub text: String,
}

/// The reason code can't be disassembled from an address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ARM code starts at a multiple of 4, but 0x{0:08X} isn't one")]
pub struct UnalignedError(pub u32);

/// Returns where code at `address` starts and the mode it's read in: ARM
/// if `arm` is set, or else Thumb, ignoring the low bit that marks a Thumb
/// function pointer.
///
/// # Errors
///
/// Returns an [`UnalignedError`] if `arm` is set and the address isn't a
/// multiple of 4.
pub const fn start(address: u32, arm: bool) -> Result<(u32, Mode), UnalignedError> {
    if !arm {
        return Ok((address & !1, Mode::Thumb));
    }
    if !address.is_multiple_of(4) {
        return Err(UnalignedError(address));
    }
    Ok((address, Mode::Arm))
}

/// Formats an instruction at `address` as its address, encoding, and text.
#[must_use]
pub fn line(address: u32, mode: Mode, instruction: &Instruction) -> String {
    let opcode = instruction.opcode;
    let encoding = match (mode, instruction.size) {
        (Mode::Thumb, 2) => format!("{opcode:04X}"),
        (Mode::Thumb, _) => format!("{:04X} {:04X}", opcode >> 16, opcode & 0xFFFF),
        (Mode::Arm, _) => format!("{opcode:08X}"),
    };
    row(address, &encoding, &instruction.text)
}

/// Formats a line of a listing as an address, an encoding, and a text, which
/// may be empty.
#[must_use]
pub fn row(address: u32, encoding: &str, text: &str) -> String {
    if text.is_empty() {
        format!("{address:08X}: {encoding}")
    } else {
        format!("{address:08X}: {encoding:<9}  {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(size: u32, opcode: u32, text: &str) -> Instruction {
        Instruction {
            size,
            opcode,
            text: text.into(),
        }
    }

    #[test]
    fn start_thumb_code() {
        assert_eq!(start(0x0804_FD08, false), Ok((0x0804_FD08, Mode::Thumb)));
        assert_eq!(start(0x0804_FD09, false), Ok((0x0804_FD08, Mode::Thumb)));
    }

    #[test]
    fn start_arm_code() {
        assert_eq!(start(0x0800_0000, true), Ok((0x0800_0000, Mode::Arm)));
        let err = start(0x0800_0002, true).unwrap_err();
        assert_eq!(err, UnalignedError(0x0800_0002));
        assert_eq!(
            err.to_string(),
            "ARM code starts at a multiple of 4, but 0x08000002 isn't one"
        );
    }

    #[test]
    fn format_lines() {
        assert_eq!(
            line(
                0x0804_FD08,
                Mode::Thumb,
                &instruction(2, 0xB500, "push {lr}")
            ),
            "0804FD08: B500       push {lr}"
        );
        assert_eq!(
            line(
                0x0800_FA00,
                Mode::Thumb,
                &instruction(4, 0xF7F6_FF3A, "bl 0x08006878")
            ),
            "0800FA00: F7F6 FF3A  bl 0x08006878"
        );
        assert_eq!(
            line(
                0x0800_0000,
                Mode::Arm,
                &instruction(4, 0xEA00_002E, "b 0x080000C0")
            ),
            "08000000: EA00002E   b 0x080000C0"
        );
        assert_eq!(
            line(0x0800_0000, Mode::Thumb, &instruction(2, 0xFFFF, "")),
            "08000000: FFFF"
        );
    }
}
