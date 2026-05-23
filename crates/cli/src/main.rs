#![forbid(unsafe_code)]

use std::{env, io, process::ExitCode};

fn main() -> ExitCode {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    match deepseek_coder_cli::run_cli(env::args(), &mut stdout, &mut stderr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
