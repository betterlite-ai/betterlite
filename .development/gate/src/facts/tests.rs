#![cfg(test)]

use std::fs;

use super::Facts;
use crate::git::git;

#[test]
fn oracle_helpers_and_sdk_http_lines_are_litellm_callers_and_other_python_is_not() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    git(root, &["init", "--quiet", "-b", "main"]).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    )
    .unwrap();
    for path in ["tools/oracle/helpers", "tools/oracle/sdk_http", "scripts"] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    fs::write(
        root.join("tools/oracle/helpers/b1.py"),
        "helper = cast(A, b)\n",
    )
    .unwrap();
    fs::write(
        root.join("tools/oracle/sdk_http/b2.py"),
        "caller = cast(A, b)\n",
    )
    .unwrap();
    fs::write(root.join("scripts/b3.py"), "other = cast(A, b)\n").unwrap();
    let facts = Facts::collect(root).unwrap();
    assert_eq!(
        facts.added_litellm_caller_lines,
        ["helper = cast(A, b)", "caller = cast(A, b)"]
    );
    assert_eq!(facts.added_python_lines, ["other = cast(A, b)"]);
}
