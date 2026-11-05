# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## About This Repository

gbamod is an emulator-driven Rust library and CLI for reverse engineering and patching GBA games. It's at an early stage: so far it reads a game folder's `gbamod.toml` manifest and validates the ROM it names.

## Game Data

Never commit game data — ROMs, savestates, memory dumps, screenshots, notes, hooks, or patches — and never name a specific game in this repository.

During development, each game lives in its own folder under `.games/<game>/`, ignored by git, and the toolkit runs from inside it with `cargo run --`. The toolkit must never assume `.games/` exists; it works from the current directory or from paths it's given.
Why: `.games/` is only a local convenience, and the released binary will run from game folders anywhere.

## Manifest Versions

`gbamod.toml` carries a `version` field. `Manifest::parse` dispatches on it with one layout struct per version, so a layout change adds a new version and case rather than editing an existing one, and older manifests keep parsing.

## Gotchas

- `lefthook run pre-commit` skips every job when nothing is staged, even ones that ignore the staged file list — pass `--all-files` to run it outside an actual commit.
- The pre-commit hook fixes files in place, and the run fails if any file changes. Report that failure and leave the fixes for the user to review and re-stage before committing again.
- The test command is `cargo tarpaulin`, not `cargo test` — only it applies the 100% coverage threshold from `tarpaulin.toml`.
- `.gitignore` ignores every dotfile and dotdir (`.*`) — a new one needs an explicit `!` exception there or it silently won't be tracked.
