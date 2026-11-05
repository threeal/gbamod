//! The GBA's buttons, and the `A+RIGHT` notation for naming a set of them.

use std::fmt;
use std::str::FromStr;

/// The names of the GBA's buttons, in the order of their bits in the key
/// register.
const NAMES: [&str; 10] = [
    "A", "B", "SELECT", "START", "RIGHT", "LEFT", "UP", "DOWN", "R", "L",
];

/// A set of GBA buttons, as a bit mask in the layout of the key register.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Keys(u16);

/// The reason text doesn't name a set of buttons.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeysError {
    /// A name between `+` signs is empty.
    #[error("empty key name in `{0}`")]
    Empty(String),
    /// A name isn't one of the GBA's buttons.
    #[error(
        "unknown key `{0}`; expected one of A, B, SELECT, START, RIGHT, LEFT, UP, DOWN, R, L, or none"
    )]
    Unknown(String),
}

impl Keys {
    /// No buttons.
    pub const NONE: Self = Self(0);

    /// Returns the bit mask in the layout of the key register.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl FromStr for Keys {
    type Err = KeysError;

    /// Parses button names joined by `+`, such as `A+RIGHT`, ignoring case,
    /// or `none` for no buttons.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.eq_ignore_ascii_case("none") {
            return Ok(Self::NONE);
        }
        let mut bits = 0;
        for name in text.split('+') {
            if name.is_empty() {
                return Err(KeysError::Empty(text.to_string()));
            }
            let bit = NAMES
                .iter()
                .position(|known| known.eq_ignore_ascii_case(name))
                .ok_or_else(|| KeysError::Unknown(name.to_string()))?;
            bits |= 1 << bit;
        }
        Ok(Self(bits))
    }
}

impl fmt::Display for Keys {
    /// Writes the button names joined by `+` in key register order, or `none`
    /// for no buttons.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("none");
        }
        let names: Vec<&str> = NAMES
            .iter()
            .enumerate()
            .filter(|(bit, _)| self.0 & (1 << bit) != 0)
            .map(|(_, name)| *name)
            .collect();
        f.write_str(&names.join("+"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keys() {
        assert_eq!("A".parse(), Ok(Keys(0b1)));
        assert_eq!("a+right".parse(), Ok(Keys(0b1_0001)));
        assert_eq!("L+R+DOWN".parse(), Ok(Keys(0b11_1000_0000)));
        assert_eq!("NONE".parse(), Ok(Keys::NONE));
    }

    #[test]
    fn reject_empty_key_name() {
        let err = "A++B".parse::<Keys>().unwrap_err();
        assert_eq!(err, KeysError::Empty("A++B".into()));
        assert_eq!(err.to_string(), "empty key name in `A++B`");
    }

    #[test]
    fn reject_unknown_key() {
        let err = "A+X".parse::<Keys>().unwrap_err();
        assert_eq!(err, KeysError::Unknown("X".into()));
        assert!(
            err.to_string()
                .starts_with("unknown key `X`; expected one of A, B,")
        );
    }

    #[test]
    fn display_keys() {
        assert_eq!(Keys::NONE.to_string(), "none");
        assert_eq!(Keys(0b1_0001).to_string(), "A+RIGHT");
        assert_eq!("l+a".parse::<Keys>().unwrap().to_string(), "A+L");
    }

    #[test]
    fn expose_bits() {
        assert_eq!(Keys(0b1000).bits(), 8);
    }
}
