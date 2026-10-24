#![cfg(test)]

use std::process::Command;

#[test]
fn invalid_case_path_is_a_harness_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_betterlite-oracle"))
        .arg("tools/oracle/cases/does-not-exist.yaml")
        .output()
        .expect("oracle should start");
    assert_eq!(output.status.code(), Some(2_i32));
    let error = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(error.contains("oracle error"));
}
