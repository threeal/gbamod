//! A headless GBA emulator, backed by mGBA through the C shim in
//! `src/emulator.c`.
//!
//! This module needs a real ROM to do anything, so it's excluded from the
//! coverage check.

use crate::rom::Rom;
use crate::screen::{HEIGHT, Screen};
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

impl Drop for Emulator {
    fn drop(&mut self) {
        // SAFETY: `raw` is a live emulator owned by `self`, freed only here.
        unsafe { gbamod_emulator_free(self.raw.as_ptr()) };
    }
}
