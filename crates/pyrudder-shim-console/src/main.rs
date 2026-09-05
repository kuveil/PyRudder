//! `PyRudder` console-subsystem shim entry point.
//! `PyRudder` 控制台子系统代理入口。

fn main() {
    #[cfg(windows)]
    match pyrudder_platform_windows::router::run_shim(false) {
        Ok(exit) => std::process::exit(exit.for_process_exit()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(i32::from(error.exit_code()));
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("PyRudder requires Windows x64");
        std::process::exit(2);
    }
}
