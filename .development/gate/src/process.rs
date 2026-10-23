mod tests;

use std::env;
use std::process::Command;

const INJECTED_BY_CARGO_RUN: [&str; 6] = [
    "CARGO",
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "RUSTUP_TOOLCHAIN",
    "RUSTUP_TOOLCHAIN_SOURCE",
    "RUST_RECURSION_COUNT",
];

pub(crate) fn check_command(program: &str) -> Command {
    let mut command = Command::new(program);
    for (name, _) in env::vars_os() {
        let injected = name.to_str().is_some_and(|text| {
            INJECTED_BY_CARGO_RUN.contains(&text) || text.starts_with("CARGO_PKG_")
        });
        if injected {
            command.env_remove(name);
        }
    }
    command
}
