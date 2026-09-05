//! `PyRudder` GUI-subsystem shim entry point.
//! `PyRudder` GUI 子系统代理入口。

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    match pyrudder_platform_windows::router::run_shim(true) {
        Ok(exit) => std::process::exit(exit.for_process_exit()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(i32::from(error.exit_code()));
        }
    }
    #[cfg(not(windows))]
    std::process::exit(2);
}
