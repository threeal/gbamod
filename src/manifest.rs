//! The `gbamod.toml` manifest that describes a game folder.

use std::path::PathBuf;

/// The file name of the manifest at the root of a game folder.
pub const FILE_NAME: &str = "gbamod.toml";

/// The contents of a manifest, independent of the version it was written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The path of the game's ROM, relative to the game folder.
    pub rom: PathBuf,
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
                Ok(Self { rom: manifest.rom })
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
                rom: "roms/game.gba".into()
            }
        );
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
