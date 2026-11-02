#![cfg(test)]

use std::fs;

use super::check;

#[test]
fn maintenance_blocks_dispatch_until_the_marker_is_explicitly_removed() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("betterlite")).unwrap();
    let marker = directory.path().join("betterlite/maintenance");
    fs::write(&marker, "maintenance").unwrap();
    assert!(
        check(directory.path(), "B000")
            .unwrap_err()
            .to_string()
            .contains("no agents will start")
    );
    fs::remove_file(marker).unwrap();
    check(directory.path(), "B000").unwrap();
}
