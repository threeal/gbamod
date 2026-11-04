//! An example library for generating a Fibonacci sequence.
//!
//! This is the placeholder library of a project template; replace
//! [`fibonacci_sequence`] with your own logic.

/// Generates a Fibonacci sequence up to the given number of terms.
///
/// The sequence starts from `1, 1`, and exactly `n` terms are returned, so an
/// `n` of 0 yields an empty sequence.
#[must_use]
pub fn fibonacci_sequence(n: usize) -> Vec<usize> {
    let mut sequence = vec![1; n];
    for i in 2..n {
        sequence[i] = sequence[i - 2] + sequence[i - 1];
    }
    sequence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_fibonacci_sequence() {
        assert_eq!(fibonacci_sequence(0), vec![]);
        assert_eq!(fibonacci_sequence(1), vec![1]);
        assert_eq!(fibonacci_sequence(2), vec![1, 1]);
        assert_eq!(fibonacci_sequence(5), vec![1, 1, 2, 3, 5]);
    }
}
