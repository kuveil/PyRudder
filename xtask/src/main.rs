//! Repository automation entry point.
//! 仓库自动化入口。

#[cfg(windows)]
mod run;

const HELP: &str = "PyRudder development runner (not runtime registration)\n\nUsage:\n  cargo run -p xtask -- run <version> <absolute-root> <command> [--exact] [--scripts <absolute-dir>]... -- [args...]\n\nExecutes a selected x64 PE or CMD/BAT/PS1 script using an in-memory runtime record.\nScripts use fixed system hosts; PS1 uses Windows PowerShell 5.1 -NoProfile -File.\nCMD/BAT reject quotes, %, !, and control characters in paths/arguments.\nScript arguments must be Unicode. Execution policy is inherited, never bypassed.\nThe version is caller-supplied, not probed. Default scripts directory: <root>/Scripts.\nNo registration, installation, PATH persistence, or shim publication is performed.\nOnly execute programs, scripts, and runtime directories you trust.";

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => println!("{HELP}"),
        Some(first) if (first == "--help" || first == "-h") && arguments.next().is_none() => {
            println!("{HELP}");
        }
        #[cfg(windows)]
        Some(first) if first == "run" => match run::execute(arguments.collect()) {
            Ok(exit) => std::process::exit(exit.for_process_exit()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(i32::from(error.exit_code()));
            }
        },
        _ => {
            eprintln!("{HELP}");
            std::process::exit(i32::from(pyrudder_core::ErrorKind::Usage.exit_code()));
        }
    }
}
