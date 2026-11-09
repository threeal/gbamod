//! Finding the words in a ROM that hold a pointer.

use crate::game::{Game, GameError};
use crate::patch::ROM_BASE;
use crate::search::MAX_LISTED;
use std::fmt::Write;
use std::path::Path;

/// A word in the ROM that holds the pointer looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ref {
    /// The address of the word.
    pub address: u32,
    /// The value it holds, which differs from the pointer looked for at most
    /// in its low bit.
    pub value: u32,
}

/// Returns every word-aligned word in `rom` that holds `pointer`, with or
/// without its low bit set.
///
/// The low bit is ignored because a stored Thumb function pointer has it set,
/// while the function's own address doesn't.
#[must_use]
pub fn find(rom: &[u8], pointer: u32) -> Vec<Ref> {
    let even = pointer & !1;
    (ROM_BASE..)
        .step_by(4)
        .zip(rom.as_chunks::<4>().0)
        .map(|(address, word)| Ref {
            address,
            value: u32::from_le_bytes(*word),
        })
        .filter(|found| found.value & !1 == even)
        .collect()
}

/// Lists the words in the ROM of the game folder at `dir` that hold
/// `pointer`, for the `refs` command.
///
/// Up to [`MAX_LISTED`] words are listed with the value each holds, followed
/// by a count and, when every word is listed, a `slots` line for a hook.
///
/// # Errors
///
/// Returns a [`GameError`] if the game folder or its ROM can't be read.
pub fn run(dir: &Path, pointer: u32) -> Result<String, GameError> {
    let rom = Game::open(dir)?.read_rom()?;
    Ok(report(&find(&rom.bytes, pointer), pointer))
}

/// Lists the words found for `pointer`, as described in [`run`].
fn report(refs: &[Ref], pointer: u32) -> String {
    let even = pointer & !1;
    let values = format!("0x{even:08X} or 0x{:08X}", even | 1);
    let mut report = String::new();
    for found in refs.iter().take(MAX_LISTED) {
        let _ = writeln!(report, "{:08X}: {:08X}", found.address, found.value);
    }
    match refs.len() {
        0 => {
            let _ = write!(report, "No words hold {values}");
        }
        1 => {
            let _ = write!(report, "1 word holds {values}");
        }
        count if count > MAX_LISTED => {
            let _ = write!(
                report,
                "{count} words hold {values}, listing the first {MAX_LISTED}"
            );
        }
        count => {
            let _ = write!(report, "{count} words hold {values}");
        }
    }
    if (1..=MAX_LISTED).contains(&refs.len()) {
        let slots: Vec<String> = refs
            .iter()
            .map(|found| format!("0x{:08X}", found.address))
            .collect();
        let _ = write!(report, "\nslots = [{}]", slots.join(", "));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest;
    use crate::rom::tests::valid_rom;
    use std::fs;

    #[test]
    fn find_pointer_with_or_without_low_bit() {
        let mut rom = vec![0; 32];
        rom[4..8].copy_from_slice(&0x0804_FD09_u32.to_le_bytes());
        rom[12..16].copy_from_slice(&0x0804_FD08_u32.to_le_bytes());
        rom[16..20].copy_from_slice(&0x0804_FD0A_u32.to_le_bytes());
        // Not word-aligned, so not a pointer the game loads.
        rom[21..25].copy_from_slice(&0x0804_FD09_u32.to_le_bytes());
        // A trailing partial word is ignored.
        rom.extend_from_slice(&[0x09, 0xFD, 0x04]);

        let expected = vec![
            Ref {
                address: 0x0800_0004,
                value: 0x0804_FD09,
            },
            Ref {
                address: 0x0800_000C,
                value: 0x0804_FD08,
            },
        ];
        assert_eq!(find(&rom, 0x0804_FD09), expected);
        assert_eq!(find(&rom, 0x0804_FD08), expected);
    }

    #[test]
    fn report_refs() {
        assert_eq!(
            report(&[], 0x0804_FD09),
            "No words hold 0x0804FD08 or 0x0804FD09"
        );

        let one = [Ref {
            address: 0x0800_0004,
            value: 0x0804_FD09,
        }];
        assert_eq!(
            report(&one, 0x0804_FD08),
            "08000004: 0804FD09\n1 word holds 0x0804FD08 or 0x0804FD09\nslots = [0x08000004]"
        );

        let two = [
            one[0],
            Ref {
                address: 0x08CB_C850,
                value: 0x0804_FD08,
            },
        ];
        assert_eq!(
            report(&two, 0x0804_FD09),
            "08000004: 0804FD09\n08CBC850: 0804FD08\n2 words hold 0x0804FD08 or 0x0804FD09\nslots = [0x08000004, 0x08CBC850]"
        );
    }

    #[test]
    fn report_too_many_refs() {
        let refs: Vec<Ref> = (0..=u32::try_from(MAX_LISTED).unwrap())
            .map(|index| Ref {
                address: ROM_BASE + index * 4,
                value: 0,
            })
            .collect();
        let report = report(&refs, 0);
        assert_eq!(report.lines().count(), MAX_LISTED + 1);
        assert!(report.ends_with(&format!(
            "201 words hold 0x00000000 or 0x00000001, listing the first {MAX_LISTED}"
        )));
    }

    #[test]
    fn run_on_game_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(manifest::FILE_NAME),
            "version = 1\nrom = \"game.gba\"\n",
        )
        .unwrap();
        let mut rom = valid_rom("TEST");
        rom[0x100..0x104].copy_from_slice(&0x0800_0181_u32.to_le_bytes());
        fs::write(dir.path().join("game.gba"), rom).unwrap();

        assert_eq!(
            run(dir.path(), 0x0800_0180).unwrap(),
            "08000100: 08000181\n1 word holds 0x08000180 or 0x08000181\nslots = [0x08000100]"
        );
        assert!(run(&dir.path().join("missing"), 0).is_err());
    }
}
