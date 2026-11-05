# gbamod

An emulator-driven toolkit for reverse engineering and patching [GBA](https://en.wikipedia.org/wiki/Game_Boy_Advance) games. It drives an emulator headlessly to dump and diff memory, set watchpoints and breakpoints, and assemble hooks into [IPS](https://zerosoft.zophar.net/ips.php) patches.

This project is at an early stage: so far it can read a game folder, check its ROM, and boot it in the emulator to take a screenshot.

## Usage

gbamod runs from inside a game folder, a directory holding a `gbamod.toml` manifest that names the game's ROM:

```toml
version = 1
rom = "roms/game.gba"
```

Print what the ROM's header says, after checking that it's a valid GBA ROM:

```sh
gbamod info
```

Boot the ROM in [mGBA](https://mgba.io/) with no window, run it for a number of frames, and save the final screen as a PNG:

```sh
gbamod boot 600 boot.png
```

## Setup

gbamod builds mGBA from source, pinned as a git submodule in `third_party/mgba`. Clone with `--recurse-submodules`, or fetch it in an existing clone:

```sh
git submodule update --init
```

Building it needs [CMake](https://cmake.org/) and a C compiler.

Install [rustup](https://rustup.rs/), then install the toolchain pinned in `rust-toolchain.toml`:

```sh
rustup install
```

Install [cargo-tarpaulin](https://github.com/xd009642/tarpaulin) to run the test suite with coverage:

```sh
cargo install cargo-tarpaulin
```

Install [Lefthook](https://lefthook.dev/) and [dprint](https://dprint.dev/), then register the pre-commit hook:

```sh
lefthook install
```

## Development

Write code in `src/`. Unit tests live in a `tests` module alongside the code they cover. Run the test suite with:

```sh
cargo tarpaulin
```

The project enforces 100% code coverage on every run.

Game data such as ROMs, savestates, and memory dumps never belongs in this repository. While developing, keep each game in its own folder under `.games/`, which `.gitignore` already excludes, and run the toolkit from inside that folder with `cargo run --`.

Each `git commit` runs the pre-commit hook registered during setup, which checks your changes and fixes what it can in place. If it fails, fix any reported issues, re-stage the changed files, and commit again.

After committing, push to `main` or open a pull request from another branch — CI will run the same checks across all files, plus additional checks of its own.

## Releasing

Update the version in `Cargo.toml`, push a version tag, and create a GitHub Release. To publish to [crates.io](https://crates.io/), run:

```sh
cargo publish
```
