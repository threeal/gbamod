//! Finding free space in a ROM, where a hook's code can go.

use crate::game::{Game, GameError};
use crate::memory::Span;
use crate::patch::ROM_BASE;
use std::fmt;
use std::path::Path;

/// The fewest bytes a run needs to count as free space, so the many short
/// runs of `0x00` inside the game's data aren't listed.
pub const MIN_SIZE: u32 = 16;

/// The number of runs listed when no count is given.
pub const DEFAULT_COUNT: u32 = 10;

/// A run of free space in the ROM: bytes that all hold `0x00` or all hold
/// `0xFF`, as a hook's `at` must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Free {
    /// Where the run lies.
    pub span: Span,
    /// The byte every byte of the run holds.
    pub fill: u8,
}

impl fmt::Display for Free {
    /// Writes the span and the byte filling it, such as
    /// `0x08CBEB6C-0x08FFFFFF (3359892 bytes), all 0xFF`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, all 0x{:02X}", self.span, self.fill)
    }
}

/// Returns every run of at least [`MIN_SIZE`] bytes in `rom` that all hold
/// `0x00` or all hold `0xFF`, longest first, then by address.
#[must_use]
pub fn find(rom: &[u8]) -> Vec<Free> {
    let big_enough = |run: &Free| run.span.size >= MIN_SIZE;
    let mut runs = Vec::new();
    let mut run: Option<Free> = None;
    for (address, &fill) in (ROM_BASE..).zip(rom) {
        match &mut run {
            Some(current) if current.fill == fill => current.span.size += 1,
            _ => {
                runs.extend(run.take().filter(big_enough));
                run = (fill == 0x00 || fill == 0xFF).then_some(Free {
                    span: Span {
                        start: address,
                        size: 1,
                    },
                    fill,
                });
            }
        }
    }
    runs.extend(run.filter(big_enough));
    runs.sort_by_key(|run| (std::cmp::Reverse(run.span.size), run.span.start));
    runs
}

/// Lists up to `count` of the longest runs of free space in the ROM of the
/// game folder at `dir`, for the `free` command.
///
/// # Errors
///
/// Returns a [`GameError`] if the game folder or its ROM can't be read.
pub fn run(dir: &Path, count: u32) -> Result<String, GameError> {
    let rom = Game::open(dir)?.read_rom()?;
    Ok(report(&find(&rom.bytes), count))
}

/// Lists up to `count` of `runs` under a heading that counts them all.
fn report(runs: &[Free], count: u32) -> String {
    let heading = match runs.len() {
        0 => {
            return format!("No runs of {MIN_SIZE} or more bytes of 0x00 or 0xFF in the ROM");
        }
        1 => format!("Free in the ROM (1 run of {MIN_SIZE} bytes or more):"),
        total => {
            format!("Free in the ROM ({total} runs of {MIN_SIZE} bytes or more, longest first):")
        }
    };
    let mut lines = vec![heading];
    lines.extend(runs.iter().take(count as usize).map(ToString::to_string));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest;
    use crate::rom::tests::valid_rom;
    use std::fs;

    fn free(start: u32, size: u32, fill: u8) -> Free {
        Free {
            span: Span { start, size },
            fill,
        }
    }

    #[test]
    fn find_runs_longest_first() {
        let mut rom = vec![0x12; 0x100];
        rom[0x10..0x20].fill(0x00);
        rom[0x40..0x60].fill(0xFF);
        rom[0x80..0x90].fill(0xFF);
        assert_eq!(
            find(&rom),
            [
                free(0x0800_0040, 0x20, 0xFF),
                free(0x0800_0010, 0x10, 0x00),
                free(0x0800_0080, 0x10, 0xFF),
            ]
        );
    }

    #[test]
    fn split_runs_where_the_fill_changes() {
        let mut rom = vec![0x00; 0x20];
        rom.extend([0xFF; 0x18]);
        assert_eq!(
            find(&rom),
            [free(0x0800_0000, 0x20, 0x00), free(0x0800_0020, 0x18, 0xFF)]
        );
    }

    #[test]
    fn skip_short_runs_and_other_bytes() {
        let mut rom = vec![0x00; 15];
        rom.push(0x12);
        rom.extend([0xFF; 15]);
        rom.extend([0x55; 0x40]);
        assert_eq!(find(&rom), []);
    }

    #[test]
    fn display_runs() {
        assert_eq!(
            free(0x08CB_EB6C, 0x34_1494, 0xFF).to_string(),
            "0x08CBEB6C-0x08FFFFFF (3413140 bytes), all 0xFF"
        );
    }

    #[test]
    fn report_runs() {
        assert_eq!(
            report(&[], 10),
            "No runs of 16 or more bytes of 0x00 or 0xFF in the ROM"
        );
        let one = [free(0x0800_0040, 0x20, 0xFF)];
        assert_eq!(
            report(&one, 10),
            "Free in the ROM (1 run of 16 bytes or more):\n\
             0x08000040-0x0800005F (32 bytes), all 0xFF"
        );
        let two = [one[0], free(0x0800_0010, 0x10, 0x00)];
        assert_eq!(
            report(&two, 1),
            "Free in the ROM (2 runs of 16 bytes or more, longest first):\n\
             0x08000040-0x0800005F (32 bytes), all 0xFF"
        );
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
        rom[..0xA0].fill(0x12);
        rom[0xC0..].fill(0x12);
        rom.extend([0xFF; 0x40]);
        fs::write(dir.path().join("game.gba"), rom).unwrap();

        assert_eq!(
            run(dir.path(), DEFAULT_COUNT).unwrap(),
            "Free in the ROM (1 run of 16 bytes or more):\n\
             0x08000200-0x0800023F (64 bytes), all 0xFF"
        );
        assert!(run(&dir.path().join("missing"), DEFAULT_COUNT).is_err());
    }
}
