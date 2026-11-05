//! A game folder: a directory holding a manifest and the ROM it names.

use crate::manifest::{self, Manifest, ManifestError};
use crate::rom::{Rom, RomError};
use std::io;
use std::path::{Path, PathBuf};

/// A game folder whose manifest has been read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// The game folder.
    pub dir: PathBuf,
    /// The manifest at the root of the game folder.
    pub manifest: Manifest,
}

/// The reason a game folder or its ROM couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum GameError {
    /// A file couldn't be read.
    #[error("failed to read `{}`: {source}", path.display())]
    Read {
        /// The file that couldn't be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: io::Error,
    },
    /// The manifest couldn't be parsed.
    #[error("invalid manifest `{}`: {source}", path.display())]
    Manifest {
        /// The manifest file.
        path: PathBuf,
        /// The reason it couldn't be parsed.
        source: ManifestError,
    },
    /// The file named as the ROM isn't a valid GBA ROM image.
    #[error("invalid ROM `{}`: {source}", path.display())]
    Rom {
        /// The ROM file.
        path: PathBuf,
        /// The reason it isn't valid.
        source: RomError,
    },
}

impl Game {
    /// Reads the manifest of the game folder at `dir`.
    ///
    /// # Errors
    ///
    /// Returns a [`GameError`] if the manifest can't be read or parsed.
    pub fn open(dir: &Path) -> Result<Self, GameError> {
        let path = dir.join(manifest::FILE_NAME);
        let text = std::fs::read_to_string(&path).map_err(read_error(&path))?;
        let manifest = Manifest::parse(&text).map_err(|source| GameError::Manifest {
            path: path.clone(),
            source,
        })?;
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    /// Returns the path of the game's ROM.
    #[must_use]
    pub fn rom_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.rom)
    }

    /// Reads and validates the game's ROM.
    ///
    /// # Errors
    ///
    /// Returns a [`GameError`] if the ROM can't be read or isn't a valid GBA
    /// ROM image.
    pub fn read_rom(&self) -> Result<Rom, GameError> {
        let path = self.rom_path();
        let bytes = std::fs::read(&path).map_err(read_error(&path))?;
        Rom::parse(bytes).map_err(|source| GameError::Rom { path, source })
    }
}

/// Returns a function that wraps an I/O error from reading `path`.
fn read_error(path: &Path) -> impl FnOnce(io::Error) -> GameError {
    let path = path.to_path_buf();
    |source| GameError::Read { path, source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::tests::valid_rom;
    use std::fs;

    fn game_dir(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(manifest::FILE_NAME), manifest).unwrap();
        dir
    }

    #[test]
    fn read_game_rom() {
        let dir = game_dir("version = 1\nrom = \"roms/game.gba\"\n");
        fs::create_dir(dir.path().join("roms")).unwrap();
        fs::write(dir.path().join("roms/game.gba"), valid_rom("TEST")).unwrap();

        let game = Game::open(dir.path()).unwrap();
        assert_eq!(game.dir, dir.path());
        assert_eq!(game.rom_path(), dir.path().join("roms/game.gba"));
        assert_eq!(game.read_rom().unwrap().header.title, "TEST");
    }

    #[test]
    fn reject_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let err = Game::open(dir.path()).unwrap_err();
        assert!(matches!(err, GameError::Read { .. }));
        let path = dir.path().join(manifest::FILE_NAME);
        assert!(
            err.to_string()
                .starts_with(&format!("failed to read `{}`: ", path.display()))
        );
    }

    #[test]
    fn reject_invalid_manifest() {
        let dir = game_dir("version = 2\nrom = \"game.gba\"\n");
        let err = Game::open(dir.path()).unwrap_err();
        let path = dir.path().join(manifest::FILE_NAME);
        assert_eq!(
            err.to_string(),
            format!(
                "invalid manifest `{}`: unsupported version 2",
                path.display()
            )
        );
    }

    #[test]
    fn reject_missing_rom() {
        let dir = game_dir("version = 1\nrom = \"game.gba\"\n");
        let err = Game::open(dir.path()).unwrap().read_rom().unwrap_err();
        assert!(matches!(err, GameError::Read { .. }));
    }

    #[test]
    fn reject_invalid_rom() {
        let dir = game_dir("version = 1\nrom = \"game.gba\"\n");
        fs::write(dir.path().join("game.gba"), [0; 4]).unwrap();
        let err = Game::open(dir.path()).unwrap().read_rom().unwrap_err();
        let path = dir.path().join("game.gba");
        assert_eq!(
            err.to_string(),
            format!(
                "invalid ROM `{}`: size of 4 bytes is smaller than the 192-byte header",
                path.display()
            )
        );
    }
}
