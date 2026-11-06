//! A headless GBA emulator, backed by mGBA through the C shim in
//! `src/emulator.c`.
//!
//! This module needs a real ROM to do anything, so it's excluded from the
//! coverage check.

use crate::keys::Keys;
use crate::memory::Width;
use crate::rom::Rom;
use crate::screen::{HEIGHT, Screen};
use crate::session::Machine;
use std::ffi::c_void;
use std::ptr::NonNull;

/// The width in pixels of each row of mGBA's frame buffer.
const VIDEO_STRIDE: usize = 256;

/// The emulator state owned by the C shim.
#[repr(C)]
struct RawEmulator {
    _private: [u8; 0],
}

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
}

impl Drop for Emulator {
    fn drop(&mut self) {
        // SAFETY: `raw` is a live emulator owned by `self`, freed only here.
        unsafe { gbamod_emulator_free(self.raw.as_ptr()) };
    }
}
