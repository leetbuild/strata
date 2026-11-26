//! `strata-headless`: the CLI without the window (development target).

fn main() -> std::process::ExitCode {
    let code = strata_cli::run(std::env::args_os().collect(), None);
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}
