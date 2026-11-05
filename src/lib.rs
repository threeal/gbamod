//! An emulator-driven toolkit for reverse engineering and patching GBA games.

pub mod game;
pub mod manifest;
pub mod rom;

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
