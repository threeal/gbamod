//! An emulator-driven toolkit for reverse engineering and patching GBA games.
//!
//! The toolkit isn't implemented yet; [`status`] only stands in for it.

/// Returns a notice that the toolkit isn't implemented yet.
#[must_use]
pub const fn status() -> &'static str {
    "gbamod is not implemented yet"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_status() {
        assert_eq!(status(), "gbamod is not implemented yet");
    }
}
