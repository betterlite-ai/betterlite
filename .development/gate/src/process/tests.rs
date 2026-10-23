#![cfg(test)]

use std::ffi::OsStr;

use super::check_command;

#[test]
fn check_commands_drop_what_cargo_run_injected() {
    let command = check_command("bash");
    let removed: Vec<&OsStr> = command
        .get_envs()
        .filter(|&(_, value)| value.is_none())
        .map(|(name, _)| name)
        .collect();
    assert!(removed.contains(&OsStr::new("CARGO")));
    assert!(removed.contains(&OsStr::new("CARGO_PKG_NAME")));
    assert!(!removed.contains(&OsStr::new("PATH")));
}
