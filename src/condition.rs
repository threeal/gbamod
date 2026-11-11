//! Conditions on a value in memory, which a session's run stops at.

use crate::memory::{self, MemoryError, Width};
use std::str::FromStr;

/// A value in memory and what it must do for a run to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Condition {
    /// The address of the value.
    pub address: u32,
    /// The width of the value, read little-endian.
    pub width: Width,
    /// What the value must do.
    pub test: Test,
}

/// What a value must do for its condition to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Test {
    /// Equal the number.
    Equals(u32),
    /// Differ from its value when the run started.
    Changed,
}

/// The reason text isn't a condition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConditionError {
    /// The text has no `=` between an address and a value.
    #[error("`{0}` isn't a condition; expected ADDRESS=VALUE or ADDRESS=changed")]
    Syntax(String),
    /// The address or the value isn't a number.
    #[error(transparent)]
    Number(#[from] MemoryError),
}

impl Test {
    /// Returns whether `value` passes this test, where `start` is the value
    /// when the run started.
    #[must_use]
    pub const fn holds(self, start: u32, value: u32) -> bool {
        match self {
            Self::Equals(number) => value == number,
            Self::Changed => value != start,
        }
    }
}

impl FromStr for Condition {
    type Err = ConditionError;

    /// Parses `ADDRESS=VALUE`, such as `0x02016BA9=0`, or `ADDRESS=changed`,
    /// reading a u8 until the width is set.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (address, value) = text
            .split_once('=')
            .ok_or_else(|| ConditionError::Syntax(text.to_string()))?;
        Ok(Self {
            address: memory::parse_number(address)?,
            width: Width::U8,
            test: if value.eq_ignore_ascii_case("changed") {
                Test::Changed
            } else {
                Test::Equals(memory::parse_number(value)?)
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_conditions() {
        assert_eq!(
            "0x02016BA9=0".parse(),
            Ok(Condition {
                address: 0x0201_6BA9,
                width: Width::U8,
                test: Test::Equals(0),
            })
        );
        assert_eq!(
            "0x0200316C=Changed"
                .parse::<Condition>()
                .map(|found| found.test),
            Ok(Test::Changed)
        );
    }

    #[test]
    fn reject_invalid_conditions() {
        assert_eq!(
            "0x02016BA9".parse::<Condition>().unwrap_err().to_string(),
            "`0x02016BA9` isn't a condition; expected ADDRESS=VALUE or ADDRESS=changed"
        );
        assert!(matches!(
            "0x02016BA9=lots".parse::<Condition>(),
            Err(ConditionError::Number(MemoryError::InvalidNumber(_)))
        ));
        assert!(matches!(
            "here=0".parse::<Condition>(),
            Err(ConditionError::Number(MemoryError::InvalidNumber(_)))
        ));
    }

    #[test]
    fn test_values() {
        assert!(Test::Equals(0).holds(5, 0));
        assert!(!Test::Equals(0).holds(0, 1));
        assert!(Test::Changed.holds(280, 273));
        assert!(!Test::Changed.holds(280, 280));
    }
}
