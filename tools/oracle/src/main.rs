#![cfg_attr(
    not(test),
    expect(
        clippy::restriction,
        reason = "The oracle CLI reports harness failures on stderr."
    )
)]

#[cfg(not(test))]
fn main() {
    let status = match betterlite_oracle::run() {
        Ok(code) => betterlite_oracle::exit_status(code),
        Err(error) => {
            eprintln!("oracle error: {error}");
            2
        }
    };
    std::process::exit(status);
}
