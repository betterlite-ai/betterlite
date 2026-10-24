use std::io::{self, Write as _};
use std::process::ExitCode;

fn main() -> ExitCode {
    match betterlite_oracle::run() {
        Ok(code) => code,
        Err(error) => {
            let _written = writeln!(io::stderr().lock(), "oracle error: {error}");
            ExitCode::from(2)
        }
    }
}
