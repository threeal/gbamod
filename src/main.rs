//! A command-line interface for generating a Fibonacci sequence.
//!
//! This is the placeholder CLI of a project template; remove it along with the
//! `clap` dependency if your project doesn't need one.

#[cfg(not(tarpaulin_include))]
fn main() {
    let matches = clap::Command::new("bonacci")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Generate a Fibonacci sequence up to the given number of terms")
        .arg(
            clap::arg!([n] "The number of terms")
                .value_parser(clap::value_parser!(usize))
                .required(true),
        )
        .get_matches();

    let n = matches.get_one::<usize>("n").unwrap_or(&0);
    let output = bonacci::fibonacci_sequence(*n)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");

    println!("{output}");
}
