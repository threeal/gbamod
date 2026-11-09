# gbamod

An emulator-driven toolkit for reverse engineering and patching [GBA](https://en.wikipedia.org/wiki/Game_Boy_Advance) games. It drives an emulator headlessly to dump and diff memory, set watchpoints and breakpoints, and assemble hooks into [IPS](https://zerosoft.zophar.net/ips.php) patches.

This project is at an early stage: so far it can read a game folder, check its ROM, and play it in the emulator through a background session, with savestates, screenshots, memory access, disassembly, watchpoints, and breakpoints. It can also search memory dumps for values, read pixels from screenshots, assemble Thumb code, find the words in a ROM that hold a pointer, and build a patched ROM and an IPS patch from byte patches and hooks.

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

Disassemble code from the session's memory, in the ROM or in RAM:

```sh
gbamod dis 0x0804FD09        # 16 Thumb instructions; the low bit of a function pointer is ignored
gbamod dis 0x0800F9EA 40     # a given number of instructions
gbamod dis 0x08000000 --arm  # ARM code
```

Each line shows the address, the encoding, and the instruction. A Thumb `bl` is shown as one instruction with its target, and a load from a literal pool shows the value it loads and where from, such as `ldr r2, =0x08006A79  @ 0x0800FA68`.

To find the code that changes a value, set watchpoints and breakpoints. The game keeps running when one fires: the hit is recorded, and `run` and `tap` list their hits after their usual message:

```sh
gbamod watch 0x0200316C              # writes to an address; or --on read, access, or change
gbamod break 0x0804FD08 --stack 48   # an instruction, showing 48 words of the stack at each hit
gbamod points                        # list the points
gbamod clear                         # remove them all
gbamod regs                          # print the CPU registers between frames
```

A watchpoint hit shows the old and new value, the address of the instruction that made the access, `lr`, and `r0` to `r3`. A breakpoint hit shows every register, `cpsr` with the CPU state, and the requested stack words. A command lists its first 50 hits and counts the rest.

To find where the game keeps a value, dump the RAM at several moments, then search the dumps for addresses whose values changed across them the way that value did. The search needs no running session:

```sh
gbamod search u16 dec before.bin after.bin                # values that fell, never rising
gbamod search u16 seq=280,273 before.bin after.bin        # exact values per dump, * for any
gbamod search u8 inc a.bin b.bin c.bin --min 1 --max 99   # values that rose, within a range
```

The rules are `dec`, `inc`, `same`, `changed`, `eq=N`, and `seq=A,B,...`. Values are read little-endian at offsets aligned to their width, and each match prints its address and its value in every dump. A search lists the first 200 matches and counts the rest.

To check what a change did on screen, read pixels from a screenshot. Each pixel prints as a color token of three hex digits, the top 4 bits of its red, green, and blue, such as `0ce`:

```sh
gbamod pixels shot.png 16 145 48 2            # a 48x2 box from (16, 145), one row per line
gbamod pixels shot.png 16 145                 # a single pixel
gbamod pixels shot.png 16 145 48 --count 0ce  # how many pixels in each row are 0ce
```

Counting reads the length of a bar or gauge straight off, such as `145: 46 of 48`.

To write a hook, assemble a file of Thumb code placed at the address it will be loaded at, such as free space at the end of the ROM. Assembly needs no running session:

```sh
gbamod asm hook.s --at 0x08CBF000
```

Instructions are written the way `dis` prints them, plus `push`, `pop`, and `nop`, so code can be copied from a disassembly:

```asm
.equ ORIGINAL, 0x08006A79   @ name an address
        push {r4, lr}
        ldr r4, =ORIGINAL   @ load a 32-bit value from the literal pool
        bl call_r4          @ branch to a label, or to an address such as 0x080BC778
        pop {r4}
        pop {r1}
        bx r1
call_r4:
        bx r4
```

Each value loaded with `=` is placed once in a literal pool after the code, word aligned, in order of first use. `.byte`, `.hword`, and `.word` place data, `.align 2` pads to a multiple of 4, and `@` starts a comment. The listing shows each line's address, encoding, and source, followed by the literal pool, and an error names the line it's on.

To build the mod, list its patches in the manifest, along with the files to write. Each patch writes bytes at a ROM address, and can give the bytes it expects to find there, such as the pointer it replaces or the free space it fills:

```toml
version = 1
rom = "roms/game.gba"

[output]
rom = "out/mod.gba"
ips = "out/mod.ips"

[[patch]]
at = 0x0800FA68
expect = "79 6A 00 08"
bytes = "01 F0 CB 08"
```

Then apply them:

```sh
gbamod build
```

Before writing anything, it checks that every patch lies in the ROM, matches the bytes it expects, and doesn't overlap another, and lists each patch that fails. It then writes the patched ROM and the IPS patch, either of which can be left out. The game's ROM is only read.

A hook is listed by its assembly file instead of its bytes. `build` assembles it at `at`, which must be free space, all `0xFF` or all `0x00`, and points each slot at it, after checking that the slot holds the pointer given as `replaces`. The slots are the words the game follows to the code being hooked, such as a literal a call site loads or an entry in a vtable:

```toml
[[hook]]
source = "hooks/guard.s"
at = 0x08CBF080
replaces = 0x0804FD09
slots = [0x08CBC850, 0x08CBC9F0]
```

To find the slots, list the words in the ROM that hold the pointer. A Thumb function's address and the pointers to it differ only in the low bit, which is ignored, so either form finds both. The list ends with a `slots` line ready for the manifest:

```sh
gbamod refs 0x0804FD09
```

The hook's code can call or jump to the pointer it replaces as `ORIGINAL`, such as `ldr r3, =ORIGINAL` followed by `bx r3`.

To try the mod, start a session on the ROM with the patches and hooks applied in memory, then play it with the usual session commands:

```sh
gbamod start --patched
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
