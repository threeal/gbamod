//! An emulator-driven toolkit for reverse engineering and patching GBA games.

pub mod asm;
pub mod build;
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
pub mod refs;
pub mod rom;
pub mod screen;
pub mod search;
pub mod session;

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
    /// The manifest's patches and hooks couldn't be applied to the ROM.
    #[error(transparent)]
    Build(#[from] build::BuildError),
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

/// The flag of [`SERVE_COMMAND`] that runs the ROM with the manifest's
/// patches and hooks applied.
pub const PATCHED_FLAG: &str = "patched";

/// Launches a session for the game folder at `dir` in a background process,
/// and waits until it's listening, for the `start` command.
///
/// If `patched` is set, the session runs the ROM with the manifest's patches
/// and hooks applied.
///
/// # Errors
///
/// Returns a [`StartError`] if a session is already running, the game folder
/// or its ROM can't be read, the patches and hooks can't be applied, or the
/// background process fails to start listening.
#[cfg(not(tarpaulin_include))]
pub fn start(dir: &Path, patched: bool) -> Result<String, StartError> {
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let socket = session::socket_path(dir);
    if UnixStream::connect(&socket).is_ok() {
        return Err(StartError::AlreadyRunning);
    }
    // Check the ROM here, where errors can still be reported to the user.
    let game = Game::open(dir)?;
    if patched {
        build::patched_rom(&game)?;
    } else {
        game.read_rom()?;
    }
    // A socket left behind by a session that was killed is no longer in use.
    let _ = std::fs::remove_file(&socket);

    let mut command = Command::new(std::env::current_exe()?);
    command.arg(SERVE_COMMAND);
    if patched {
        command.arg(format!("--{PATCHED_FLAG}"));
    }
    let mut child = command
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    for _ in 0..200 {
        if UnixStream::connect(&socket).is_ok() {
            let rom = if patched {
                "the patched ROM"
            } else {
                "the ROM"
            };
            return Ok(format!(
                "Started a session on {rom} (process {})",
                child.id()
            ));
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
    /// The manifest's patches and hooks couldn't be applied to the ROM.
    #[error(transparent)]
    Build(#[from] build::BuildError),
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
/// If `patched` is set, the session runs the ROM with the manifest's patches
/// and hooks applied.
///
/// # Errors
///
/// Returns a [`ServeError`] if the game folder or its ROM can't be read, the
/// patches and hooks can't be applied, the emulator can't load the ROM, or
/// the session socket can't be created.
#[cfg(not(tarpaulin_include))]
pub fn serve(dir: &Path, patched: bool) -> Result<(), ServeError> {
    let game = Game::open(dir)?;
    let rom = if patched {
        build::patched_rom(&game)?.0
    } else {
        game.read_rom()?
    };
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
