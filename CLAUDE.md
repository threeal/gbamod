# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## About This Repository

gbamod is an emulator-driven Rust library and CLI for reverse engineering and patching GBA games. It's at an early stage: so far it reads a game folder's `gbamod.toml` manifest, validates the ROM it names, and runs it in mGBA, either once with `boot` or in a background session that later commands drive, including reading, writing, dumping, and disassembling its memory and watching it with mGBA's debugger. It also searches memory dumps for values and reads pixels from screenshots.

## Game Data

Never commit game data — ROMs, savestates, memory dumps, screenshots, notes, hooks, or patches — and never name a specific game in this repository.

During development, each game lives in its own folder under `.games/<game>/`, ignored by git, and the toolkit runs from inside it with `cargo run --`. The toolkit must never assume `.games/` exists; it works from the current directory or from paths it's given.
Why: `.games/` is only a local convenience, and the released binary will run from game folders anywhere.

## Emulator

mGBA 0.10.3 is a git submodule in `third_party/mgba`, built by `build.rs` through CMake. Rust never calls mGBA directly: `src/emulator.c` is a C shim with a flat API, and `src/emulator.rs` binds to that shim. A new emulator capability means adding a shim function, not mirroring mGBA's structs in Rust. A struct the shim hands to Rust, like `struct gbamod_hit`, is mirrored field for field by a `#[repr(C)]` struct in `src/emulator.rs`, so a change to one needs the same change to the other.

`src/emulator.rs`, and anything in `src/lib.rs` that needs a running emulator, is gated behind `#[cfg(not(tarpaulin_include))]`, since it can't be unit-tested without a real ROM. Keep logic that doesn't need the emulator, like the PNG encoding in `src/screen.rs`, outside that gate so it stays covered.

## Sessions

`gbamod start` relaunches the binary with the hidden `serve` subcommand, which runs the emulator and listens on `.gbamod-session.sock` in the game folder. Every other session command sends it one JSON request and reads one JSON response. Session behavior lives in `src/session.rs` behind the `Machine` trait, so it's tested against a fake machine; only `start`, `serve`, and the emulator's `Machine` implementation are excluded from coverage. A new session command means a new `Request` variant, a `Machine` method if the emulator must do something new, and a CLI subcommand.

## Manifest Versions

`gbamod.toml` carries a `version` field. `Manifest::parse` dispatches on it with one layout struct per version, so a layout change adds a new version and case rather than editing an existing one, and older manifests keep parsing.

## Gotchas

- `lefthook run pre-commit` skips every job when nothing is staged, even ones that ignore the staged file list — pass `--all-files` to run it outside an actual commit.
- The pre-commit hook fixes files in place, and the run fails if any file changes. Report that failure and leave the fixes for the user to review and re-stage before committing again.
- The test command is `cargo tarpaulin`, not `cargo test` — only it applies the 100% coverage threshold from `tarpaulin.toml`.
- `.gitignore` ignores every dotfile and dotdir (`.*`) — a new one needs an explicit `!` exception there or it silently won't be tracked.
