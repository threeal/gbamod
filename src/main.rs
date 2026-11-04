//! A command-line interface for reverse engineering and patching GBA games.
//!
//! The CLI isn't implemented yet; it only prints the toolkit's status.

#[cfg(not(tarpaulin_include))]
fn main() {
    clap::Command::new("gbamod")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Reverse engineer and patch GBA games with an emulator-driven toolkit")
        .get_matches();

    println!("{}", gbamod::status());
}
