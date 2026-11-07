# gbamod

An emulator-driven toolkit for reverse engineering and patching [GBA](https://en.wikipedia.org/wiki/Game_Boy_Advance) games. It drives an emulator headlessly to dump and diff memory, set watchpoints and breakpoints, and assemble hooks into [IPS](https://zerosoft.zophar.net/ips.php) patches.

This project is at an early stage: so far it can read a game folder, check its ROM, and play it in the emulator through a background session, with savestates, screenshots, and memory access, and search memory dumps for values.

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

To play further than that, start an emulator session that keeps running in the background for the game folder. Each command acts on it and returns right away, so the game continues where the last command left it:

```sh
gbamod start
gbamod run 600         # run frames with the held keys pressed
gbamod tap START       # press keys for 4 frames (or a given count), then release them
gbamod hold A+RIGHT    # keep keys pressed while later frames run; `none` releases them
gbamod save title.st   # write a savestate
gbamod load title.st   # restore a savestate
gbamod shot title.png  # save the screen as a PNG
gbamod stop
```

While a session runs, read and write its memory. Addresses and values are decimal, or hex with a `0x` prefix:

```sh
gbamod peek 0x02000000 32         # print bytes as hex, 16 by default
gbamod poke16 0x02000010 0x0120   # write a value; also poke8 and poke32
gbamod dump ram.bin               # write all working RAM to a file
```

A dump holds the 256 KiB EWRAM from `0x02000000`, followed by the 32 KiB IWRAM from `0x03000000`, so a file offset below `0x40000` is that offset into EWRAM and anything from there on is IWRAM.

To find where the game keeps a value, dump the RAM at several moments, then search the dumps for addresses whose values changed across them the way that value did. The search needs no running session:

```sh
gbamod search u16 dec before.bin after.bin                # values that fell, never rising
gbamod search u16 seq=280,273 before.bin after.bin        # exact values per dump, * for any
gbamod search u8 inc a.bin b.bin c.bin --min 1 --max 99   # values that rose, within a range
```

The rules are `dec`, `inc`, `same`, `changed`, `eq=N`, and `seq=A,B,...`. Values are read little-endian at offsets aligned to their width, and each match prints its address and its value in every dump. A search lists the first 200 matches and counts the rest.

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
