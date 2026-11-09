---
name: modding-games
description: The method for reverse engineering and patching a GBA game with gbamod, from finding where the game keeps a value to building and verifying the mod. TRIGGER - working in a game folder (one holding a `gbamod.toml`), finding a game's values or code, writing a hook or `[[patch]]`, or checking a built mod. SKIP - changing gbamod's own source, unless it's being tried on a game.
---

Run every command from the game folder. The README documents each command's syntax; this is the order they're chained in, and the traps between them.

## Notes

Keep a `NOTES.md` in the game folder, and record each finding as it's made: the address, what it holds, how it was found, and what's still unverified. Name the savestates it relies on and what moment each captures.
Why: a later session, or a later feature, starts from these notes instead of redoing the search, and a finding with no record of how it was made can't be rechecked.

## Finding a Value

1. Save a savestate just before the value changes, so every experiment starts from the same frame.
2. Read the value off the screen when it's shown as a bar: `pixels shot.png X Y W 1 --count TOKEN` gives the bar's length in pixels, which tracks the value even when no number is shown.
3. Dump the RAM at several moments where the value is known to have changed, or not, and `search` across the dumps. `seq=` with exact values narrows fastest; `dec`, `inc`, and `same` work when only the direction is known. Add dumps until a handful of addresses remain.
4. Confirm each candidate by poking it, running on, and comparing against the same run without the poke, from the same savestate. A value that changes the right way in the dumps is often only a copy, such as the one the HUD draws from, and a poke to it gets overwritten or changes nothing in play.

Dump offsets below `0x40000` are EWRAM from `0x02000000`, and the rest are IWRAM from `0x03000000`.

## Finding the Code

1. `watch` the confirmed address and run until it fires. The hit names the instruction that wrote it, with `lr` and `r0` to `r3`.
2. `dis` around that instruction to find the start of the function and what its arguments are.
3. `break` on the function's entry with `--stack` and run again. The registers show the arguments, such as the object it acts on, and the stack words show the caller's locals, such as the other party in an interaction. `lr` names the caller, so `dis` there for the call site.
4. Follow pointers out from the object: a value's struct usually holds a vtable pointer, ids, and related values next to it, and a global pointer usually leads to the list the object came from. Record the offsets.

A function called through a register or veneer has no `bl` naming it; find its callers with `refs` on its address instead.

## Writing a Hook

Thumb `bl` reaches only ±4 MB, so code in free space at the end of the ROM is reached by taking over a pointer the game already follows: a literal that a call site loads, or a vtable slot.

1. Find free space with `peek` or `dis` past the end of the game's data: a run of `0xFF` or `0x00`. Start hooks at a round address inside it, leaving a gap after the game's last byte.
2. Find the slots with `refs` on the function's address. It lists every word holding the pointer, but only some belong in `slots`: a hook that reads its caller's stack or registers is only right for callers that put them there. Check each candidate's call site with `dis` before listing it, and record the ones left out and why.
3. Write the hook in a `.s` file. `ORIGINAL` is predefined as the pointer it replaces, so don't define it again.
   - To change the arguments, adjust the registers and tail-call with `ldr r3, =ORIGINAL` then `bx r3`, clobbering only registers the original is free to clobber.
   - To change the result, `push {r4, lr}`, call the original through a `bx` veneer label, adjust `r0`, then `pop {r4}`, `pop {r1}`, `bx r1`.
   - Every `push` moves the caller's stack words further from `sp`: a caller's `[sp+N]` is `[sp+N+8]` after `push {r4, lr}`.
4. `asm hook.s --at ADDRESS` lists the code with its literal pool, but doesn't know `ORIGINAL`, so it rejects a hook that uses it. Check such a hook with `build` instead, which names the failing line, and read the placed code with `dis` in a `start --patched` session.
5. Add a `[[hook]]` entry with `source`, `at`, `replaces`, and `slots`. Use `[[patch]]` with `expect` for any other bytes, so a patch fails loudly if it's applied to the wrong ROM.

## Verifying

1. Run `build`. It refuses to write anything if a patch misses its expected bytes, a hook isn't on free space, or two patches overlap.
2. `start --patched`, load the savestate the value was found with, and play the same keys as a run on the original ROM. Savestates hold only RAM, so they load into either ROM.
3. Compare the value between the two runs over the same frames, by `peek` or by bar length. A mod only works if the patched run differs from the original one; replaying one savestate on the patched ROM alone can't show that, since the moment it starts from may never trigger the change.
4. Set a breakpoint on the hook's address to confirm it's actually reached, and check the values it sees.
5. Record what was verified, and under which conditions, in `NOTES.md`, along with what wasn't.
