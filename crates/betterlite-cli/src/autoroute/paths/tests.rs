#![cfg(test)]

use std::path::{Path, PathBuf};

use super::{autoroute_dir, config_path, log_path, pid_record_path};

fn home() -> PathBuf {
    Path::new("/home/tester").to_path_buf()
}

#[test]
fn paths_resolve_under_the_litellm_autorouter_directory() {
    let directory = home().join(".litellm").join("autorouter");
    assert_eq!(autoroute_dir(&home()), directory);
    assert_eq!(config_path(&home()), directory.join("config.yaml"));
    assert_eq!(log_path(&home()), directory.join("proxy.log"));
    assert_eq!(pid_record_path(&home()), directory.join("proxy.pid.json"));
}

#[test]
fn paths_do_not_touch_the_file_system() {
    let scratch = tempfile::TempDir::new().expect("a scratch home");
    let missing = scratch.path().join("never-created");
    let _resolved = config_path(&missing);
    assert!(!missing.exists());
}
