//! An emulator-driven toolkit for reverse engineering and patching GBA games.

pub mod asm;
pub mod debug;
pub mod disasm;
#[cfg(not(tarpaulin_include))]
pub mod emulator;
pub mod game;
pub mod keys;
pub mod manifest;
pub mod memory;
pub mod patch;
pub mod pixels;
pub mod rom;
pub mod screen;
pub mod search;
pub mod session;

use game::{Game, GameError};
use patch::PatchError;
use std::path::{Path, PathBuf};

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

/// The reason the `build` command failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The game folder or its ROM couldn't be read.
    #[error(transparent)]
    Game(#[from] GameError),
    /// The manifest names no file to write.
    #[error("the manifest names no output; add `rom` or `ips` under `[output]`")]
    NoOutput,
    /// An output file is the game's ROM.
    #[error("the output `{}` is the game's ROM, which gbamod only reads", .0.display())]
    OverwriteRom(PathBuf),
    /// Some patches can't be applied to the ROM.
    #[error("the patches can't be applied:\n{0}")]
    Patch(#[from] PatchError),
    /// An output file couldn't be written.
    #[error("failed to write `{}`: {source}", path.display())]
    Write {
        /// The output file.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Applies the manifest's patches to the ROM of the game folder at `dir` and
/// writes the outputs it names, for the `build` command.
///
/// Nothing is written unless every patch can be applied, and the ROM itself
/// is only read.
///
/// # Errors
///
/// Returns a [`BuildError`] if the game folder or its ROM can't be read, the
/// manifest names no output or names the ROM as one, a patch can't be
/// applied, or an output can't be written.
pub fn build(dir: &Path) -> Result<String, BuildError> {
    let game = Game::open(dir)?;
    let manifest = &game.manifest;
    let output = &manifest.output;
    if output.rom.is_none() && output.ips.is_none() {
        return Err(BuildError::NoOutput);
    }
    let rom_path = std::fs::canonicalize(game.rom_path()).ok();
    if let Some(path) =
        output.rom.iter().chain(&output.ips).find(|path| {
            rom_path.is_some() && std::fs::canonicalize(dir.join(path)).ok() == rom_path
        })
    {
        return Err(BuildError::OverwriteRom(path.clone()));
    }
    let rom = game.read_rom()?;
    let patched = patch::apply(&rom.bytes, &manifest.patches)?;

    let mut lines: Vec<String> = manifest
        .patches
        .iter()
        .map(|patch| {
            format!(
                "Patched {} byte(s) at 0x{:08X}",
                patch.bytes.len(),
                patch.at
            )
        })
        .collect();
    if let Some(path) = &output.rom {
        write_output(&dir.join(path), &patched)?;
        lines.push(format!(
            "Wrote {} ({} bytes)",
            path.display(),
            patched.len()
        ));
    }
    if let Some(path) = &output.ips {
        let ips = patch::ips(&patched, &manifest.patches);
        write_output(&dir.join(path), &ips)?;
        lines.push(format!("Wrote {} ({} bytes)", path.display(), ips.len()));
    }
    Ok(lines.join("\n"))
}

/// Writes an output file, creating the directories it's in.
fn write_output(path: &Path, bytes: &[u8]) -> Result<(), BuildError> {
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, bytes))
        .map_err(|source| BuildError::Write {
            path: path.to_path_buf(),
            source,
        })
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

/// The hidden subcommand that runs a session in the background, launched by
/// [`start`].
pub const SERVE_COMMAND: &str = "serve";

/// The reason the `start` command failed.
#[cfg(not(tarpaulin_include))]
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The game folder or its ROM couldn't be read.
    #[error(transparent)]
    Game(#[from] GameError),
    /// A session is already listening in the game folder.
    #[error("a session is already running in this folder")]
    AlreadyRunning,
    /// The background process couldn't be launched or checked on.
    #[error("failed to launch the session: {0}")]
    Spawn(#[from] std::io::Error),
    /// The background process exited before it started listening.
    #[error("the session exited before it was ready ({0})")]
    Exited(std::process::ExitStatus),
    /// The background process didn't start listening in time.
    #[error("the session didn't become ready within 10 seconds")]
    Timeout,
}

/// Launches a session for the game folder at `dir` in a background process,
/// and waits until it's listening, for the `start` command.
///
/// # Errors
///
/// Returns a [`StartError`] if a session is already running, the game folder
/// or its ROM can't be read, or the background process fails to start
/// listening.
#[cfg(not(tarpaulin_include))]
pub fn start(dir: &Path) -> Result<String, StartError> {
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let socket = session::socket_path(dir);
    if UnixStream::connect(&socket).is_ok() {
        return Err(StartError::AlreadyRunning);
    }
    // Check the ROM here, where errors can still be reported to the user.
    Game::open(dir)?.read_rom()?;
    // A socket left behind by a session that was killed is no longer in use.
    let _ = std::fs::remove_file(&socket);

    let mut child = Command::new(std::env::current_exe()?)
        .arg(SERVE_COMMAND)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    for _ in 0..200 {
        if UnixStream::connect(&socket).is_ok() {
            return Ok(format!("Started a session (process {})", child.id()));
        }
        if let Some(status) = child.try_wait()? {
            return Err(StartError::Exited(status));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(StartError::Timeout)
}

/// The reason a background session couldn't run.
#[cfg(not(tarpaulin_include))]
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The game folder or its ROM couldn't be read.
    #[error(transparent)]
    Game(#[from] GameError),
    /// The emulator couldn't load the ROM.
    #[error(transparent)]
    Load(#[from] emulator::LoadError),
    /// The session socket couldn't be created.
    #[error("failed to listen on the session socket: {0}")]
    Listen(#[from] std::io::Error),
}

/// Runs a session for the game folder at `dir` until it's stopped, for the
/// hidden subcommand that [`start`] launches.
///
/// # Errors
///
/// Returns a [`ServeError`] if the game folder or its ROM can't be read, the
/// emulator can't load the ROM, or the session socket can't be created.
#[cfg(not(tarpaulin_include))]
pub fn serve(dir: &Path) -> Result<(), ServeError> {
    let rom = Game::open(dir)?.read_rom()?;
    let emulator = emulator::Emulator::new(&rom)?;
    let socket = session::socket_path(dir);
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    session::serve(&listener, &mut session::Session::new(emulator));
    // Remove the socket while still listening, so no new command can connect
    // to a session that's about to exit.
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::tests::valid_rom;
    use std::fs;

    fn game_dir(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(manifest::FILE_NAME),
            format!("version = 1\nrom = \"game.gba\"\n{manifest}"),
        )
        .unwrap();
        fs::write(dir.path().join("game.gba"), valid_rom("TEST")).unwrap();
        dir
    }

    const PATCHES: &str = "[[patch]]\n\
        at = 0x08000100\n\
        expect = \"00 00\"\n\
        bytes = \"10 B5\"\n\
        [[patch]]\n\
        at = 0x08000004\n\
        bytes = \"AA\"\n";

    #[test]
    fn build_patched_rom_and_ips() {
        let dir = game_dir(&format!(
            "[output]\nrom = \"out/mod.gba\"\nips = \"out/mod.ips\"\n{PATCHES}"
        ));
        assert_eq!(
            build(dir.path()).unwrap(),
            "Patched 2 byte(s) at 0x08000100\n\
             Patched 1 byte(s) at 0x08000004\n\
             Wrote out/mod.gba (512 bytes)\n\
             Wrote out/mod.ips (21 bytes)"
        );
        let mut expected = valid_rom("TEST");
        expected[0x100..0x102].copy_from_slice(&[0x10, 0xB5]);
        expected[4] = 0xAA;
        assert_eq!(fs::read(dir.path().join("out/mod.gba")).unwrap(), expected);
        assert_eq!(
            fs::read(dir.path().join("out/mod.ips")).unwrap(),
            b"PATCH\x00\x01\x00\x00\x02\x10\xB5\x00\x00\x04\x00\x01\xAAEOF"
        );
        assert_eq!(
            fs::read(dir.path().join("game.gba")).unwrap(),
            valid_rom("TEST")
        );
    }

    #[test]
    fn build_only_ips() {
        let dir = game_dir("[output]\nips = \"mod.ips\"\n");
        assert_eq!(build(dir.path()).unwrap(), "Wrote mod.ips (8 bytes)");
        assert_eq!(fs::read(dir.path().join("mod.ips")).unwrap(), b"PATCHEOF");
    }

    #[test]
    fn reject_build_without_output() {
        let dir = game_dir(PATCHES);
        let err = build(dir.path()).unwrap_err();
        assert!(matches!(err, BuildError::NoOutput));
        assert_eq!(
            err.to_string(),
            "the manifest names no output; add `rom` or `ips` under `[output]`"
        );
    }

    #[test]
    fn reject_build_over_rom() {
        let dir = game_dir("[output]\nrom = \"./game.gba\"\n");
        let err = build(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the output `./game.gba` is the game's ROM, which gbamod only reads"
        );
        assert_eq!(
            fs::read(dir.path().join("game.gba")).unwrap(),
            valid_rom("TEST")
        );
    }

    #[test]
    fn reject_build_with_bad_patches() {
        let dir = game_dir(
            "[output]\nrom = \"mod.gba\"\n[[patch]]\nat = 0x08000100\nexpect = \"01\"\nbytes = \"02\"\n",
        );
        let err = build(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the patches can't be applied:\n\
             patch 1 at 0x08000100: the byte at 0x08000100 is 0x00, not the expected 0x01"
        );
        assert!(!dir.path().join("mod.gba").exists());
    }

    #[test]
    fn reject_build_with_missing_rom() {
        let dir = game_dir("[output]\nrom = \"mod.gba\"\n");
        fs::remove_file(dir.path().join("game.gba")).unwrap();
        assert!(matches!(
            build(dir.path()).unwrap_err(),
            BuildError::Game(_)
        ));
        assert!(matches!(
            build(Path::new("/nonexistent")).unwrap_err(),
            BuildError::Game(_)
        ));
    }

    #[test]
    fn reject_unwritable_output() {
        let dir = game_dir("[output]\nips = \"game.gba/mod.ips\"\n");
        let err = build(dir.path()).unwrap_err();
        let path = dir.path().join("game.gba/mod.ips");
        assert!(
            err.to_string()
                .starts_with(&format!("failed to write `{}`: ", path.display())),
            "{err}"
        );
    }

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
