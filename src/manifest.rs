//! The `gbamod.toml` manifest that describes a game folder.

use crate::patch;
use std::path::PathBuf;

/// The file name of the manifest at the root of a game folder.
pub const FILE_NAME: &str = "gbamod.toml";

/// The contents of a manifest, independent of the version it was written in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The path of the game's ROM, relative to the game folder.
    pub rom: PathBuf,
    /// The files the `build` command writes.
    pub output: Output,
    /// The bytes to write over the ROM, in order.
    pub patches: Vec<BytePatch>,
    /// The hooks to place in the ROM, in order.
    pub hooks: Vec<Hook>,
}

/// Bytes to write over the ROM at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytePatch {
    /// The address of the first byte to write.
    pub at: u32,
    /// The bytes to write.
    pub bytes: Vec<u8>,
    /// The bytes the ROM must hold there before the patch, if given.
    pub expect: Option<Vec<u8>>,
}

/// Thumb code assembled into free space in the ROM, with pointers the game
/// follows redirected to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    /// The assembly file, relative to the game folder.
    pub source: PathBuf,
    /// The address the code is placed at, in free space.
    pub at: u32,
    /// The pointer the hook takes over, which its code can refer to as
    /// `ORIGINAL`.
    pub replaces: Option<u32>,
    /// The addresses of the words that hold `replaces`, which are pointed at
    /// the hook.
    pub slots: Vec<u32>,
}

/// The files the `build` command writes, relative to the game folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The patched ROM, if one is written.
    pub rom: Option<PathBuf>,
    /// The IPS patch, if one is written.
    pub ips: Option<PathBuf>,
}

/// The reason a manifest couldn't be parsed.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The text isn't valid TOML, or doesn't match its version's layout.
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    /// The manifest has no `version` field.
    #[error("missing the `version` field")]
    MissingVersion,
    /// The manifest's version isn't one this build of gbamod can read.
    #[error("unsupported version {0}")]
    UnsupportedVersion(i64),
}

/// The fields every manifest version shares, read first to choose a layout.
#[derive(serde::Deserialize)]
struct Versioned {
    version: Option<i64>,
}

/// The layout of a version 1 manifest.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestV1 {
    #[serde(rename = "version")]
    _version: i64,
    rom: PathBuf,
    #[serde(default)]
    output: OutputV1,
    #[serde(default, rename = "patch")]
    patches: Vec<PatchV1>,
    #[serde(default, rename = "hook")]
    hooks: Vec<HookV1>,
}

/// The `[output]` table of a version 1 manifest.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputV1 {
    rom: Option<PathBuf>,
    ips: Option<PathBuf>,
}

/// A `[[patch]]` entry of a version 1 manifest.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchV1 {
    at: u32,
    bytes: Hex,
    expect: Option<Hex>,
}

/// A `[[hook]]` entry of a version 1 manifest.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HookV1 {
    source: PathBuf,
    at: u32,
    replaces: Option<u32>,
    #[serde(default)]
    slots: Vec<u32>,
}

/// Bytes written in a manifest as a string of hex digits.
#[derive(serde::Deserialize)]
#[serde(try_from = "String")]
struct Hex(Vec<u8>);

impl TryFrom<String> for Hex {
    type Error = patch::HexError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        patch::parse_hex(&text).map(Self)
    }
}

impl Manifest {
    /// Parses the text of a manifest in any supported version.
    ///
    /// # Errors
    ///
    /// Returns a [`ManifestError`] if the text isn't valid TOML, has no
    /// supported version, or doesn't match the layout of its version.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let Versioned { version } = toml::from_str(text)?;
        match version {
            Some(1) => {
                let manifest: ManifestV1 = toml::from_str(text)?;
                Ok(Self {
                    rom: manifest.rom,
                    output: Output {
                        rom: manifest.output.rom,
                        ips: manifest.output.ips,
                    },
                    patches: manifest
                        .patches
                        .into_iter()
                        .map(|patch| BytePatch {
                            at: patch.at,
                            bytes: patch.bytes.0,
                            expect: patch.expect.map(|expect| expect.0),
                        })
                        .collect(),
                    hooks: manifest
                        .hooks
                        .into_iter()
                        .map(|hook| Hook {
                            source: hook.source,
                            at: hook.at,
                            replaces: hook.replaces,
                            slots: hook.slots,
                        })
                        .collect(),
                })
            }
            Some(version) => Err(ManifestError::UnsupportedVersion(version)),
            None => Err(ManifestError::MissingVersion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_v1_manifest() {
        let manifest = Manifest::parse("version = 1\nrom = \"roms/game.gba\"\n").unwrap();
        assert_eq!(
            manifest,
            Manifest {
                rom: "roms/game.gba".into(),
                ..Manifest::default()
            }
        );
    }

    #[test]
    fn parse_v1_manifest_with_patches_and_hooks() {
        let manifest = Manifest::parse(
            "version = 1\n\
             rom = \"roms/game.gba\"\n\
             [output]\n\
             rom = \"out/mod.gba\"\n\
             ips = \"out/mod.ips\"\n\
             [[patch]]\n\
             at = 0x0800FA68\n\
             expect = \"79 6A 00 08\"\n\
             bytes = \"01 F0 CB 08\"\n\
             [[patch]]\n\
             at = 0x08CBF000\n\
             bytes = \"10B5\"\n\
             [[hook]]\n\
             source = \"hooks/guard.s\"\n\
             at = 0x08CBF080\n\
             replaces = 0x0804FD09\n\
             slots = [0x08CBC850, 0x08CBC9F0]\n\
             [[hook]]\n\
             source = \"hooks/new.s\"\n\
             at = 0x08CBF100\n",
        )
        .unwrap();
        assert_eq!(
            manifest,
            Manifest {
                rom: "roms/game.gba".into(),
                output: Output {
                    rom: Some("out/mod.gba".into()),
                    ips: Some("out/mod.ips".into()),
                },
                patches: vec![
                    BytePatch {
                        at: 0x0800_FA68,
                        bytes: vec![0x01, 0xF0, 0xCB, 0x08],
                        expect: Some(vec![0x79, 0x6A, 0x00, 0x08]),
                    },
                    BytePatch {
                        at: 0x08CB_F000,
                        bytes: vec![0x10, 0xB5],
                        expect: None,
                    },
                ],
                hooks: vec![
                    Hook {
                        source: "hooks/guard.s".into(),
                        at: 0x08CB_F080,
                        replaces: Some(0x0804_FD09),
                        slots: vec![0x08CB_C850, 0x08CB_C9F0],
                    },
                    Hook {
                        source: "hooks/new.s".into(),
                        at: 0x08CB_F100,
                        replaces: None,
                        slots: vec![],
                    },
                ],
            }
        );
    }

    #[test]
    fn reject_v1_manifest_with_bad_patch() {
        let err = Manifest::parse(
            "version = 1\nrom = \"game.gba\"\n[[patch]]\nat = 0x08000000\nbytes = \"0x10\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("`0x10` isn't hex bytes"), "{err}");

        let err = Manifest::parse(
            "version = 1\nrom = \"game.gba\"\n[[patch]]\nat = -1\nbytes = \"10\"\n",
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn reject_invalid_toml() {
        let err = Manifest::parse("version = ").unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn reject_missing_version() {
        let err = Manifest::parse("rom = \"game.gba\"").unwrap_err();
        assert!(matches!(err, ManifestError::MissingVersion));
        assert_eq!(err.to_string(), "missing the `version` field");
    }

    #[test]
    fn reject_unsupported_version() {
        let err = Manifest::parse("version = 2\nrom = \"game.gba\"").unwrap_err();
        assert!(matches!(err, ManifestError::UnsupportedVersion(2)));
        assert_eq!(err.to_string(), "unsupported version 2");
    }

    #[test]
    fn reject_v1_manifest_without_rom() {
        let err = Manifest::parse("version = 1").unwrap_err();
        assert!(err.to_string().contains("missing field `rom`"));
    }

    #[test]
    fn reject_v1_manifest_with_unknown_field() {
        let err = Manifest::parse("version = 1\nrom = \"game.gba\"\nroms = 1").unwrap_err();
        assert!(err.to_string().contains("unknown field `roms`"));
    }
}
