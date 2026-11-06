//! A long-running emulator session that commands reach over a Unix socket.
//!
//! The session runs in a background process for a game folder and listens on
//! [`SOCKET_NAME`] there. Each command connects, sends one [`Request`] as a
//! line of JSON, and reads back one line holding the result.

use crate::keys::Keys;
use crate::memory::{self, Width};
use crate::screen::Screen;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The file name of the session's socket in the game folder.
pub const SOCKET_NAME: &str = ".gbamod-session.sock";

/// How many frames a tap runs after releasing its keys, so the game sees the
/// release before the next command.
pub const TAP_RELEASE_FRAMES: u32 = 4;

/// Returns the path of the session socket for the game folder at `dir`.
#[must_use]
pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_NAME)
}

/// A command for the session to run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "command", rename_all = "lowercase")]
pub enum Request {
    /// Runs frames with the held keys pressed.
    Run {
        /// The number of frames to run.
        frames: u32,
    },
    /// Sets the keys that stay pressed while frames run.
    Hold {
        /// The keys to hold.
        keys: Keys,
    },
    /// Presses keys for some frames, then releases them back to the held keys.
    Tap {
        /// The keys to press.
        keys: Keys,
        /// The number of frames to press them for.
        frames: u32,
    },
    /// Writes a savestate to a file.
    Save {
        /// The savestate file.
        path: PathBuf,
    },
    /// Restores a savestate from a file.
    Load {
        /// The savestate file.
        path: PathBuf,
    },
    /// Saves the screen to a PNG file.
    Shot {
        /// The PNG file.
        path: PathBuf,
    },
    /// Reads bytes from memory.
    Peek {
        /// The address of the first byte.
        address: u32,
        /// The number of bytes to read.
        count: u32,
    },
    /// Writes a value to memory.
    Poke(Poke),
    /// Writes all working RAM to a file, laid out as [`memory::RAM_REGIONS`].
    Dump {
        /// The dump file.
        path: PathBuf,
    },
    /// Ends the session.
    Stop,
}

/// A value to write to memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Poke {
    /// The address to write to.
    pub address: u32,
    /// The width of the value.
    pub width: Width,
    /// The value, which must fit in `width`.
    pub value: u32,
}

/// The emulator operations a session needs.
pub trait Machine {
    /// Sets the keys pressed while frames run.
    fn set_keys(&mut self, keys: Keys);
    /// Runs `frames` frames.
    fn run_frames(&mut self, frames: u32);
    /// Returns the number of frames run since power on.
    fn frame(&self) -> u32;
    /// Captures the whole emulator state.
    fn save_state(&self) -> Vec<u8>;
    /// Restores a state captured by [`Machine::save_state`], returning
    /// whether it was accepted.
    fn load_state(&mut self, state: &[u8]) -> bool;
    /// Captures the last frame drawn to the screen.
    fn screen(&self) -> Screen;
    /// Reads the byte at `address`, without the side effects a game's own
    /// read could have.
    fn read_byte(&self, address: u32) -> u8;
    /// Writes `value` at `address` with the given width, without the side
    /// effects a game's own write could have.
    fn write(&mut self, address: u32, width: Width, value: u32);
}

/// The state of a session: its machine and the keys held between commands.
pub struct Session<M> {
    machine: M,
    held: Keys,
}

impl<M: Machine> Session<M> {
    /// Creates a session around `machine`, with no keys held.
    pub const fn new(machine: M) -> Self {
        Self {
            machine,
            held: Keys::NONE,
        }
    }

    /// Runs a request, returning a message describing the result.
    ///
    /// # Errors
    ///
    /// Returns a message describing the failure if a file can't be read or
    /// written, or a savestate is rejected.
    pub fn handle(&mut self, request: &Request) -> Result<String, String> {
        match request {
            Request::Run { frames } => {
                self.machine.set_keys(self.held);
                self.machine.run_frames(*frames);
                Ok(format!(
                    "Ran {frames} frames, now at frame {}",
                    self.machine.frame()
                ))
            }
            Request::Hold { keys } => {
                self.held = *keys;
                Ok(format!("Holding {keys}"))
            }
            Request::Tap { keys, frames } => {
                self.machine.set_keys(*keys);
                self.machine.run_frames(*frames);
                self.machine.set_keys(self.held);
                self.machine.run_frames(TAP_RELEASE_FRAMES);
                Ok(format!(
                    "Tapped {keys} for {frames} frames, now at frame {}",
                    self.machine.frame()
                ))
            }
            Request::Save { path } => {
                write(path, &self.machine.save_state())?;
                Ok(format!("Saved the state to {}", path.display()))
            }
            Request::Load { path } => {
                let state = std::fs::read(path)
                    .map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
                if !self.machine.load_state(&state) {
                    return Err(format!(
                        "`{}` isn't a savestate this emulator accepts",
                        path.display()
                    ));
                }
                Ok(format!(
                    "Loaded the state from {}, now at frame {}; the screen updates on the next frame",
                    path.display(),
                    self.machine.frame()
                ))
            }
            Request::Shot { path } => {
                write(path, &self.machine.screen().encode_png())?;
                Ok(format!("Saved the screen to {}", path.display()))
            }
            Request::Peek { address, count } => {
                let bytes: Vec<u8> = (0..*count)
                    .map(|offset| self.machine.read_byte(address.wrapping_add(offset)))
                    .collect();
                Ok(memory::hex_dump(*address, &bytes))
            }
            Request::Poke(poke) => self.poke(*poke),
            Request::Dump { path } => {
                let bytes: Vec<u8> = memory::RAM_REGIONS
                    .iter()
                    .flat_map(|region| region.start..region.start + region.size)
                    .map(|address| self.machine.read_byte(address))
                    .collect();
                write(path, &bytes)?;
                Ok(format!(
                    "Saved {} bytes of RAM to {}",
                    bytes.len(),
                    path.display()
                ))
            }
            Request::Stop => Ok("Stopped the session".to_string()),
        }
    }

    /// Writes a poke's value to memory if it fits in its width.
    fn poke(&mut self, poke: Poke) -> Result<String, String> {
        let value = poke
            .width
            .check(poke.value)
            .map_err(|err| err.to_string())?;
        self.machine.write(poke.address, poke.width, value);
        Ok(format!(
            "Wrote {} to 0x{:08X}",
            poke.width.format(value),
            poke.address
        ))
    }
}

/// Writes `bytes` to the file at `path`, describing any failure.
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes)
        .map_err(|err| format!("failed to write `{}`: {err}", path.display()))
}

/// Answers requests on `listener` one connection at a time, until a
/// [`Request::Stop`] has been answered.
pub fn serve<M: Machine>(listener: &UnixListener, session: &mut Session<M>) {
    for stream in listener.incoming().flatten() {
        if answer(&stream, session) == Some(Request::Stop) {
            return;
        }
    }
}

/// Answers the one request sent on `stream`, returning it if it was valid.
fn answer<M: Machine>(stream: &UnixStream, session: &mut Session<M>) -> Option<Request> {
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let (request, response) = match serde_json::from_str::<Request>(&line) {
        Ok(request) => {
            let response = session.handle(&request);
            (Some(request), response)
        }
        Err(err) => (None, Err(format!("invalid request: {err}"))),
    };
    // A client that hung up doesn't stop the session from answering others.
    let _ = writeln!(&mut &*stream, "{}", encode(&response));
    request
}

/// Encodes a value as one line of JSON.
fn encode<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// The reason a request couldn't be completed by a session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No session is listening in the game folder.
    #[error("no session is running in this folder; start one with `gbamod start`")]
    NotRunning,
    /// The connection to the session failed.
    #[error("failed to talk to the session: {0}")]
    Io(#[from] io::Error),
    /// The session's answer couldn't be decoded.
    #[error("the session sent an invalid response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    /// The session ran the request, and it failed.
    #[error("{0}")]
    Failed(String),
}

/// Sends a request to the session listening on `socket` and returns its
/// result.
///
/// # Errors
///
/// Returns a [`SessionError`] if no session is listening, the connection
/// fails, or the session reports that the request failed.
pub fn send(socket: &Path, request: &Request) -> Result<String, SessionError> {
    let stream = UnixStream::connect(socket).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => SessionError::NotRunning,
        _ => SessionError::Io(err),
    })?;
    writeln!(&mut &stream, "{}", encode(request))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    serde_json::from_str::<Result<String, String>>(&line)?.map_err(SessionError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::{HEIGHT, WIDTH};
    use std::fs;
    use std::thread;

    /// A machine that records what it was asked to do.
    #[derive(Default)]
    struct FakeMachine {
        frame: u32,
        keys: Vec<(Keys, u32)>,
        current_keys: Keys,
        state: Vec<u8>,
        memory: std::collections::HashMap<u32, u8>,
    }

    impl Machine for FakeMachine {
        fn set_keys(&mut self, keys: Keys) {
            self.current_keys = keys;
        }

        fn run_frames(&mut self, frames: u32) {
            self.keys.push((self.current_keys, frames));
            self.frame += frames;
        }

        fn frame(&self) -> u32 {
            self.frame
        }

        fn save_state(&self) -> Vec<u8> {
            self.frame.to_le_bytes().to_vec()
        }

        fn load_state(&mut self, state: &[u8]) -> bool {
            let Ok(bytes) = state.try_into() else {
                return false;
            };
            self.frame = u32::from_le_bytes(bytes);
            self.state = state.to_vec();
            true
        }

        fn screen(&self) -> Screen {
            Screen::from_frame_buffer(&vec![0x00FF_0000; WIDTH * HEIGHT], WIDTH)
        }

        /// Reads the written byte, or the address's low byte if none was.
        fn read_byte(&self, address: u32) -> u8 {
            self.memory
                .get(&address)
                .copied()
                .unwrap_or_else(|| address.to_le_bytes()[0])
        }

        fn write(&mut self, address: u32, width: Width, value: u32) {
            for (offset, byte) in (0..width.bytes()).zip(value.to_le_bytes()) {
                self.memory.insert(address + offset, byte);
            }
        }
    }

    fn keys(text: &str) -> Keys {
        text.parse().unwrap()
    }

    #[test]
    fn run_with_held_keys() {
        let mut session = Session::new(FakeMachine::default());
        assert_eq!(
            session.handle(&Request::Run { frames: 10 }),
            Ok("Ran 10 frames, now at frame 10".into())
        );
        assert_eq!(
            session.handle(&Request::Hold { keys: keys("A+UP") }),
            Ok("Holding A+UP".into())
        );
        session.handle(&Request::Run { frames: 5 }).unwrap();
        assert_eq!(
            session.machine.keys,
            vec![(Keys::NONE, 10), (keys("A+UP"), 5)]
        );
    }

    #[test]
    fn tap_keys() {
        let mut session = Session::new(FakeMachine::default());
        session.handle(&Request::Hold { keys: keys("B") }).unwrap();
        assert_eq!(
            session.handle(&Request::Tap {
                keys: keys("START"),
                frames: 3
            }),
            Ok("Tapped START for 3 frames, now at frame 7".into())
        );
        assert_eq!(
            session.machine.keys,
            vec![(keys("START"), 3), (keys("B"), TAP_RELEASE_FRAMES)]
        );
    }

    #[test]
    fn save_and_load_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.st");
        let mut session = Session::new(FakeMachine::default());
        session.handle(&Request::Run { frames: 42 }).unwrap();

        assert_eq!(
            session.handle(&Request::Save { path: path.clone() }),
            Ok(format!("Saved the state to {}", path.display()))
        );
        session.handle(&Request::Run { frames: 8 }).unwrap();
        assert_eq!(
            session.handle(&Request::Load { path: path.clone() }),
            Ok(format!(
                "Loaded the state from {}, now at frame 42; the screen updates on the next frame",
                path.display()
            ))
        );
        assert_eq!(session.machine.state, 42_u32.to_le_bytes());
    }

    #[test]
    fn reject_unwritable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/a.st");
        let mut session = Session::new(FakeMachine::default());
        let err = session
            .handle(&Request::Save { path: path.clone() })
            .unwrap_err();
        assert!(err.starts_with(&format!("failed to write `{}`: ", path.display())));
    }

    #[test]
    fn reject_unreadable_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.st");
        let mut session = Session::new(FakeMachine::default());
        let err = session
            .handle(&Request::Load { path: path.clone() })
            .unwrap_err();
        assert!(err.starts_with(&format!("failed to read `{}`: ", path.display())));
    }

    #[test]
    fn reject_invalid_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.st");
        fs::write(&path, [1, 2, 3]).unwrap();
        let mut session = Session::new(FakeMachine::default());
        assert_eq!(
            session.handle(&Request::Load { path: path.clone() }),
            Err(format!(
                "`{}` isn't a savestate this emulator accepts",
                path.display()
            ))
        );
    }

    #[test]
    fn save_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let mut session = Session::new(FakeMachine::default());
        assert_eq!(
            session.handle(&Request::Shot { path: path.clone() }),
            Ok(format!("Saved the screen to {}", path.display()))
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            session.machine.screen().encode_png()
        );
    }

    #[test]
    fn peek_memory() {
        let mut session = Session::new(FakeMachine::default());
        assert_eq!(
            session.handle(&Request::Peek {
                address: 0x0200_00FE,
                count: 4
            }),
            Ok("020000FE: FE FF 00 01".into())
        );
    }

    #[test]
    fn poke_memory() {
        let mut session = Session::new(FakeMachine::default());
        for (width, value, message) in [
            (Width::U8, 0xAA, "Wrote 0xAA to 0x02000000"),
            (Width::U16, 0xBBCC, "Wrote 0xBBCC to 0x02000000"),
            (Width::U32, 0x1122_3344, "Wrote 0x11223344 to 0x02000000"),
        ] {
            assert_eq!(
                session.handle(&Request::Poke(Poke {
                    address: 0x0200_0000,
                    width,
                    value
                })),
                Ok(message.into())
            );
        }
        assert_eq!(
            session.handle(&Request::Peek {
                address: 0x0200_0000,
                count: 4
            }),
            Ok("02000000: 44 33 22 11".into())
        );
    }

    #[test]
    fn reject_too_wide_poke() {
        let mut session = Session::new(FakeMachine::default());
        assert_eq!(
            session.handle(&Request::Poke(Poke {
                address: 0x0200_0000,
                width: Width::U8,
                value: 0x100
            })),
            Err("value 0x100 doesn't fit in 8 bits".into())
        );
        assert!(session.machine.memory.is_empty());
    }

    #[test]
    fn dump_ram() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ram.bin");
        let mut session = Session::new(FakeMachine::default());
        session.machine.write(0x0300_0010, Width::U8, 0x5A);

        assert_eq!(
            session.handle(&Request::Dump { path: path.clone() }),
            Ok(format!("Saved 294912 bytes of RAM to {}", path.display()))
        );
        let dump = fs::read(&path).unwrap();
        assert_eq!(dump.len(), 0x4_8000);
        assert_eq!(dump[0x01], 0x01);
        assert_eq!(dump[0x4_0010], 0x5A);
    }

    #[test]
    fn reject_unwritable_dump() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(FakeMachine::default());
        assert!(
            session
                .handle(&Request::Dump {
                    path: dir.path().join("missing/ram.bin")
                })
                .is_err()
        );
    }

    #[test]
    fn stop_session() {
        let mut session = Session::new(FakeMachine::default());
        assert_eq!(
            session.handle(&Request::Stop),
            Ok("Stopped the session".into())
        );
    }

    #[test]
    fn serve_requests_until_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let socket = socket_path(dir.path());
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let mut session = Session::new(FakeMachine::default());
            serve(&listener, &mut session);
            session.machine.frame
        });

        assert_eq!(
            send(&socket, &Request::Run { frames: 3 }).unwrap(),
            "Ran 3 frames, now at frame 3"
        );
        let err = send(
            &socket,
            &Request::Load {
                path: dir.path().join("missing.st"),
            },
        )
        .unwrap_err();
        assert!(matches!(err, SessionError::Failed(_)));
        assert!(err.to_string().starts_with("failed to read `"));

        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "not json").unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        assert!(line.starts_with(r#"{"Err":"invalid request: "#));

        assert_eq!(
            send(&socket, &Request::Stop).unwrap(),
            "Stopped the session"
        );
        assert_eq!(server.join().unwrap(), 3);
    }

    #[test]
    fn report_missing_session() {
        let dir = tempfile::tempdir().unwrap();
        let err = send(&socket_path(dir.path()), &Request::Stop).unwrap_err();
        assert!(matches!(err, SessionError::NotRunning));
        assert_eq!(
            err.to_string(),
            "no session is running in this folder; start one with `gbamod start`"
        );
    }

    #[test]
    fn report_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = socket_path(dir.path());
        drop(UnixListener::bind(&socket).unwrap());
        let err = send(&socket, &Request::Stop).unwrap_err();
        assert!(matches!(err, SessionError::NotRunning));
    }

    #[test]
    fn report_connection_error() {
        let socket = PathBuf::from("x".repeat(200));
        let err = send(&socket, &Request::Stop).unwrap_err();
        assert!(matches!(err, SessionError::Io(_)));
        assert!(
            err.to_string()
                .starts_with("failed to talk to the session: ")
        );
    }

    #[test]
    fn report_invalid_response() {
        let dir = tempfile::tempdir().unwrap();
        let socket = socket_path(dir.path());
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            BufReader::new(&stream)
                .read_line(&mut String::new())
                .unwrap();
            writeln!(&mut &stream, "nonsense").unwrap();
        });

        let err = send(&socket, &Request::Stop).unwrap_err();
        assert!(matches!(err, SessionError::InvalidResponse(_)));
        assert!(
            err.to_string()
                .starts_with("the session sent an invalid response: ")
        );
        server.join().unwrap();
    }

    #[test]
    fn encode_poke_request() {
        assert_eq!(
            encode(&Request::Poke(Poke {
                address: 16,
                width: Width::U16,
                value: 288
            })),
            r#"{"command":"poke","address":16,"width":"u16","value":288}"#
        );
    }

    #[test]
    fn encode_requests() {
        assert_eq!(
            encode(&Request::Tap {
                keys: keys("A"),
                frames: 4
            }),
            r#"{"command":"tap","keys":1,"frames":4}"#
        );
    }
}
