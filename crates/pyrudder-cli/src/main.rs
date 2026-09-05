//! `PyRudder` management command-line entry point.
//! `PyRudder` 管理命令行入口。

#[cfg(windows)]
mod app;
mod arguments;
#[cfg(any(windows, test))]
mod output;
#[cfg(windows)]
mod prompts;

use arguments::Arguments;
use clap::{CommandFactory, FromArgMatches};
const BANNER: &str = include_str!("../../../assets/banner.txt");

fn main() {
    let raw: Vec<_> = std::env::args_os().collect();
    if raw
        .get(1)
        .is_some_and(|argument| argument == "--version" || argument == "-V")
        && raw.len() != 2
    {
        eprintln!("Usage: pyrudder --version");
        std::process::exit(2);
    }
    let json_errors = raw
        .iter()
        .skip(1)
        .take_while(|argument| *argument != "--")
        .any(|argument| argument == "--json");
    let no_color = raw
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| argument == "--no-color");
    let parser = Arguments::command().color(if no_color {
        clap::ColorChoice::Never
    } else {
        clap::ColorChoice::Auto
    });
    let parsed = parser
        .try_get_matches_from(raw)
        .and_then(|matches| Arguments::from_arg_matches(&matches));
    let arguments = match parsed {
        Ok(arguments) => arguments,
        Err(error) => {
            if json_errors && error.exit_code() != 0 {
                eprintln!(
                    "{}",
                    serde_json::json!({"schema_version": 1, "ok": false, "error": {"code": "usage", "message": error.to_string()}})
                );
            } else if let Err(print_error) = error.print() {
                eprintln!("Cannot print argument diagnostic: {print_error}");
            }
            std::process::exit(error.exit_code());
        }
    };
    if arguments.command.is_none() {
        if arguments.json {
            println!(
                "{}",
                serde_json::json!({"schema_version": 1, "ok": true, "data": {"name": "PyRudder", "version": env!("CARGO_PKG_VERSION"), "hint": "Use --help for commands"}})
            );
            return;
        }
        if arguments.quiet {
            return;
        }
        println!("{BANNER}");
        if let Err(error) = Arguments::command().print_help() {
            eprintln!("Cannot print help: {error}");
        }
        println!();
        return;
    }
    #[cfg(windows)]
    run(&arguments);
    #[cfg(not(windows))]
    {
        eprintln!("PyRudder requires Windows x64");
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn run(arguments: &Arguments) {
    if arguments.verbose {
        eprintln!(
            "PyRudder {} — explicit user configuration; no automatic runtime installation during dispatch. / 显式用户配置，命令转发不自动安装运行时。",
            env!("CARGO_PKG_VERSION")
        );
    }
    let result = app::App::load(arguments).and_then(|application| {
        application.execute(
            arguments
                .command
                .as_ref()
                .ok_or_else(|| app::usage("Missing command"))?,
        )
    });
    match result {
        Ok(app::Outcome::Child(code)) => std::process::exit(code),
        Ok(app::Outcome::Text(text)) => println!("{text}"),
        Ok(app::Outcome::Data(data))
            if !arguments.quiet
                && !arguments.json
                && matches!(arguments.command.as_ref(), Some(arguments::Action::List)) =>
        {
            println!("{}", output::runtime_list(&data));
        }
        Ok(app::Outcome::Data(data)) if !arguments.quiet => {
            if !arguments.json
                && matches!(
                    arguments.command.as_ref(),
                    Some(arguments::Action::Available { .. })
                )
            {
                println!("{}", output::release_list(&data));
                return;
            }
            let output = if arguments.json {
                serde_json::to_string(
                    &serde_json::json!({"schema_version": 1, "ok": true, "data": data}),
                )
            } else {
                serde_json::to_string_pretty(&data)
            };
            if let Ok(output) = output {
                println!("{output}");
            } else {
                eprintln!("Cannot encode command output");
                std::process::exit(70);
            }
        }
        Ok(app::Outcome::Data(_)) => {}
        Err(error) => {
            if arguments.json {
                eprintln!(
                    "{}",
                    serde_json::json!({"schema_version": 1, "ok": false, "error": {"code": error.kind().code(), "message": error.message(), "hint": error.hint()}})
                );
            } else {
                eprintln!("{error}");
            }
            std::process::exit(i32::from(error.exit_code()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BANNER;

    #[test]
    fn banner_is_ascii() {
        assert!(BANNER.is_ascii());
    }

    #[test]
    fn banner_has_no_trailing_whitespace() {
        for (line_number, line) in BANNER.lines().enumerate() {
            assert!(
                !line.ends_with(' ') && !line.ends_with('\t'),
                "banner line {} has trailing whitespace",
                line_number + 1
            );
        }
    }

    #[test]
    fn banner_lines_fit_standard_terminal() {
        for (line_number, line) in BANNER.lines().enumerate() {
            assert!(
                line.len() <= 80,
                "banner line {} is {} columns wide",
                line_number + 1,
                line.len()
            );
        }
    }
}
