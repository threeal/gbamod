//! Searching RAM dumps for values that changed across them in a known way.
//!
//! Dumps come in groups, each taken in the same state of the game. An address
//! matches only when its value is the same across every dump in a group, and
//! the rule then applies to the one value of each group, in order. A group of
//! one dump is a dump on its own.

use crate::memory::{self, MemoryError, Width};
use std::convert::Infallible;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// The most matches a search lists before it only counts the rest.
pub const MAX_LISTED: usize = 200;

/// How a value must change across the groups of dumps, in order, to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Never rises, and falls at least once, by no more than the bound from
    /// one group to the next.
    Dec(Option<Bound>),
    /// Never falls, and rises at least once, by no more than the bound from
    /// one group to the next.
    Inc(Option<Bound>),
    /// Equal in every group.
    Same,
    /// Differs between the first and last group.
    Changed,
    /// Differs between every two groups.
    Distinct,
    /// Equal to the number in every group.
    Eq(u32),
    /// Equal to each number in the group at the same position, where `None`
    /// matches anything.
    Seq(Vec<Option<u32>>),
}

/// The most a value may move from one group to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// By the number.
    Amount(u32),
    /// By the percentage of the earlier value.
    Percent(u32),
}

impl Bound {
    /// Returns whether a value moving from `from` to `to` stays within this
    /// bound.
    #[must_use]
    pub fn allows(self, from: u32, to: u32) -> bool {
        let moved = u64::from(from.abs_diff(to));
        match self {
            Self::Amount(number) => moved <= u64::from(number),
            Self::Percent(percent) => moved * 100 <= u64::from(from) * u64::from(percent),
        }
    }
}

impl FromStr for Bound {
    type Err = MemoryError;

    /// Parses a number, such as `50`, or a percentage, such as `25%`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.strip_suffix('%').map_or_else(
            || memory::parse_number(text).map(Self::Amount),
            |percent| memory::parse_number(percent).map(Self::Percent),
        )
    }
}

/// The reason a search couldn't run.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The text isn't a known rule.
    #[error(
        "unknown rule `{0}`; expected dec, inc, same, changed, distinct, eq=N, or seq=A,B,..., where dec and inc take an optional <=N or <=N%"
    )]
    UnknownRule(String),
    /// A number in the rule isn't valid.
    #[error(transparent)]
    Number(#[from] MemoryError),
    /// A `seq` rule doesn't list one value per group of dumps.
    #[error("a seq rule needs one value per group of dumps, but lists {values} for {groups}")]
    SeqLength {
        /// The number of values the rule lists.
        values: usize,
        /// The number of groups.
        groups: usize,
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
    /// Returns whether the values, one per group in order, follow this rule.
    #[must_use]
    pub fn matches(&self, values: &[u32]) -> bool {
        let (Some(&first), Some(&last)) = (values.first(), values.last()) else {
            return false;
        };
        match self {
            Self::Dec(bound) => {
                values
                    .windows(2)
                    .all(|pair| pair[1] <= pair[0] && within(*bound, pair))
                    && last < first
            }
            Self::Inc(bound) => {
                values
                    .windows(2)
                    .all(|pair| pair[1] >= pair[0] && within(*bound, pair))
                    && last > first
            }
            Self::Same => values.iter().all(|&value| value == first),
            Self::Changed => last != first,
            Self::Distinct => values
                .iter()
                .enumerate()
                .all(|(index, value)| !values[..index].contains(value)),
            Self::Eq(number) => values.iter().all(|value| value == number),
            Self::Seq(wanted) => wanted
                .iter()
                .zip(values)
                .all(|(want, value)| want.is_none_or(|want| want == *value)),
        }
    }
}

/// Returns whether a pair of values, from one group to the next, stays
/// within the bound, if there is one.
fn within(bound: Option<Bound>, pair: &[u32]) -> bool {
    bound.is_none_or(|bound| bound.allows(pair[0], pair[1]))
}

impl FromStr for Rule {
    type Err = SearchError;

    /// Parses a rule such as `dec`, `dec<=25%`, `eq=288`, or
    /// `seq=288,*,0x111`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if let Some(bound) = text.strip_prefix("dec<=") {
            return Ok(Self::Dec(Some(bound.parse()?)));
        }
        if let Some(bound) = text.strip_prefix("inc<=") {
            return Ok(Self::Inc(Some(bound.parse()?)));
        }
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
            "dec" => Ok(Self::Dec(None)),
            "inc" => Ok(Self::Inc(None)),
            "same" => Ok(Self::Same),
            "changed" => Ok(Self::Changed),
            "distinct" => Ok(Self::Distinct),
            _ => Err(SearchError::UnknownRule(text.to_string())),
        }
    }
}

/// The paths of a group of RAM dumps taken in the same state of the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group(pub Vec<PathBuf>);

impl FromStr for Group {
    type Err = Infallible;

    /// Parses the paths of a group joined by commas, such as `a.bin,b.bin`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.split(',').map(PathBuf::from).collect()))
    }
}

/// What to search RAM dumps for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The width of the values, read little-endian at aligned offsets.
    pub width: Width,
    /// How the values must change across the groups of dumps.
    pub rule: Rule,
    /// The smallest value allowed in every group.
    pub min: u32,
    /// The largest value allowed in every group.
    pub max: u32,
}

/// An address whose values matched a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The address the values were read from.
    pub address: u32,
    /// The value at the address in each group, in order.
    pub values: Vec<u32>,
}

impl Query {
    /// Returns every address whose value is the same across each group of
    /// `groups`, and whose values, one per group, stay within range and follow
    /// the rule. Each dump must be laid out as [`memory::ram_addresses`].
    #[must_use]
    pub fn find(&self, groups: &[Vec<Vec<u8>>]) -> Vec<Match> {
        let width = self.width.bytes() as usize;
        memory::ram_addresses()
            .enumerate()
            .step_by(width)
            .filter_map(|(offset, address)| {
                let values = groups
                    .iter()
                    .map(|dumps| self.group_value(dumps, offset))
                    .collect::<Option<Vec<u32>>>()?;
                let in_range = values
                    .iter()
                    .all(|value| (self.min..=self.max).contains(value));
                (in_range && self.rule.matches(&values)).then_some(Match { address, values })
            })
            .collect()
    }

    /// Returns the value at `offset` if it's the same in every dump of a
    /// group, and `None` if it differs or the group is empty.
    fn group_value(&self, dumps: &[Vec<u8>], offset: usize) -> Option<u32> {
        let mut values = dumps
            .iter()
            .map(|dump| self.width.read(dump.get(offset..).unwrap_or_default()));
        let first = values.next()?;
        values.all(|value| value == first).then_some(first)
    }

    /// Reads the groups of RAM dumps and lists the addresses that match, for
    /// the `search` command.
    ///
    /// # Errors
    ///
    /// Returns a [`SearchError`] if a dump can't be read or isn't the size of
    /// a RAM dump, or a `seq` rule doesn't list one value per group.
    pub fn run(&self, groups: &[Group]) -> Result<String, SearchError> {
        if let Rule::Seq(wanted) = &self.rule
            && wanted.len() != groups.len()
        {
            return Err(SearchError::SeqLength {
                values: wanted.len(),
                groups: groups.len(),
            });
        }
        let dumps = groups
            .iter()
            .map(|Group(paths)| paths.iter().map(|path| read_dump(path)).collect())
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
        assert_eq!("dec".parse::<Rule>().unwrap(), Rule::Dec(None));
        assert_eq!("inc".parse::<Rule>().unwrap(), Rule::Inc(None));
        assert_eq!(
            "dec<=25%".parse::<Rule>().unwrap(),
            Rule::Dec(Some(Bound::Percent(25)))
        );
        assert_eq!(
            "inc<=0x10".parse::<Rule>().unwrap(),
            Rule::Inc(Some(Bound::Amount(16)))
        );
        assert_eq!("same".parse::<Rule>().unwrap(), Rule::Same);
        assert_eq!("changed".parse::<Rule>().unwrap(), Rule::Changed);
        assert_eq!("distinct".parse::<Rule>().unwrap(), Rule::Distinct);
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
            "unknown rule `down`; expected dec, inc, same, changed, distinct, eq=N, or seq=A,B,..., where dec and inc take an optional <=N or <=N%"
        );
        assert!(matches!(
            "dec<=lots%".parse::<Rule>(),
            Err(SearchError::Number(MemoryError::InvalidNumber(_)))
        ));
        assert!(matches!(
            "inc<=".parse::<Rule>(),
            Err(SearchError::Number(MemoryError::InvalidNumber(_)))
        ));
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
        assert!(Rule::Dec(None).matches(&[280, 280, 273]));
        assert!(!Rule::Dec(None).matches(&[280, 281, 273]));
        assert!(!Rule::Dec(None).matches(&[280, 280]));
        assert!(Rule::Inc(None).matches(&[1, 1, 2]));
        assert!(!Rule::Inc(None).matches(&[1, 0, 2]));
        assert!(!Rule::Inc(None).matches(&[1, 1]));
        assert!(Rule::Same.matches(&[5, 5, 5]));
        assert!(!Rule::Same.matches(&[5, 6, 5]));
        assert!(Rule::Changed.matches(&[5, 5, 6]));
        assert!(!Rule::Changed.matches(&[5, 6, 5]));
        assert!(Rule::Distinct.matches(&[0, 1, 2]));
        assert!(!Rule::Distinct.matches(&[0, 1, 0]));
        assert!(Rule::Eq(7).matches(&[7, 7]));
        assert!(!Rule::Eq(7).matches(&[7, 8]));
        let seq = Rule::Seq(vec![Some(1), None, Some(3)]);
        assert!(seq.matches(&[1, 99, 3]));
        assert!(!seq.matches(&[1, 99, 4]));
        assert!(!Rule::Same.matches(&[]));
    }

    #[test]
    fn match_bounded_rules() {
        let quarter = Rule::Dec(Some(Bound::Percent(25)));
        assert!(quarter.matches(&[280, 273, 270]));
        assert!(quarter.matches(&[280, 210]));
        assert!(!quarter.matches(&[280, 209]));
        assert!(!quarter.matches(&[280, 0]));
        assert!(!quarter.matches(&[280, 280]));
        assert!(!quarter.matches(&[280, 281, 273]));
        let small = Rule::Inc(Some(Bound::Amount(10)));
        assert!(small.matches(&[0, 10, 20]));
        assert!(!small.matches(&[0, 11]));
        assert!(!small.matches(&[5, 4, 6]));
    }

    #[test]
    fn bound_moves() {
        assert!(Bound::Percent(25).allows(4, 3));
        assert!(!Bound::Percent(25).allows(3, 2));
        assert!(Bound::Percent(100).allows(u32::MAX, 0));
        assert!(!Bound::Percent(50).allows(0, 1));
        assert!(Bound::Amount(u32::MAX).allows(0, u32::MAX));
        assert!(!Bound::Amount(1).allows(2, 0));
    }

    #[test]
    fn parse_groups() {
        assert_eq!(
            "a.bin".parse::<Group>().unwrap(),
            Group(vec![PathBuf::from("a.bin")])
        );
        assert_eq!(
            "a.bin,b.bin".parse::<Group>().unwrap(),
            Group(vec![PathBuf::from("a.bin"), PathBuf::from("b.bin")])
        );
    }

    #[test]
    fn find_falling_value() {
        let dumps = [
            vec![dump_with(0x0200_316C, Width::U16, 280)],
            vec![dump_with(0x0200_316C, Width::U16, 273)],
        ];
        assert_eq!(
            query(Width::U16, Rule::Dec(None)).find(&dumps),
            vec![Match {
                address: 0x0200_316C,
                values: vec![280, 273],
            }]
        );
    }

    #[test]
    fn find_values_same_within_groups() {
        let player = dump_with(0x0201_6BA9, Width::U8, 0);
        let mut moved = player.clone();
        moved[0] = 7;
        let groups = [
            vec![player.clone(), moved],
            vec![dump_with(0x0201_6BA9, Width::U8, 1)],
            vec![dump_with(0x0201_6BA9, Width::U8, 2)],
        ];
        assert_eq!(
            query(Width::U8, Rule::Distinct).find(&groups),
            vec![Match {
                address: 0x0201_6BA9,
                values: vec![0, 1, 2],
            }]
        );
        assert_eq!(query(Width::U8, Rule::Same).find(&[vec![]]), vec![]);
    }

    #[test]
    fn find_values_in_internal_ram() {
        let dumps = [vec![dump_with(0x0300_0010, Width::U32, 0x0200_315C)]];
        let found = query(Width::U32, Rule::Eq(0x0200_315C)).find(&dumps);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address, 0x0300_0010);
    }

    #[test]
    fn find_only_aligned_values() {
        let dumps = [vec![dump_with(0x0200_0001, Width::U8, 9)]];
        assert_eq!(query(Width::U16, Rule::Eq(9)).find(&dumps).len(), 0);
        assert_eq!(query(Width::U8, Rule::Eq(9)).find(&dumps).len(), 1);
    }

    #[test]
    fn find_values_in_range() {
        let dumps = [
            vec![dump_with(0x0200_0000, Width::U32, 0x0200_0000)],
            vec![dump_with(0x0200_0000, Width::U32, 0x0100_0000)],
        ];
        let mut search = query(Width::U32, Rule::Dec(None));
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
        let again = dir.path().join("again.bin");
        let after = dir.path().join("after.bin");
        fs::write(&before, dump_with(0x0200_316C, Width::U16, 280)).unwrap();
        fs::write(&again, dump_with(0x0200_316C, Width::U16, 280)).unwrap();
        fs::write(&after, dump_with(0x0200_316C, Width::U16, 273)).unwrap();

        let search = query(Width::U16, "seq=280,273".parse().unwrap());
        assert_eq!(
            search
                .run(&[Group(vec![before, again]), Group(vec![after])])
                .unwrap(),
            "0200316C: 280 273\n1 match"
        );
    }

    #[test]
    fn reject_seq_of_wrong_length() {
        let search = query(Width::U16, "seq=280,273".parse().unwrap());
        let err = search
            .run(&[Group(vec![PathBuf::from("a.bin"), PathBuf::from("b.bin")])])
            .unwrap_err();
        assert!(matches!(
            err,
            SearchError::SeqLength {
                values: 2,
                groups: 1
            }
        ));
        assert_eq!(
            err.to_string(),
            "a seq rule needs one value per group of dumps, but lists 2 for 1"
        );
    }

    #[test]
    fn reject_missing_dump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.bin");
        let err = query(Width::U8, Rule::Same)
            .run(&[Group(vec![path.clone()])])
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
            .run(&[Group(vec![path.clone()])])
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
