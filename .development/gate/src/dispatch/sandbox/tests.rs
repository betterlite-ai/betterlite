#![cfg(test)]

use std::env;
use std::fs;

use crate::dispatch::sandbox::write_settings;

#[test]
fn settings_sandbox_bash_to_the_worktree_and_the_writable_folders() {
    let path = env::temp_dir().join("betterlite-gate-sandbox-settings.json");
    write_settings(
        &path,
        "/w",
        &["/g".to_owned(), "/h/.cargo/registry".to_owned()],
    )
    .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let compact: String = text.split_whitespace().collect();
    assert_eq!(
        compact,
        r#"{"sandbox":{"enabled":true,"autoAllowBashIfSandboxed":true,"filesystem":{"allowWrite":["/w","/g","/h/.cargo/registry"]},"network":{"allowedDomains":["*"]}}}"#
    );
}
