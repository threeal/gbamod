//! A headless GBA emulator, backed by mGBA through the C shim in
//! `src/emulator.c`.
//!
//! This module needs a real ROM to do anything, so it's excluded from the
//! coverage check.

use crate::debug::{self, Access, Hit, Hits, Registers, WatchKind};
use crate::disasm::{Instruction, Mode};
use crate::keys::Keys;
use crate::memory::Width;
use crate::rom::Rom;
use crate::screen::{HEIGHT, Screen};
use crate::session::Machine;
use crate::writes::{self, Written};
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

/// The width in pixels of each row of mGBA's frame buffer.
const VIDEO_STRIDE: usize = 256;

/// The emulator state owned by the C shim.
#[repr(C)]
struct RawEmulator {
    _private: [u8; 0],
}

/// A hit recorded by the C shim, laid out as its `struct gbamod_hit`.
#[repr(C)]
struct RawHit {
    point: i64,
    frame: u32,
    address: u32,
    access: u32,
    old_value: u32,
    new_value: u32,
    registers: [u32; 16],
    cpsr: u32,
    stack: [u32; debug::MAX_STACK_WORDS as usize],
}

/// mGBA's `mWatchpointType` values.
const WATCHPOINT_WRITE: u32 = 1;
const WATCHPOINT_READ: u32 = 2;
const WATCHPOINT_RW: u32 = 3;
const WATCHPOINT_WRITE_CHANGE: u32 = 5;

unsafe extern "C" {
    fn gbamod_emulator_new(rom: *const c_void, size: usize) -> *mut RawEmulator;
    fn gbamod_emulator_free(emulator: *mut RawEmulator);
    fn gbamod_emulator_run_frames(emulator: *mut RawEmulator, frames: u32);
    fn gbamod_emulator_video(emulator: *const RawEmulator) -> *const u32;
    fn gbamod_emulator_set_keys(emulator: *mut RawEmulator, keys: u32);
    fn gbamod_emulator_frame(emulator: *const RawEmulator) -> u32;
    fn gbamod_emulator_state_size(emulator: *const RawEmulator) -> usize;
    fn gbamod_emulator_save_state(emulator: *const RawEmulator, state: *mut c_void) -> bool;
    fn gbamod_emulator_read8(emulator: *const RawEmulator, address: u32) -> u8;
    fn gbamod_emulator_write(emulator: *mut RawEmulator, address: u32, width: u32, value: u32);
    fn gbamod_emulator_watch(emulator: *mut RawEmulator, address: u32, kind: u32) -> i64;
    fn gbamod_emulator_break(emulator: *mut RawEmulator, address: u32) -> i64;
    fn gbamod_emulator_clear_point(emulator: *mut RawEmulator, id: i64);
    fn gbamod_emulator_hits(
        emulator: *const RawEmulator,
        kept: *mut usize,
        total: *mut u32,
    ) -> *const RawHit;
    fn gbamod_emulator_clear_hits(emulator: *mut RawEmulator);
    fn gbamod_emulator_registers(emulator: *const RawEmulator, registers: *mut u32);
    fn gbamod_emulator_track_writes(emulator: *mut RawEmulator);
    fn gbamod_emulator_written(emulator: *const RawEmulator, frames: *mut u32) -> *const u8;
    fn gbamod_emulator_disassemble(
        emulator: *mut RawEmulator,
        address: u32,
        thumb: bool,
        opcode: *mut u32,
        text: *mut c_char,
        size: usize,
    ) -> u32;
    fn gbamod_emulator_load_state(
        emulator: *mut RawEmulator,
        state: *const c_void,
        size: usize,
    ) -> bool;
}

/// The reason mGBA couldn't start running a ROM.
#[derive(Debug, thiserror::Error)]
#[error("mGBA failed to load the ROM")]
pub struct LoadError;

/// A running GBA emulator with no window, audio, or input.
pub struct Emulator {
    raw: NonNull<RawEmulator>,
}

impl Emulator {
    /// Powers on a GBA with a copy of `rom` inserted.
    ///
    /// # Errors
    ///
    /// Returns a [`LoadError`] if mGBA can't create its core or load the ROM.
    pub fn new(rom: &Rom) -> Result<Self, LoadError> {
        // SAFETY: the shim copies the ROM bytes before returning.
        let raw = unsafe { gbamod_emulator_new(rom.bytes.as_ptr().cast(), rom.bytes.len()) };
        NonNull::new(raw).map(|raw| Self { raw }).ok_or(LoadError)
    }

    /// Runs the emulator for `frames` frames.
    pub fn run_frames(&mut self, frames: u32) {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_run_frames(self.raw.as_ptr(), frames) };
    }

    /// Captures the last frame drawn to the screen.
    #[must_use]
    pub fn screen(&self) -> Screen {
        // SAFETY: the shim's frame buffer is `VIDEO_STRIDE * HEIGHT` pixels and
        // lives as long as the emulator, which `self` borrows here.
        let buffer = unsafe {
            std::slice::from_raw_parts(
                gbamod_emulator_video(self.raw.as_ptr()),
                VIDEO_STRIDE * HEIGHT,
            )
        };
        Screen::from_frame_buffer(buffer, VIDEO_STRIDE)
    }
}

impl Machine for Emulator {
    fn set_keys(&mut self, keys: Keys) {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_set_keys(self.raw.as_ptr(), keys.bits().into()) };
    }

    fn run_frames(&mut self, frames: u32) {
        Self::run_frames(self, frames);
    }

    fn frame(&self) -> u32 {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_frame(self.raw.as_ptr()) }
    }

    fn save_state(&self) -> Vec<u8> {
        // SAFETY: `raw` is a live emulator owned by `self`, and the buffer is
        // exactly the size the shim reports for a savestate.
        unsafe {
            let mut state = vec![0; gbamod_emulator_state_size(self.raw.as_ptr())];
            gbamod_emulator_save_state(self.raw.as_ptr(), state.as_mut_ptr().cast());
            state
        }
    }

    fn load_state(&mut self, state: &[u8]) -> bool {
        // SAFETY: `raw` is a live emulator owned by `self`, and the shim
        // checks the size before reading `state`.
        unsafe { gbamod_emulator_load_state(self.raw.as_ptr(), state.as_ptr().cast(), state.len()) }
    }

    fn screen(&self) -> Screen {
        Self::screen(self)
    }

    fn read_byte(&self, address: u32) -> u8 {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_read8(self.raw.as_ptr(), address) }
    }

    fn write(&mut self, address: u32, width: Width, value: u32) {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_write(self.raw.as_ptr(), address, width.bytes(), value) };
    }

    fn disassemble(&mut self, address: u32, mode: Mode) -> Instruction {
        let mut opcode = 0;
        let mut text = [0 as c_char; 128];
        // SAFETY: `raw` is a live emulator owned by `self`, and the shim
        // writes a null-terminated string of at most `text.len()` bytes.
        let size = unsafe {
            gbamod_emulator_disassemble(
                self.raw.as_ptr(),
                address,
                mode == Mode::Thumb,
                &raw mut opcode,
                text.as_mut_ptr(),
                text.len(),
            )
        };
        // SAFETY: the shim null-terminates `text`.
        let text = unsafe { CStr::from_ptr(text.as_ptr()) };
        Instruction {
            size,
            opcode,
            text: text.to_string_lossy().into_owned(),
        }
    }

    fn add_watchpoint(&mut self, address: u32, kind: WatchKind) -> i64 {
        let kind = match kind {
            WatchKind::Write => WATCHPOINT_WRITE,
            WatchKind::Read => WATCHPOINT_READ,
            WatchKind::Access => WATCHPOINT_RW,
            WatchKind::Change => WATCHPOINT_WRITE_CHANGE,
        };
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_watch(self.raw.as_ptr(), address, kind) }
    }

    fn add_breakpoint(&mut self, address: u32) -> i64 {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_break(self.raw.as_ptr(), address) }
    }

    fn clear_point(&mut self, id: i64) {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_clear_point(self.raw.as_ptr(), id) };
    }

    fn take_hits(&mut self) -> Hits {
        let mut kept = 0;
        let mut total = 0;
        // SAFETY: `raw` is a live emulator owned by `self`, and the shim
        // returns an array of `kept` hits that stays valid until the hits are
        // cleared, after they're copied here.
        unsafe {
            let raw = gbamod_emulator_hits(self.raw.as_ptr(), &raw mut kept, &raw mut total);
            let kept = std::slice::from_raw_parts(raw, kept)
                .iter()
                .map(Hit::from)
                .collect();
            gbamod_emulator_clear_hits(self.raw.as_ptr());
            Hits { kept, total }
        }
    }

    fn registers(&self) -> Registers {
        let mut values = [0; 17];
        // SAFETY: `raw` is a live emulator owned by `self`, and the shim
        // writes exactly 17 values.
        unsafe { gbamod_emulator_registers(self.raw.as_ptr(), values.as_mut_ptr()) };
        let mut general = [0; 16];
        general.copy_from_slice(&values[..16]);
        Registers {
            general,
            cpsr: values[16],
        }
    }

    fn track_writes(&mut self) {
        // SAFETY: `raw` is a live emulator owned by `self`.
        unsafe { gbamod_emulator_track_writes(self.raw.as_ptr()) };
    }

    fn written(&self) -> Option<Written> {
        let mut frames = 0;
        // SAFETY: `raw` is a live emulator owned by `self`, and a bitmap the
        // shim returns holds one bit for each byte of a RAM dump.
        unsafe {
            let bits = gbamod_emulator_written(self.raw.as_ptr(), &raw mut frames);
            (!bits.is_null()).then(|| {
                let bits = std::slice::from_raw_parts(bits, writes::BITMAP_SIZE).to_vec();
                Written::new(frames, bits)
            })
        }
    }
}

impl From<&RawHit> for Hit {
    fn from(raw: &RawHit) -> Self {
        let access = match raw.access {
            0 => Access::Execute,
            2 => Access::Read {
                value: raw.old_value,
            },
            _ => Access::Write {
                old: raw.old_value,
                new: raw.new_value,
            },
        };
        Self {
            point: raw.point,
            frame: raw.frame,
            address: raw.address,
            access,
            registers: Registers {
                general: raw.registers,
                cpsr: raw.cpsr,
            },
            stack: raw.stack.to_vec(),
        }
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        // SAFETY: `raw` is a live emulator owned by `self`, freed only here.
        unsafe { gbamod_emulator_free(self.raw.as_ptr()) };
    }
}
