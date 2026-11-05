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
        .subcommand(
            clap::Command::new("boot")
                .about("Boot the game folder's ROM headlessly and save a screenshot")
                .arg(
                    clap::arg!(<frames> "The number of frames to run")
                        .value_parser(clap::value_parser!(u32)),
                )
                .arg(
                    clap::arg!(<screenshot> "The PNG file to save the final screen to")
                        .value_parser(clap::value_parser!(std::path::PathBuf)),
                ),
        )
        .get_matches();

    let dir = std::path::Path::new(".");
    let result = match matches.subcommand() {
        Some(("info", _)) => gbamod::info(dir).map_err(|err| err.to_string()),
        Some(("boot", args)) => {
            let frames = args.get_one::<u32>("frames").copied().unwrap_or_default();
            let screenshot = args
                .get_one::<std::path::PathBuf>("screenshot")
                .cloned()
                .unwrap_or_default();
            gbamod::boot(dir, frames, &screenshot).map_err(|err| err.to_string())
        }
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
