//! A command-line interface for reverse engineering and patching GBA games.
//!
//! It runs from inside a game folder, which holds a `gbamod.toml` manifest.

#[cfg(not(tarpaulin_include))]
mod cli {
    use clap::{Arg, ArgMatches, Command, arg, value_parser};
    use gbamod::asm;
    use gbamod::debug::{self, WatchKind};
    use gbamod::disasm;
    use gbamod::keys::Keys;
    use gbamod::memory::{self, Width};
    use gbamod::pixels::{self, Area, Token};
    use gbamod::search::{Query, Rule};
    use gbamod::session::{self, Dis, Poke, Request};
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
                Command::new("start")
                    .about("Start an emulator session for the game folder")
                    .arg(patched_arg()),
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
            .subcommand(
                Command::new("peek")
                    .about("Print bytes from the session's memory as hex")
                    .arg(number_arg("address", "The address of the first byte"))
                    .arg(
                        Arg::new("count")
                            .help("The number of bytes to print [default: 16]")
                            .value_parser(memory::parse_number),
                    ),
            )
            .subcommand(poke_command("poke8", "byte"))
            .subcommand(poke_command("poke16", "16-bit halfword"))
            .subcommand(poke_command("poke32", "32-bit word"))
            .subcommand(
                Command::new("dump")
                    .about("Write the session's working RAM to a file, EWRAM then IWRAM")
                    .arg(file_arg("The dump file to write")),
            )
            .subcommand(search_command())
            .subcommand(pixels_command())
            .subcommands(debugger_commands())
            .subcommand(
                Command::new("asm")
                    .about("Assemble a file of Thumb code placed at an address, and list it")
                    .arg(
                        arg!(<file> "The assembly file to read")
                            .value_parser(value_parser!(PathBuf)),
                    )
                    .arg(
                        arg!(--at <address> "The address the code starts at")
                            .required(true)
                            .value_parser(memory::parse_number),
                    ),
            )
            .subcommand(Command::new("build").about(
                "Apply the manifest's patches and hooks to the ROM and write the patched ROM and IPS patch",
            ))
            .subcommand(
                Command::new(gbamod::SERVE_COMMAND)
                    .hide(true)
                    .arg(patched_arg()),
            )
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
            Some(("start", args)) => gbamod::start(dir, args.get_flag(gbamod::PATCHED_FLAG))
                .map_err(|err| err.to_string()),
            Some((gbamod::SERVE_COMMAND, args)) => {
                gbamod::serve(dir, args.get_flag(gbamod::PATCHED_FLAG))
                    .map(|()| String::new())
                    .map_err(|err| err.to_string())
            }
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
            Some(("peek", args)) => request(Request::Peek {
                address: number(args, "address"),
                count: args
                    .get_one::<u32>("count")
                    .copied()
                    .unwrap_or(memory::DEFAULT_PEEK_COUNT),
            }),
            Some(("poke8", args)) => poke(args, Width::U8).and_then(request),
            Some(("poke16", args)) => poke(args, Width::U16).and_then(request),
            Some(("poke32", args)) => poke(args, Width::U32).and_then(request),
            Some(("dump", args)) => request(Request::Dump { path: file(args) }),
            Some(("dis", args)) => request(Request::Dis(Dis {
                address: number(args, "address"),
                count: args
                    .get_one::<u32>("count")
                    .copied()
                    .unwrap_or(disasm::DEFAULT_COUNT),
                arm: args.get_flag("arm"),
            })),
            Some(("watch", args)) => request(Request::Watch {
                address: number(args, "address"),
                on: args
                    .get_one::<WatchKind>("on")
                    .copied()
                    .unwrap_or(WatchKind::Write),
            }),
            Some(("break", args)) => request(Request::Break {
                address: number(args, "address"),
                stack: args.get_one::<u32>("stack").copied().unwrap_or_default(),
            }),
            Some(("points", _)) => request(Request::Points),
            Some(("clear", _)) => request(Request::Clear),
            Some(("regs", _)) => request(Request::Regs),
            Some(("asm", args)) => {
                let file = args.get_one::<PathBuf>("file").cloned().unwrap_or_default();
                asm::run(&file, number(args, "at")).map_err(|err| err.to_string())
            }
            Some(("build", _)) => gbamod::build::run(dir).map_err(|err| err.to_string()),
            Some(("search", args)) => search(args),
            Some(("pixels", args)) => {
                let position = |name| args.get_one::<usize>(name).copied().unwrap_or_default();
                let area = Area {
                    x: position("x"),
                    y: position("y"),
                    width: position("width"),
                    height: position("height"),
                };
                let file = args.get_one::<PathBuf>("file").cloned().unwrap_or_default();
                pixels::run(&file, area, args.get_one::<Token>("count").copied())
                    .map_err(|err| err.to_string())
            }
            _ => unreachable!("clap requires a known subcommand"),
        }
    }

    fn search_command() -> Command {
        Command::new("search")
            .about("List the addresses whose values changed across RAM dumps by a rule")
            .arg(
                arg!(<width> "The width of the values: u8, u16, or u32")
                    .value_parser(value_parser!(Width)),
            )
            .arg(
                arg!(<rule> "How the values change across the dumps: dec, inc, same, changed, eq=N, or seq=A,B,... with * for any")
                    .value_parser(value_parser!(Rule)),
            )
            .arg(
                arg!(<dumps> ... "The RAM dumps, in the order they were taken")
                    .value_parser(value_parser!(PathBuf)),
            )
            .arg(
                arg!(--min <value> "The smallest value allowed in every dump")
                    .value_parser(memory::parse_number),
            )
            .arg(
                arg!(--max <value> "The largest value allowed in every dump")
                    .value_parser(memory::parse_number),
            )
    }

    fn pixels_command() -> Command {
        Command::new("pixels")
            .about("Print a box of pixels from a screenshot as color tokens such as 0ce")
            .arg(arg!(<file> "The PNG screenshot to read").value_parser(value_parser!(PathBuf)))
            .arg(arg!(<x> "The column of the box's left edge").value_parser(value_parser!(usize)))
            .arg(arg!(<y> "The row of the box's top edge").value_parser(value_parser!(usize)))
            .arg(
                arg!([width] "The number of columns in the box")
                    .value_parser(value_parser!(usize))
                    .default_value("1"),
            )
            .arg(
                arg!([height] "The number of rows in the box")
                    .value_parser(value_parser!(usize))
                    .default_value("1"),
            )
            .arg(
                arg!(--count <token> "Count the pixels of this color token in each row instead")
                    .value_parser(value_parser!(Token)),
            )
    }

    fn debugger_commands() -> [Command; 6] {
        [
            Command::new("dis")
                .about("Disassemble code from the session's memory, as Thumb unless --arm is given")
                .arg(number_arg(
                    "address",
                    "The address of the first instruction; a Thumb function pointer's low bit is ignored",
                ))
                .arg(
                    Arg::new("count")
                        .help("The number of instructions to print [default: 16]")
                        .value_parser(memory::parse_number),
                )
                .arg(arg!(--arm "Read the code as ARM instead of Thumb")),
            Command::new("watch")
                .about("Add a watchpoint; later commands that run frames list its hits")
                .arg(number_arg("address", "The address to watch"))
                .arg(
                    arg!(--on <kind> "The accesses that fire it: write, read, access, or change")
                        .value_parser(value_parser!(WatchKind))
                        .default_value("write"),
                ),
            Command::new("break")
                .about("Add a breakpoint; later commands that run frames list its hits")
                .arg(number_arg(
                    "address",
                    "The address of the instruction to break on",
                ))
                .arg(
                    arg!(--stack <words> "The number of stack words to show for each hit")
                        .value_parser(
                            value_parser!(u32).range(0..=i64::from(debug::MAX_STACK_WORDS)),
                        )
                        .default_value("0"),
                ),
            Command::new("points").about("List the session's watchpoints and breakpoints"),
            Command::new("clear").about("Remove every watchpoint and breakpoint"),
            Command::new("regs").about("Print the CPU registers as they stand between frames"),
        ]
    }

    fn number_arg(name: &'static str, help: &'static str) -> Arg {
        Arg::new(name)
            .required(true)
            .help(help)
            .value_parser(memory::parse_number)
    }

    fn poke_command(name: &'static str, value: &str) -> Command {
        Command::new(name)
            .about(format!("Write a {value} to the session's memory"))
            .arg(number_arg("address", "The address to write to"))
            .arg(number_arg("value", "The value to write"))
    }

    fn number(args: &ArgMatches, name: &str) -> u32 {
        args.get_one::<u32>(name).copied().unwrap_or_default()
    }

    /// Builds a poke request, rejecting a value too wide for `width` before
    /// it reaches the session.
    fn poke(args: &ArgMatches, width: Width) -> Result<Request, String> {
        let value = width
            .check(number(args, "value"))
            .map_err(|err| err.to_string())?;
        Ok(Request::Poke(Poke {
            address: number(args, "address"),
            width,
            value,
        }))
    }

    /// Searches the dumps named on the command line.
    fn search(args: &ArgMatches) -> Result<String, String> {
        let query = Query {
            width: args.get_one::<Width>("width").copied().unwrap_or(Width::U8),
            rule: args.get_one::<Rule>("rule").cloned().unwrap_or(Rule::Same),
            min: args.get_one::<u32>("min").copied().unwrap_or(0),
            max: args.get_one::<u32>("max").copied().unwrap_or(u32::MAX),
        };
        let dumps: Vec<PathBuf> = args
            .get_many::<PathBuf>("dumps")
            .unwrap_or_default()
            .cloned()
            .collect();
        query.run(&dumps).map_err(|err| err.to_string())
    }

    fn patched_arg() -> Arg {
        Arg::new(gbamod::PATCHED_FLAG)
            .long(gbamod::PATCHED_FLAG)
            .action(clap::ArgAction::SetTrue)
            .help("Run the ROM with the manifest's patches and hooks applied in memory")
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
