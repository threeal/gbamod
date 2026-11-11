//! The GBA's buttons, the `A+RIGHT` notation for naming a set of them, and
//! the `B:4,none:4` notation for pressing sets in turn.

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
    /// A step of a pattern isn't keys and a frame count joined by `:`.
    #[error("`{0}` isn't a pattern step; expected KEYS:FRAMES, such as B:4")]
    Step(String),
    /// A step of a pattern lasts no frames.
    #[error("the pattern step `{0}` lasts no frames")]
    NoFrames(String),
}

/// Sets of keys pressed in turn while frames run, each for some frames, over
/// and over.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Pattern(Vec<(Keys, u32)>);

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

impl Pattern {
    /// Returns the keys pressed on frame `frame` of a run, counted from 0.
    #[must_use]
    pub fn keys_at(&self, frame: u32) -> Keys {
        let period: u32 = self.0.iter().map(|(_, frames)| frames).sum();
        let mut left = frame % period.max(1);
        for &(keys, frames) in &self.0 {
            if left < frames {
                return keys;
            }
            left -= frames;
        }
        Keys::NONE
    }
}

impl FromStr for Pattern {
    type Err = KeysError;

    /// Parses steps of keys and a frame count joined by `:`, separated by
    /// commas, such as `B:4,none:4`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.split(',')
            .map(|step| {
                let (keys, frames) = step
                    .split_once(':')
                    .ok_or_else(|| KeysError::Step(step.to_string()))?;
                let frames: u32 = frames
                    .parse()
                    .map_err(|_| KeysError::Step(step.to_string()))?;
                if frames == 0 {
                    return Err(KeysError::NoFrames(step.to_string()));
                }
                Ok((keys.parse()?, frames))
            })
            .collect::<Result<_, _>>()
            .map(Self)
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
    fn parse_patterns() {
        assert_eq!(
            "B:4,none:2".parse(),
            Ok(Pattern(vec![(Keys(0b10), 4), (Keys::NONE, 2)]))
        );
    }

    #[test]
    fn reject_invalid_patterns() {
        assert_eq!(
            "B".parse::<Pattern>().unwrap_err().to_string(),
            "`B` isn't a pattern step; expected KEYS:FRAMES, such as B:4"
        );
        assert_eq!(
            "B:many".parse::<Pattern>(),
            Err(KeysError::Step("B:many".into()))
        );
        assert_eq!(
            "B:4,none:0".parse::<Pattern>().unwrap_err().to_string(),
            "the pattern step `none:0` lasts no frames"
        );
        assert_eq!(
            "X:4".parse::<Pattern>(),
            Err(KeysError::Unknown("X".into()))
        );
    }

    #[test]
    fn press_patterns_in_turn() {
        let pattern: Pattern = "B:2,UP:1".parse().unwrap();
        let pressed: Vec<String> = (0..7)
            .map(|frame| pattern.keys_at(frame).to_string())
            .collect();
        assert_eq!(pressed, ["B", "B", "UP", "B", "B", "UP", "B"]);
        assert_eq!(Pattern(Vec::new()).keys_at(3), Keys::NONE);
    }

    #[test]
    fn expose_bits() {
        assert_eq!(Keys(0b1000).bits(), 8);
    }
}
