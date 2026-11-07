//! Searching RAM dumps for values that changed across them in a known way.

use crate::memory::{self, MemoryError, Width};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// The most matches a search lists before it only counts the rest.
pub const MAX_LISTED: usize = 200;

/// How a value must change across the dumps, in order, to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Never rises, and falls at least once.
    Dec,
    /// Never falls, and rises at least once.
    Inc,
    /// Equal in every dump.
    Same,
    /// Differs between the first and last dump.
    Changed,
    /// Equal to the number in every dump.
    Eq(u32),
    /// Equal to each number in the dump at the same position, where `None`
    /// matches anything.
    Seq(Vec<Option<u32>>),
}

/// The reason a search couldn't run.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The text isn't a known rule.
    #[error("unknown rule `{0}`; expected dec, inc, same, changed, eq=N, or seq=A,B,...")]
    UnknownRule(String),
    /// A number in the rule isn't valid.
    #[error(transparent)]
    Number(#[from] MemoryError),
    /// A `seq` rule doesn't list one value per dump.
    #[error("a seq rule needs one value per dump, but lists {values} for {dumps}")]
    SeqLength {
        /// The number of values the rule lists.
        values: usize,
        /// The number of dumps.
        dumps: usize,
    },
    /// A dump couldn't be read.
    #[error("failed to read `{}`: {source}", path.display())]
    Read {
        /// The dump file.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A file isn't the size of a RAM dump.
    #[error("`{}` is {size} bytes, but a RAM dump is {} bytes", path.display(), memory::RAM_DUMP_SIZE)]
    Size {
        /// The dump file.
        path: PathBuf,
        /// The file's size in bytes.
        size: usize,
    },
}

impl Rule {
    /// Returns whether the values, one per dump in order, follow this rule.
    #[must_use]
    pub fn matches(&self, values: &[u32]) -> bool {
        let (Some(&first), Some(&last)) = (values.first(), values.last()) else {
            return false;
        };
        match self {
            Self::Dec => values.windows(2).all(|pair| pair[1] <= pair[0]) && last < first,
            Self::Inc => values.windows(2).all(|pair| pair[1] >= pair[0]) && last > first,
            Self::Same => values.iter().all(|&value| value == first),
            Self::Changed => last != first,
            Self::Eq(number) => values.iter().all(|value| value == number),
            Self::Seq(wanted) => wanted
                .iter()
                .zip(values)
                .all(|(want, value)| want.is_none_or(|want| want == *value)),
        }
    }
}

impl FromStr for Rule {
    type Err = SearchError;

    /// Parses a rule such as `dec`, `eq=288`, or `seq=288,*,0x111`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if let Some(number) = text.strip_prefix("eq=") {
            return Ok(Self::Eq(memory::parse_number(number)?));
        }
        if let Some(list) = text.strip_prefix("seq=") {
            let wanted = list
                .split(',')
                .map(|value| match value {
                    "*" => Ok(None),
                    _ => memory::parse_number(value).map(Some),
                })
                .collect::<Result<_, _>>()?;
            return Ok(Self::Seq(wanted));
        }
        match text {
            "dec" => Ok(Self::Dec),
            "inc" => Ok(Self::Inc),
            "same" => Ok(Self::Same),
            "changed" => Ok(Self::Changed),
            _ => Err(SearchError::UnknownRule(text.to_string())),
        }
    }
}

/// What to search RAM dumps for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The width of the values, read little-endian at aligned offsets.
    pub width: Width,
    /// How the values must change across the dumps.
    pub rule: Rule,
    /// The smallest value allowed in every dump.
    pub min: u32,
    /// The largest value allowed in every dump.
    pub max: u32,
}

/// An address whose values matched a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The address the values were read from.
    pub address: u32,
    /// The value at the address in each dump, in order.
    pub values: Vec<u32>,
}

impl Query {
    /// Returns every address whose values in `dumps` stay within range and
    /// follow the rule. Each dump must be laid out as
    /// [`memory::ram_addresses`].
    #[must_use]
    pub fn find(&self, dumps: &[Vec<u8>]) -> Vec<Match> {
        let width = self.width.bytes() as usize;
        memory::ram_addresses()
            .enumerate()
            .step_by(width)
            .filter_map(|(offset, address)| {
                let values: Vec<u32> = dumps
                    .iter()
                    .map(|dump| self.width.read(dump.get(offset..).unwrap_or_default()))
                    .collect();
                let in_range = values
                    .iter()
                    .all(|value| (self.min..=self.max).contains(value));
                (in_range && self.rule.matches(&values)).then_some(Match { address, values })
            })
            .collect()
    }

    /// Reads the RAM dumps at `paths` and lists the addresses that match, for
    /// the `search` command.
    ///
    /// # Errors
    ///
    /// Returns a [`SearchError`] if a dump can't be read or isn't the size of
    /// a RAM dump, or a `seq` rule doesn't list one value per dump.
    pub fn run(&self, paths: &[PathBuf]) -> Result<String, SearchError> {
        if let Rule::Seq(wanted) = &self.rule
            && wanted.len() != paths.len()
        {
            return Err(SearchError::SeqLength {
                values: wanted.len(),
                dumps: paths.len(),
            });
        }
        let dumps = paths
            .iter()
            .map(|path| read_dump(path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(report(&self.find(&dumps)))
    }
}

/// Reads a RAM dump, checking that it's the right size.
fn read_dump(path: &Path) -> Result<Vec<u8>, SearchError> {
    let dump = std::fs::read(path).map_err(|source| SearchError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if dump.len() != memory::RAM_DUMP_SIZE as usize {
        return Err(SearchError::Size {
            path: path.to_path_buf(),
            size: dump.len(),
        });
    }
    Ok(dump)
}

/// Lists up to [`MAX_LISTED`] matches as an address followed by its values in
/// decimal, then counts them all.
fn report(matches: &[Match]) -> String {
    let mut report = String::new();
    for found in matches.iter().take(MAX_LISTED) {
        let _ = write!(report, "{:08X}:", found.address);
        for value in &found.values {
            let _ = write!(report, " {value}");
        }
        report.push('\n');
    }
    match matches.len() {
        0 => report.push_str("No matches"),
        1 => report.push_str("1 match"),
        count if count > MAX_LISTED => {
            let _ = write!(report, "{count} matches, listing the first {MAX_LISTED}");
        }
        count => {
            let _ = write!(report, "{count} matches");
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn query(width: Width, rule: Rule) -> Query {
        Query {
            width,
            rule,
            min: 0,
            max: u32::MAX,
        }
    }

    fn blank_dump() -> Vec<u8> {
        vec![0; memory::RAM_DUMP_SIZE as usize]
    }

    /// Returns a blank dump with `value` written at the dump offset of
    /// `address`.
    fn dump_with(address: u32, width: Width, value: u32) -> Vec<u8> {
        let mut dump = blank_dump();
        let offset = memory::ram_addresses()
            .position(|known| known == address)
            .unwrap();
        let bytes = value.to_le_bytes();
        dump[offset..offset + width.bytes() as usize]
            .copy_from_slice(&bytes[..width.bytes() as usize]);
        dump
    }

    #[test]
    fn parse_rules() {
        assert_eq!("dec".parse::<Rule>().unwrap(), Rule::Dec);
        assert_eq!("inc".parse::<Rule>().unwrap(), Rule::Inc);
        assert_eq!("same".parse::<Rule>().unwrap(), Rule::Same);
        assert_eq!("changed".parse::<Rule>().unwrap(), Rule::Changed);
        assert_eq!("eq=0x120".parse::<Rule>().unwrap(), Rule::Eq(288));
        assert_eq!(
            "seq=288,*,273".parse::<Rule>().unwrap(),
            Rule::Seq(vec![Some(288), None, Some(273)])
        );
    }

    #[test]
    fn reject_invalid_rules() {
        assert_eq!(
            "down".parse::<Rule>().unwrap_err().to_string(),
            "unknown rule `down`; expected dec, inc, same, changed, eq=N, or seq=A,B,..."
        );
        assert!(matches!(
            "eq=lots".parse::<Rule>(),
            Err(SearchError::Number(MemoryError::InvalidNumber(_)))
        ));
        assert!(matches!(
            "seq=1,,2".parse::<Rule>(),
            Err(SearchError::Number(MemoryError::InvalidNumber(_)))
        ));
    }

    #[test]
    fn match_rules() {
        assert!(Rule::Dec.matches(&[280, 280, 273]));
        assert!(!Rule::Dec.matches(&[280, 281, 273]));
        assert!(!Rule::Dec.matches(&[280, 280]));
        assert!(Rule::Inc.matches(&[1, 1, 2]));
        assert!(!Rule::Inc.matches(&[1, 0, 2]));
        assert!(!Rule::Inc.matches(&[1, 1]));
        assert!(Rule::Same.matches(&[5, 5, 5]));
        assert!(!Rule::Same.matches(&[5, 6, 5]));
        assert!(Rule::Changed.matches(&[5, 5, 6]));
        assert!(!Rule::Changed.matches(&[5, 6, 5]));
        assert!(Rule::Eq(7).matches(&[7, 7]));
        assert!(!Rule::Eq(7).matches(&[7, 8]));
        let seq = Rule::Seq(vec![Some(1), None, Some(3)]);
        assert!(seq.matches(&[1, 99, 3]));
        assert!(!seq.matches(&[1, 99, 4]));
        assert!(!Rule::Same.matches(&[]));
    }

    #[test]
    fn find_falling_value() {
        let dumps = [
            dump_with(0x0200_316C, Width::U16, 280),
            dump_with(0x0200_316C, Width::U16, 273),
        ];
        assert_eq!(
            query(Width::U16, Rule::Dec).find(&dumps),
            vec![Match {
                address: 0x0200_316C,
                values: vec![280, 273],
            }]
        );
    }

    #[test]
    fn find_values_in_internal_ram() {
        let dumps = [dump_with(0x0300_0010, Width::U32, 0x0200_315C)];
        let found = query(Width::U32, Rule::Eq(0x0200_315C)).find(&dumps);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address, 0x0300_0010);
    }

    #[test]
    fn find_only_aligned_values() {
        let dumps = [dump_with(0x0200_0001, Width::U8, 9)];
        assert_eq!(query(Width::U16, Rule::Eq(9)).find(&dumps).len(), 0);
        assert_eq!(query(Width::U8, Rule::Eq(9)).find(&dumps).len(), 1);
    }

    #[test]
    fn find_values_in_range() {
        let dumps = [
            dump_with(0x0200_0000, Width::U32, 0x0200_0000),
            dump_with(0x0200_0000, Width::U32, 0x0100_0000),
        ];
        let mut search = query(Width::U32, Rule::Dec);
        assert_eq!(search.find(&dumps).len(), 1);
        search.max = 0xFFFF;
        assert_eq!(search.find(&dumps).len(), 0);
        search.max = u32::MAX;
        search.min = 0x0180_0000;
        assert_eq!(search.find(&dumps).len(), 0);
    }

    #[test]
    fn report_matches() {
        let one = Match {
            address: 0x0200_316C,
            values: vec![280, 273],
        };
        assert_eq!(report(&[]), "No matches");
        assert_eq!(
            report(std::slice::from_ref(&one)),
            "0200316C: 280 273\n1 match"
        );
        assert_eq!(
            report(&[one.clone(), one.clone()]),
            "0200316C: 280 273\n0200316C: 280 273\n2 matches"
        );

        let many = vec![one; MAX_LISTED + 1];
        let listed = report(&many);
        assert_eq!(listed.lines().count(), MAX_LISTED + 1);
        assert!(listed.ends_with("\n201 matches, listing the first 200"));
    }

    #[test]
    fn run_on_dump_files() {
        let dir = tempfile::tempdir().unwrap();
        let before = dir.path().join("before.bin");
        let after = dir.path().join("after.bin");
        fs::write(&before, dump_with(0x0200_316C, Width::U16, 280)).unwrap();
        fs::write(&after, dump_with(0x0200_316C, Width::U16, 273)).unwrap();

        let search = query(Width::U16, "seq=280,273".parse().unwrap());
        assert_eq!(
            search.run(&[before, after]).unwrap(),
            "0200316C: 280 273\n1 match"
        );
    }

    #[test]
    fn reject_seq_of_wrong_length() {
        let search = query(Width::U16, "seq=280,273".parse().unwrap());
        let err = search.run(&[PathBuf::from("ram.bin")]).unwrap_err();
        assert!(matches!(
            err,
            SearchError::SeqLength {
                values: 2,
                dumps: 1
            }
        ));
        assert_eq!(
            err.to_string(),
            "a seq rule needs one value per dump, but lists 2 for 1"
        );
    }

    #[test]
    fn reject_missing_dump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.bin");
        let err = query(Width::U8, Rule::Same)
            .run(std::slice::from_ref(&path))
            .unwrap_err();
        assert!(matches!(err, SearchError::Read { .. }));
        assert!(
            err.to_string()
                .starts_with(&format!("failed to read `{}`: ", path.display()))
        );
    }

    #[test]
    fn reject_dump_of_wrong_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.bin");
        fs::write(&path, [0; 16]).unwrap();
        let err = query(Width::U8, Rule::Same)
            .run(std::slice::from_ref(&path))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "`{}` is 16 bytes, but a RAM dump is 294912 bytes",
                path.display()
            )
        );
    }
}
