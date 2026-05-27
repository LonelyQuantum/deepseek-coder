#![forbid(unsafe_code)]

use std::{
    env,
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    match prole_coder_cli::run_cli_with_input(env::args(), &mut stdin, &mut stdout, &mut stderr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if !error.is_reported() {
                let _ = writeln!(stderr, "{error}");
            }
            ExitCode::FAILURE
        }
    }
}
