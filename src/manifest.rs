//! The `gbamod.toml` manifest that describes a game folder.

use crate::patch::{self, Patch};
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
    /// The patches to write over the ROM, in order.
    pub patches: Vec<Patch>,
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
                        .map(|patch| Patch {
                            at: patch.at,
                            bytes: patch.bytes.0,
                            expect: patch.expect.map(|expect| expect.0),
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
    fn parse_v1_manifest_with_patches() {
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
             bytes = \"10B5\"\n",
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
                    Patch {
                        at: 0x0800_FA68,
                        bytes: vec![0x01, 0xF0, 0xCB, 0x08],
                        expect: Some(vec![0x79, 0x6A, 0x00, 0x08]),
                    },
                    Patch {
                        at: 0x08CB_F000,
                        bytes: vec![0x10, 0xB5],
                        expect: None,
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
