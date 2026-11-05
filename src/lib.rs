//! An emulator-driven toolkit for reverse engineering and patching GBA games.

#[cfg(not(tarpaulin_include))]
pub mod emulator;
pub mod game;
pub mod manifest;
pub mod rom;
pub mod screen;

use game::{Game, GameError};
use std::path::Path;

/// Describes the ROM of the game folder at `dir`, for the `info` command.
///
/// # Errors
///
/// Returns a [`GameError`] if the game folder's manifest or ROM can't be read
/// or isn't valid.
pub fn info(dir: &Path) -> Result<String, GameError> {
    let game = Game::open(dir)?;
    let rom = game.read_rom()?;
    let header = &rom.header;
    Ok(format!(
        "ROM:        {}\nTitle:      {}\nGame code:  {}\nMaker code: {}\nVersion:    {}\nSize:       {} bytes",
        game.manifest.rom.display(),
        header.title,
        header.game_code,
        header.maker_code,
        header.version,
        rom.bytes.len(),
    ))
}

/// The reason the `boot` command failed.
#[cfg(not(tarpaulin_include))]
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// The game folder or its ROM couldn't be read.
    #[error(transparent)]
    Game(#[from] GameError),
    /// The emulator couldn't load the ROM.
    #[error(transparent)]
    Load(#[from] emulator::LoadError),
    /// The screenshot couldn't be written.
    #[error("failed to write `{}`: {source}", path.display())]
    Write {
        /// The screenshot file.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Boots the ROM of the game folder at `dir` with no window, runs it for
/// `frames` frames, and saves the screen as a PNG at `screenshot`, for the
/// `boot` command.
///
/// # Errors
///
/// Returns a [`BootError`] if the game folder or its ROM can't be read, the
/// emulator can't load the ROM, or the screenshot can't be written.
#[cfg(not(tarpaulin_include))]
pub fn boot(dir: &Path, frames: u32, screenshot: &Path) -> Result<String, BootError> {
    let rom = Game::open(dir)?.read_rom()?;
    let mut emulator = emulator::Emulator::new(&rom)?;
    emulator.run_frames(frames);
    std::fs::write(screenshot, emulator.screen().encode_png()).map_err(|source| {
        BootError::Write {
            path: screenshot.to_path_buf(),
            source,
        }
    })?;
    Ok(format!(
        "Ran {frames} frames and saved the screen to {}",
        screenshot.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::tests::valid_rom;
    use std::fs;

    #[test]
    fn describe_game_rom() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(manifest::FILE_NAME),
            "version = 1\nrom = \"game.gba\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("game.gba"), valid_rom("TEST")).unwrap();

        assert_eq!(
            info(dir.path()).unwrap(),
            "ROM:        game.gba\nTitle:      TEST\nGame code:  ATSE\nMaker code: 01\nVersion:    2\nSize:       512 bytes"
        );
    }

    #[test]
    fn reject_invalid_game() {
        let dir = tempfile::tempdir().unwrap();
        assert!(info(dir.path()).is_err());
    }
}
