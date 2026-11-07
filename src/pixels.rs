//! Reading pixels from screenshots, as compact color tokens.

use crate::screen::{self, Screen, ScreenError};
use std::fmt;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// A color reduced to the top 4 bits of its red, green, and blue values,
/// written as three hex digits such as `0ce`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token([u8; 3]);

/// A box of pixels on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    /// The column of the box's left edge.
    pub x: usize,
    /// The row of the box's top edge.
    pub y: usize,
    /// The number of columns in the box.
    pub width: usize,
    /// The number of rows in the box.
    pub height: usize,
}

/// The reason pixels couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum PixelsError {
    /// The text isn't a color token.
    #[error("`{0}` isn't a color token; expected three hex digits such as 0ce")]
    InvalidToken(String),
    /// The box is empty or reaches outside the screen.
    #[error(
        "a {}x{} box at ({}, {}) doesn't fit on the 240x160 screen",
        area.width, area.height, area.x, area.y
    )]
    OutsideScreen {
        /// The box.
        area: Area,
    },
    /// The screenshot couldn't be read.
    #[error("failed to read `{}`: {source}", path.display())]
    Read {
        /// The screenshot file.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The screenshot isn't a usable image.
    #[error("`{}` isn't a screenshot: {source}", path.display())]
    Decode {
        /// The screenshot file.
        path: PathBuf,
        /// Why it couldn't be decoded.
        source: ScreenError,
    },
}

impl Token {
    /// Returns the token of a color given as red, green, and blue values.
    #[must_use]
    pub const fn of(rgb: [u8; 3]) -> Self {
        Self([rgb[0] >> 4, rgb[1] >> 4, rgb[2] >> 4])
    }
}

impl FromStr for Token {
    type Err = PixelsError;

    /// Parses three hex digits, ignoring case.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || PixelsError::InvalidToken(text.to_string());
        let digits: Vec<u8> = text
            .chars()
            .map(|digit| {
                digit
                    .to_digit(16)
                    .and_then(|value| u8::try_from(value).ok())
            })
            .collect::<Option<_>>()
            .ok_or_else(invalid)?;
        let digits: [u8; 3] = digits.try_into().map_err(|_| invalid())?;
        Ok(Self(digits))
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [red, green, blue] = self.0;
        write!(f, "{red:x}{green:x}{blue:x}")
    }
}

impl Area {
    /// Returns the area if it holds at least one pixel and fits on the
    /// screen.
    ///
    /// # Errors
    ///
    /// Returns [`PixelsError::OutsideScreen`] otherwise.
    pub fn check(self) -> Result<Self, PixelsError> {
        let fits = |start: usize, size: usize, limit: usize| {
            size > 0 && start.checked_add(size).is_some_and(|end| end <= limit)
        };
        if fits(self.x, self.width, screen::WIDTH) && fits(self.y, self.height, screen::HEIGHT) {
            Ok(self)
        } else {
            Err(PixelsError::OutsideScreen { area: self })
        }
    }

    /// Returns the tokens of each row of the area, with the row's number.
    fn rows(self, screen: &Screen) -> impl Iterator<Item = (usize, Vec<Token>)> {
        (self.y..self.y + self.height).map(move |y| {
            let tokens = (self.x..self.x + self.width)
                .map(|x| Token::of(screen.pixel(x, y)))
                .collect();
            (y, tokens)
        })
    }
}

/// Lists each row of the area as its number followed by its pixels' tokens.
#[must_use]
pub fn show(screen: &Screen, area: Area) -> String {
    let lines: Vec<String> = area
        .rows(screen)
        .map(|(y, tokens)| {
            let mut line = format!("{y:3}:");
            for token in tokens {
                let _ = write!(line, " {token}");
            }
            line
        })
        .collect();
    lines.join("\n")
}

/// Lists each row of the area as its number followed by how many of its
/// pixels have the token.
#[must_use]
pub fn count(screen: &Screen, area: Area, token: Token) -> String {
    let lines: Vec<String> = area
        .rows(screen)
        .map(|(y, tokens)| {
            let matching = tokens.iter().filter(|&&pixel| pixel == token).count();
            format!("{y:3}: {matching} of {}", area.width)
        })
        .collect();
    lines.join("\n")
}

/// Reads the area of the screenshot at `path`, listing its pixels, or how
/// many of them in each row have the token if one is given, for the `pixels`
/// command.
///
/// # Errors
///
/// Returns a [`PixelsError`] if the area doesn't fit on the screen, or the
/// screenshot can't be read or decoded.
pub fn run(path: &Path, area: Area, token: Option<Token>) -> Result<String, PixelsError> {
    let area = area.check()?;
    let png = std::fs::read(path).map_err(|source| PixelsError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let screen = Screen::decode_png(&png).map_err(|source| PixelsError::Decode {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(token.map_or_else(|| show(&screen, area), |token| count(&screen, area, token)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Builds a screen of black pixels with a bar of `blue` pixels in row 145
    /// from column 16, followed by `red` pixels.
    fn bar_screen(blue: usize, red: usize) -> Screen {
        let mut buffer = vec![0_u32; screen::WIDTH * screen::HEIGHT];
        let row = 145 * screen::WIDTH + 16;
        buffer[row..row + blue].fill(u32::from_le_bytes([0x08, 0xC8, 0xE8, 0]));
        buffer[row + blue..row + blue + red].fill(u32::from_le_bytes([0xF8, 0x08, 0x00, 0]));
        Screen::from_frame_buffer(&buffer, screen::WIDTH)
    }

    const fn area(x: usize, y: usize, width: usize, height: usize) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn make_tokens() {
        assert_eq!(Token::of([0x08, 0xC8, 0xE8]).to_string(), "0ce");
        assert_eq!(Token::of([0xFF, 0x10, 0x0F]).to_string(), "f10");
    }

    #[test]
    fn parse_tokens() {
        assert_eq!("0ce".parse::<Token>().unwrap(), Token([0, 12, 14]));
        assert_eq!("F00".parse::<Token>().unwrap(), Token([15, 0, 0]));
        for text in ["", "0c", "0cef", "0cg", "+1f"] {
            assert_eq!(
                text.parse::<Token>().unwrap_err().to_string(),
                format!("`{text}` isn't a color token; expected three hex digits such as 0ce"),
            );
        }
    }

    #[test]
    fn check_areas() {
        assert!(area(0, 0, 240, 160).check().is_ok());
        assert!(area(239, 159, 1, 1).check().is_ok());
        for outside in [
            area(0, 0, 0, 1),
            area(0, 0, 1, 0),
            area(239, 0, 2, 1),
            area(0, 159, 1, 2),
            area(usize::MAX, 0, 1, 1),
        ] {
            assert!(matches!(
                outside.check(),
                Err(PixelsError::OutsideScreen { area }) if area == outside
            ));
        }
        assert_eq!(
            area(230, 150, 20, 5).check().unwrap_err().to_string(),
            "a 20x5 box at (230, 150) doesn't fit on the 240x160 screen"
        );
    }

    #[test]
    fn show_pixels() {
        let screen = bar_screen(2, 1);
        assert_eq!(
            show(&screen, area(15, 144, 5, 2)),
            "144: 000 000 000 000 000\n145: 000 0ce 0ce f00 000"
        );
        assert_eq!(show(&screen, area(16, 145, 1, 1)), "145: 0ce");
    }

    #[test]
    fn count_pixels() {
        let screen = bar_screen(46, 2);
        let area = area(16, 145, 48, 2);
        assert_eq!(
            count(&screen, area, "0ce".parse().unwrap()),
            "145: 46 of 48\n146: 0 of 48"
        );
        assert_eq!(
            count(&screen, area, "f00".parse().unwrap()),
            "145: 2 of 48\n146: 0 of 48"
        );
    }

    #[test]
    fn run_on_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        fs::write(&path, bar_screen(12, 36).encode_png()).unwrap();

        assert_eq!(
            run(&path, area(16, 145, 48, 1), Some("f00".parse().unwrap())).unwrap(),
            "145: 36 of 48"
        );
        assert_eq!(
            run(&path, area(27, 145, 2, 1), None).unwrap(),
            "145: 0ce f00"
        );
    }

    #[test]
    fn reject_area_outside_screen() {
        let err = run(Path::new("shot.png"), area(0, 160, 1, 1), None).unwrap_err();
        assert!(matches!(err, PixelsError::OutsideScreen { .. }));
    }

    #[test]
    fn reject_missing_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.png");
        let err = run(&path, area(0, 0, 1, 1), None).unwrap_err();
        assert!(matches!(err, PixelsError::Read { .. }));
        assert!(
            err.to_string()
                .starts_with(&format!("failed to read `{}`: ", path.display()))
        );
    }

    #[test]
    fn reject_file_that_isnt_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.png");
        fs::write(&path, "not a png").unwrap();
        let err = run(&path, area(0, 0, 1, 1), None).unwrap_err();
        assert!(matches!(err, PixelsError::Decode { .. }));
        assert!(err.to_string().starts_with(&format!(
            "`{}` isn't a screenshot: not a valid PNG image: ",
            path.display()
        )));
    }
}
