//! Building a mod: applying a manifest's byte patches and hooks to its ROM.

use crate::asm::{self, AsmError};
use crate::game::{Game, GameError};
use crate::manifest::{Hook, Manifest};
use crate::patch::{self, Expect, Patch, PatchError};
use crate::rom::Rom;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The name a hook's code uses for the pointer it replaces.
pub const ORIGINAL: &str = "ORIGINAL";

/// The reason a mod couldn't be built.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The game folder or its ROM couldn't be read.
    #[error(transparent)]
    Game(#[from] GameError),
    /// The manifest names no file to write.
    #[error("the manifest names no output; add `rom` or `ips` under `[output]`")]
    NoOutput,
    /// An output file is the game's ROM.
    #[error("the output `{}` is the game's ROM, which gbamod only reads", .0.display())]
    OverwriteRom(PathBuf),
    /// A hook has slots but no pointer for them to hold.
    #[error("hook `{}` has slots but no `replaces` pointer for them to hold", .0.display())]
    NoReplaces(PathBuf),
    /// Code that isn't a hook couldn't be read or assembled.
    #[error(transparent)]
    Asm(#[from] AsmError),
    /// Code to list has no address, given or from a hook.
    #[error("`{}` isn't a hook in the manifest, so give the address it starts at with `--at`", .0.display())]
    NoAddress(PathBuf),
    /// A hook's code couldn't be read or assembled.
    #[error("hook `{}`: {error}", file.display())]
    Hook {
        /// The hook's assembly file, as the manifest names it.
        file: PathBuf,
        /// Why it couldn't be read or assembled.
        error: AsmError,
    },
    /// Some patches can't be applied to the ROM.
    #[error("the patches can't be applied:\n{0}")]
    Patch(#[from] PatchError),
    /// An output file couldn't be written.
    #[error("failed to write `{}`: {source}", path.display())]
    Write {
        /// The output file.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Returns what the manifest of the game folder at `dir` writes over the
/// ROM, in order: its byte patches, then each hook's code over free space,
/// then each slot pointed at its hook.
///
/// # Errors
///
/// Returns a [`BuildError`] if a hook has slots but no pointer they hold, or
/// its code can't be read or assembled.
pub fn patches(dir: &Path, manifest: &Manifest) -> Result<Vec<Patch>, BuildError> {
    let mut patches: Vec<Patch> = (1..)
        .zip(&manifest.patches)
        .map(|(number, patch)| Patch {
            name: format!("patch {number}"),
            at: patch.at,
            bytes: patch.bytes.clone(),
            expect: patch.expect.clone().map_or(Expect::Any, Expect::Bytes),
        })
        .collect();
    let mut slots = Vec::new();
    for hook in &manifest.hooks {
        let name = format!("hook `{}`", hook.source.display());
        patches.push(Patch {
            name: name.clone(),
            at: hook.at,
            bytes: assemble(dir, hook)?.bytes,
            expect: Expect::Free,
        });
        if hook.slots.is_empty() {
            continue;
        }
        let replaces = hook
            .replaces
            .ok_or_else(|| BuildError::NoReplaces(hook.source.clone()))?;
        slots.extend(hook.slots.iter().map(|&slot| Patch {
            name: format!("a slot of {name}"),
            at: slot,
            bytes: (hook.at | 1).to_le_bytes().to_vec(),
            expect: Expect::Bytes(replaces.to_le_bytes().to_vec()),
        }));
    }
    patches.extend(slots);
    Ok(patches)
}

/// Assembles a hook's code at its address, with [`ORIGINAL`] defined as the
/// pointer it replaces, if any.
fn assemble(dir: &Path, hook: &Hook) -> Result<asm::Program, BuildError> {
    let error = |error| BuildError::Hook {
        file: hook.source.clone(),
        error,
    };
    let path = dir.join(&hook.source);
    let source =
        std::fs::read_to_string(&path).map_err(|source| error(AsmError::Read { path, source }))?;
    let names: Vec<(&str, u32)> = hook
        .replaces
        .map(|value| (ORIGINAL, value))
        .into_iter()
        .collect();
    asm::assemble_with(&source, hook.at, &names).map_err(error)
}

/// Assembles the Thumb code in `file` and lists it, for the `asm` command.
///
/// If a hook in the manifest of the game folder at `dir` names `file`, the
/// code is assembled the way [`run`] places it: at the hook's address unless
/// `at` is given, with [`ORIGINAL`] defined as the pointer it replaces.
/// Otherwise it's assembled at `at`, and `dir` needn't be a game folder.
///
/// # Errors
///
/// Returns a [`BuildError`] if the game folder has a manifest that can't be
/// read, `file` isn't a hook and no `at` is given, or the code can't be read
/// or assembled.
pub fn list(dir: &Path, file: &Path, at: Option<u32>) -> Result<String, BuildError> {
    match (find_hook(dir, file)?, at) {
        (Some(hook), at) => {
            let hook = Hook {
                at: at.unwrap_or(hook.at),
                ..hook
            };
            let mut report = assemble(dir, &hook)?.report();
            let _ = write!(report, " for hook `{}`", hook.source.display());
            if let Some(replaces) = hook.replaces {
                let _ = write!(report, ", with {ORIGINAL} as 0x{replaces:08X}");
            }
            Ok(report)
        }
        (None, Some(at)) => Ok(asm::run(file, at)?),
        (None, None) => Err(BuildError::NoAddress(file.to_path_buf())),
    }
}

/// Returns the first hook in the manifest of the game folder at `dir` whose
/// source is `file`, if the folder has a manifest.
fn find_hook(dir: &Path, file: &Path) -> Result<Option<Hook>, BuildError> {
    if !dir.join(crate::manifest::FILE_NAME).exists() {
        return Ok(None);
    }
    let Ok(file) = std::fs::canonicalize(file) else {
        return Ok(None);
    };
    let game = Game::open(dir)?;
    Ok(game.manifest.hooks.into_iter().find(|hook| {
        std::fs::canonicalize(dir.join(&hook.source)).is_ok_and(|source| source == file)
    }))
}

/// Returns the game's ROM with its manifest's byte patches and hooks
/// applied, along with what was written.
///
/// # Errors
///
/// Returns a [`BuildError`] if the ROM can't be read, a hook can't be
/// assembled, or a patch can't be applied.
pub fn patched_rom(game: &Game) -> Result<(Rom, Vec<Patch>), BuildError> {
    let rom = game.read_rom()?;
    let patches = patches(&game.dir, &game.manifest)?;
    let bytes = patch::apply(&rom.bytes, &patches)?;
    Ok((
        Rom {
            bytes,
            header: rom.header,
        },
        patches,
    ))
}

/// Applies the manifest's byte patches and hooks to the ROM of the game
/// folder at `dir` and writes the outputs it names, for the `build` command.
///
/// Nothing is written unless every patch can be applied, and the ROM itself
/// is only read.
///
/// # Errors
///
/// Returns a [`BuildError`] if the game folder or its ROM can't be read, the
/// manifest names no output or names the ROM as one, a hook can't be
/// assembled, a patch can't be applied, or an output can't be written.
pub fn run(dir: &Path) -> Result<String, BuildError> {
    let game = Game::open(dir)?;
    let output = &game.manifest.output;
    if output.rom.is_none() && output.ips.is_none() {
        return Err(BuildError::NoOutput);
    }
    let rom_path = std::fs::canonicalize(game.rom_path()).ok();
    if let Some(path) =
        output.rom.iter().chain(&output.ips).find(|path| {
            rom_path.is_some() && std::fs::canonicalize(dir.join(path)).ok() == rom_path
        })
    {
        return Err(BuildError::OverwriteRom(path.clone()));
    }
    let (rom, patches) = patched_rom(&game)?;

    let mut lines: Vec<String> = patches
        .iter()
        .map(|patch| {
            format!(
                "Patched {} at 0x{:08X} for {}",
                bytes(patch.bytes.len()),
                patch.at,
                patch.name
            )
        })
        .collect();
    if let Some(path) = &output.rom {
        write_output(&dir.join(path), &rom.bytes)?;
        lines.push(format!(
            "Wrote {} ({})",
            path.display(),
            bytes(rom.bytes.len())
        ));
    }
    if let Some(path) = &output.ips {
        let ips = patch::ips(&rom.bytes, &patches);
        write_output(&dir.join(path), &ips)?;
        lines.push(format!("Wrote {} ({})", path.display(), bytes(ips.len())));
    }
    Ok(lines.join("\n"))
}

/// Writes an output file, creating the directories it's in.
fn write_output(path: &Path, bytes: &[u8]) -> Result<(), BuildError> {
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, bytes))
        .map_err(|source| BuildError::Write {
            path: path.to_path_buf(),
            source,
        })
}

/// Counts bytes, as `1 byte` or `N bytes`.
fn bytes(count: usize) -> String {
    if count == 1 {
        "1 byte".into()
    } else {
        format!("{count} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest;
    use crate::rom::tests::valid_rom;
    use std::fs;

    fn game_dir(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(manifest::FILE_NAME),
            format!("version = 1\nrom = \"game.gba\"\n{manifest}"),
        )
        .unwrap();
        let mut rom = valid_rom("TEST");
        rom[0x180..].fill(0xFF);
        fs::write(dir.path().join("game.gba"), rom).unwrap();
        dir
    }

    /// The test ROM, with the given bytes written over it.
    fn rom_with(edits: &[(usize, &[u8])]) -> Vec<u8> {
        let mut rom = valid_rom("TEST");
        rom[0x180..].fill(0xFF);
        for &(offset, bytes) in edits {
            rom[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
        rom
    }

    const PATCHES: &str = "[[patch]]\n\
        at = 0x08000100\n\
        expect = \"00 00 00 00\"\n\
        bytes = \"81 01 00 08\"\n\
        [[patch]]\n\
        at = 0x08000004\n\
        bytes = \"AA\"\n";

    const HOOK: &str = "[[hook]]\n\
        source = \"hooks/guard.s\"\n\
        at = 0x08000180\n\
        replaces = 0x08000181\n\
        slots = [0x08000100, 0x08000104]\n";

    const GUARD: &str = "ldr r3, =ORIGINAL\nbx r3\n";

    #[test]
    fn build_patched_rom_and_ips() {
        let dir = game_dir(&format!(
            "[output]\nrom = \"out/mod.gba\"\nips = \"out/mod.ips\"\n{PATCHES}"
        ));
        assert_eq!(
            run(dir.path()).unwrap(),
            "Patched 4 bytes at 0x08000100 for patch 1\n\
             Patched 1 byte at 0x08000004 for patch 2\n\
             Wrote out/mod.gba (512 bytes)\n\
             Wrote out/mod.ips (23 bytes)"
        );
        let expected = rom_with(&[(0x100, &[0x81, 0x01, 0x00, 0x08]), (4, &[0xAA])]);
        assert_eq!(fs::read(dir.path().join("out/mod.gba")).unwrap(), expected);
        assert_eq!(
            fs::read(dir.path().join("out/mod.ips")).unwrap(),
            b"PATCH\x00\x01\x00\x00\x04\x81\x01\x00\x08\x00\x00\x04\x00\x01\xAAEOF"
        );
        assert_eq!(
            fs::read(dir.path().join("game.gba")).unwrap(),
            rom_with(&[])
        );
    }

    #[test]
    fn build_hooks() {
        let dir = game_dir(&format!(
            "[output]\nips = \"mod.ips\"\n{HOOK}\
             [[hook]]\nsource = \"hooks/plain.s\"\nat = 0x08000190\n\
             [[patch]]\nat = 0x08000104\nbytes = \"81 01 00 08\"\n"
        ));
        fs::create_dir(dir.path().join("hooks")).unwrap();
        fs::write(dir.path().join("hooks/guard.s"), GUARD).unwrap();
        fs::write(dir.path().join("hooks/plain.s"), "bx lr\n").unwrap();
        // The ROM already holds the replaced pointer in both slots, but the
        // patch writes over the second slot first.
        let rom = rom_with(&[
            (0x100, &[0x81, 0x01, 0x00, 0x08]),
            (0x104, &[0x81, 0x01, 0x00, 0x08]),
        ]);
        fs::write(dir.path().join("game.gba"), &rom).unwrap();
        let err = run(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the patches can't be applied:\n\
             a slot of hook `hooks/guard.s` at 0x08000104: it overlaps patch 1"
        );

        let manifest = fs::read_to_string(dir.path().join(manifest::FILE_NAME)).unwrap();
        let manifest = manifest.replace("at = 0x08000104\nbytes", "at = 0x08000108\nbytes");
        fs::write(dir.path().join(manifest::FILE_NAME), manifest).unwrap();
        assert_eq!(
            run(dir.path()).unwrap(),
            "Patched 4 bytes at 0x08000108 for patch 1\n\
             Patched 8 bytes at 0x08000180 for hook `hooks/guard.s`\n\
             Patched 2 bytes at 0x08000190 for hook `hooks/plain.s`\n\
             Patched 4 bytes at 0x08000100 for a slot of hook `hooks/guard.s`\n\
             Patched 4 bytes at 0x08000104 for a slot of hook `hooks/guard.s`\n\
             Wrote mod.ips (55 bytes)"
        );
        let mut expected = b"PATCH".to_vec();
        expected.extend_from_slice(b"\x00\x01\x08\x00\x04\x81\x01\x00\x08");
        expected.extend_from_slice(b"\x00\x01\x80\x00\x08\x00\x4B\x18\x47\x81\x01\x00\x08");
        expected.extend_from_slice(b"\x00\x01\x90\x00\x02\x70\x47");
        expected.extend_from_slice(b"\x00\x01\x00\x00\x04\x81\x01\x00\x08");
        expected.extend_from_slice(b"\x00\x01\x04\x00\x04\x81\x01\x00\x08");
        expected.extend_from_slice(b"EOF");
        assert_eq!(fs::read(dir.path().join("mod.ips")).unwrap(), expected);
    }

    #[test]
    fn patch_rom_in_memory() {
        let dir = game_dir(HOOK);
        fs::create_dir(dir.path().join("hooks")).unwrap();
        fs::write(dir.path().join("hooks/guard.s"), GUARD).unwrap();
        let rom = rom_with(&[
            (0x100, &[0x81, 0x01, 0x00, 0x08]),
            (0x104, &[0x81, 0x01, 0x00, 0x08]),
        ]);
        fs::write(dir.path().join("game.gba"), &rom).unwrap();

        let (modded, patches) = patched_rom(&Game::open(dir.path()).unwrap()).unwrap();
        assert_eq!(modded.header.title, "TEST");
        assert_eq!(patches.len(), 3);
        assert_eq!(
            &modded.bytes[0x180..0x188],
            [0x00, 0x4B, 0x18, 0x47, 0x81, 0x01, 0x00, 0x08]
        );
        assert_eq!(
            &modded.bytes[0x100..0x108],
            [0x81, 0x01, 0x00, 0x08, 0x81, 0x01, 0x00, 0x08]
        );
    }

    #[test]
    fn reject_bad_hooks() {
        let dir = game_dir(
            "[output]\nips = \"mod.ips\"\n[[hook]]\nsource = \"hook.s\"\nat = 0x08000180\nslots = [0x08000100]\n",
        );
        fs::write(dir.path().join("hook.s"), "bx lr\n").unwrap();
        let err = run(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "hook `hook.s` has slots but no `replaces` pointer for them to hold"
        );

        let dir = game_dir(
            "[output]\nips = \"mod.ips\"\n[[hook]]\nsource = \"hook.s\"\nat = 0x08000180\n",
        );
        fs::write(dir.path().join("hook.s"), "nop\nbx r16\n").unwrap();
        assert_eq!(
            run(dir.path()).unwrap_err().to_string(),
            "hook `hook.s`: line 2: `bx` doesn't take these operands"
        );
        fs::write(dir.path().join("hook.s"), "ORIGINAL: bx lr\n").unwrap();
        assert!(run(dir.path()).is_ok());

        fs::remove_file(dir.path().join("hook.s")).unwrap();
        let err = run(dir.path()).unwrap_err();
        assert!(
            err.to_string().starts_with(&format!(
                "hook `hook.s`: failed to read `{}`: ",
                dir.path().join("hook.s").display()
            )),
            "{err}"
        );

        let dir = game_dir(
            "[output]\nips = \"mod.ips\"\n[[hook]]\nsource = \"hook.s\"\nat = 0x0800017E\n",
        );
        fs::write(dir.path().join("hook.s"), "nop\nbx lr\n").unwrap();
        assert_eq!(
            run(dir.path()).unwrap_err().to_string(),
            "the patches can't be applied:\n\
             hook `hook.s` at 0x0800017E: the byte at 0x08000180 is 0xFF, but free space is all 0xFF or all 0x00"
        );
    }

    #[test]
    fn list_hook_as_built() {
        let dir = game_dir(HOOK);
        let hooks = dir.path().join("hooks");
        fs::create_dir(&hooks).unwrap();
        let guard = hooks.join("guard.s");
        fs::write(&guard, GUARD).unwrap();

        assert_eq!(
            list(dir.path(), &guard, None).unwrap(),
            "08000180: 4B00       ldr r3, =ORIGINAL\n\
             08000182: 4718       bx r3\n\
             08000184: 08000181   .word 0x08000181\n\
             Assembled 8 bytes at 0x08000180 for hook `hooks/guard.s`, with ORIGINAL as 0x08000181"
        );
        assert!(
            list(dir.path(), &guard, Some(0x0800_0200))
                .unwrap()
                .starts_with("08000200: 4B00       ldr r3, =ORIGINAL")
        );

        fs::write(&guard, "bx r16\n").unwrap();
        assert_eq!(
            list(dir.path(), &guard, None).unwrap_err().to_string(),
            "hook `hooks/guard.s`: line 1: `bx` doesn't take these operands"
        );
    }

    #[test]
    fn list_hook_without_replaces() {
        let dir = game_dir("[[hook]]\nsource = \"new.s\"\nat = 0x08000180\n");
        let new = dir.path().join("new.s");
        fs::write(&new, "bx lr\n").unwrap();
        assert_eq!(
            list(dir.path(), &new, None).unwrap(),
            "08000180: 4770       bx lr\nAssembled 2 bytes at 0x08000180 for hook `new.s`"
        );
    }

    #[test]
    fn list_code_outside_manifest() {
        let dir = game_dir(HOOK);
        let other = dir.path().join("other.s");
        fs::write(&other, "bx lr\n").unwrap();
        assert_eq!(
            list(dir.path(), &other, Some(0x0800_0000)).unwrap(),
            "08000000: 4770       bx lr\nAssembled 2 bytes at 0x08000000"
        );
        assert_eq!(
            list(dir.path(), &other, None).unwrap_err().to_string(),
            format!(
                "`{}` isn't a hook in the manifest, so give the address it starts at with `--at`",
                other.display()
            )
        );
        let missing = dir.path().join("missing.s");
        assert!(matches!(
            list(dir.path(), &missing, Some(0x0800_0000)),
            Err(BuildError::Asm(AsmError::Read { .. }))
        ));

        let folder = tempfile::tempdir().unwrap();
        assert!(list(folder.path(), &other, Some(0x0800_0000)).is_ok());
        assert!(matches!(
            list(folder.path(), &other, None),
            Err(BuildError::NoAddress(_))
        ));

        fs::write(dir.path().join(manifest::FILE_NAME), "version = ").unwrap();
        assert!(matches!(
            list(dir.path(), &other, Some(0x0800_0000)),
            Err(BuildError::Game(_))
        ));
    }

    #[test]
    fn build_only_ips() {
        let dir = game_dir("[output]\nips = \"mod.ips\"\n");
        assert_eq!(run(dir.path()).unwrap(), "Wrote mod.ips (8 bytes)");
        assert_eq!(fs::read(dir.path().join("mod.ips")).unwrap(), b"PATCHEOF");
    }

    #[test]
    fn reject_build_without_output() {
        let dir = game_dir(PATCHES);
        let err = run(dir.path()).unwrap_err();
        assert!(matches!(err, BuildError::NoOutput));
        assert_eq!(
            err.to_string(),
            "the manifest names no output; add `rom` or `ips` under `[output]`"
        );
    }

    #[test]
    fn reject_build_over_rom() {
        let dir = game_dir("[output]\nrom = \"./game.gba\"\n");
        let err = run(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the output `./game.gba` is the game's ROM, which gbamod only reads"
        );
        assert_eq!(
            fs::read(dir.path().join("game.gba")).unwrap(),
            rom_with(&[])
        );
    }

    #[test]
    fn reject_build_with_bad_patches() {
        let dir = game_dir(
            "[output]\nrom = \"mod.gba\"\n[[patch]]\nat = 0x08000100\nexpect = \"01\"\nbytes = \"02\"\n",
        );
        let err = run(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the patches can't be applied:\n\
             patch 1 at 0x08000100: the byte at 0x08000100 is 0x00, not the expected 0x01"
        );
        assert!(!dir.path().join("mod.gba").exists());
    }

    #[test]
    fn reject_build_with_missing_rom() {
        let dir = game_dir("[output]\nrom = \"mod.gba\"\n");
        fs::remove_file(dir.path().join("game.gba")).unwrap();
        assert!(matches!(run(dir.path()).unwrap_err(), BuildError::Game(_)));
        assert!(matches!(
            run(Path::new("/nonexistent")).unwrap_err(),
            BuildError::Game(_)
        ));
    }

    #[test]
    fn reject_unwritable_output() {
        let dir = game_dir("[output]\nips = \"game.gba/mod.ips\"\n");
        let err = run(dir.path()).unwrap_err();
        let path = dir.path().join("game.gba/mod.ips");
        assert!(
            err.to_string()
                .starts_with(&format!("failed to write `{}`: ", path.display())),
            "{err}"
        );
    }
}
