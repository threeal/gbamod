//! A command-line interface for reverse engineering and patching GBA games.
//!
//! It runs from inside a game folder, which holds a `gbamod.toml` manifest.

#[cfg(not(tarpaulin_include))]
mod cli {
    use clap::{Arg, ArgMatches, Command, arg, value_parser};
    use gbamod::keys::Keys;
    use gbamod::session::{self, Request};
    use std::path::{Path, PathBuf};

    /// Defines the command line.
    pub fn command() -> Command {
        Command::new("gbamod")
            .version(env!("CARGO_PKG_VERSION"))
            .about("Reverse engineer and patch GBA games with an emulator-driven toolkit")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(Command::new("info").about("Print the header of the game folder's ROM"))
            .subcommand(
                Command::new("boot")
                    .about("Boot the game folder's ROM headlessly and save a screenshot")
                    .arg(frames_arg())
                    .arg(
                        arg!(<screenshot> "The PNG file to save the final screen to")
                            .value_parser(value_parser!(PathBuf)),
                    ),
            )
            .subcommand(
                Command::new("start").about("Start an emulator session for the game folder"),
            )
            .subcommand(Command::new("stop").about("Stop the game folder's emulator session"))
            .subcommand(
                Command::new("run")
                    .about("Run frames in the session with the held keys pressed")
                    .arg(frames_arg()),
            )
            .subcommand(
                Command::new("hold")
                    .about("Set the keys that stay pressed while frames run")
                    .arg(keys_arg()),
            )
            .subcommand(
                Command::new("tap")
                    .about("Press keys for some frames, then release them")
                    .arg(keys_arg())
                    .arg(
                        arg!([frames] "The number of frames to press them for")
                            .value_parser(value_parser!(u32))
                            .default_value("4"),
                    ),
            )
            .subcommand(
                Command::new("save")
                    .about("Write a savestate of the session")
                    .arg(file_arg("The savestate file to write")),
            )
            .subcommand(
                Command::new("load")
                    .about("Restore a savestate into the session")
                    .arg(file_arg("The savestate file to read")),
            )
            .subcommand(
                Command::new("shot")
                    .about("Save the session's screen as a PNG")
                    .arg(file_arg("The PNG file to write")),
            )
            .subcommand(Command::new(gbamod::SERVE_COMMAND).hide(true))
    }

    /// Runs the parsed command in the game folder at `dir`, returning its
    /// output or an error message.
    pub fn run(matches: &ArgMatches, dir: &Path) -> Result<String, String> {
        let request = |request: Request| {
            session::send(&session::socket_path(dir), &request).map_err(|err| err.to_string())
        };
        match matches.subcommand() {
            Some(("info", _)) => gbamod::info(dir).map_err(|err| err.to_string()),
            Some(("boot", args)) => {
                let screenshot = args
                    .get_one::<PathBuf>("screenshot")
                    .cloned()
                    .unwrap_or_default();
                gbamod::boot(dir, frames(args), &screenshot).map_err(|err| err.to_string())
            }
            Some(("start", _)) => gbamod::start(dir).map_err(|err| err.to_string()),
            Some((gbamod::SERVE_COMMAND, _)) => gbamod::serve(dir)
                .map(|()| String::new())
                .map_err(|err| err.to_string()),
            Some(("stop", _)) => request(Request::Stop),
            Some(("run", args)) => request(Request::Run {
                frames: frames(args),
            }),
            Some(("hold", args)) => request(Request::Hold { keys: keys(args) }),
            Some(("tap", args)) => request(Request::Tap {
                keys: keys(args),
                frames: frames(args),
            }),
            Some(("save", args)) => request(Request::Save { path: file(args) }),
            Some(("load", args)) => request(Request::Load { path: file(args) }),
            Some(("shot", args)) => request(Request::Shot { path: file(args) }),
            _ => unreachable!("clap requires a known subcommand"),
        }
    }

    fn frames_arg() -> Arg {
        arg!(<frames> "The number of frames to run").value_parser(value_parser!(u32))
    }

    fn keys_arg() -> Arg {
        arg!(<keys> "The keys joined by `+`, such as `A+RIGHT`, or `none`")
            .value_parser(value_parser!(Keys))
    }

    fn file_arg(help: &'static str) -> Arg {
        Arg::new("file")
            .required(true)
            .help(help)
            .value_parser(value_parser!(PathBuf))
    }

    fn frames(args: &ArgMatches) -> u32 {
        args.get_one::<u32>("frames").copied().unwrap_or_default()
    }

    fn keys(args: &ArgMatches) -> Keys {
        args.get_one::<Keys>("keys").copied().unwrap_or_default()
    }

    /// Returns the file argument as an absolute path, since the session runs
    /// in its own working directory.
    fn file(args: &ArgMatches) -> PathBuf {
        let file = args.get_one::<PathBuf>("file").cloned().unwrap_or_default();
        std::path::absolute(&file).unwrap_or(file)
    }
}

#[cfg(not(tarpaulin_include))]
fn main() -> std::process::ExitCode {
    let matches = cli::command().get_matches();
    match cli::run(&matches, std::path::Path::new(".")) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
