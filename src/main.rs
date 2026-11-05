//! A command-line interface for reverse engineering and patching GBA games.
//!
//! It runs from inside a game folder, which holds a `gbamod.toml` manifest.

#[cfg(not(tarpaulin_include))]
fn main() -> std::process::ExitCode {
    let matches = clap::Command::new("gbamod")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Reverse engineer and patch GBA games with an emulator-driven toolkit")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(clap::Command::new("info").about("Print the header of the game folder's ROM"))
        .get_matches();

    let result = match matches.subcommand() {
        Some(("info", _)) => gbamod::info(std::path::Path::new(".")),
        _ => unreachable!("clap requires a known subcommand"),
    };

    match result {
        Ok(output) => {
            println!("{output}");
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
