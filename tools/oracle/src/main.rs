#![expect(
    clippy::restriction,
    reason = "The oracle CLI reports harness failures on stderr."
)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match betterlite_oracle::run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("oracle error: {error}");
            ExitCode::from(2)
        }
    }
}
