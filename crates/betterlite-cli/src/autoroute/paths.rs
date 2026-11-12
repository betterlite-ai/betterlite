use std::path::{Path, PathBuf};

const AUTOROUTE_PARENT: &str = ".litellm";
const AUTOROUTE_DIRECTORY: &str = "autorouter";
const CONFIG_FILE: &str = "config.yaml";
const LOG_FILE: &str = "proxy.log";
const PID_RECORD_FILE: &str = "proxy.pid.json";

#[must_use]
pub fn autoroute_dir(home: &Path) -> PathBuf {
    let mut path = home.to_path_buf();
    path.extend([AUTOROUTE_PARENT, AUTOROUTE_DIRECTORY]);
    path
}

#[must_use]
pub fn config_path(home: &Path) -> PathBuf {
    let mut path = home.to_path_buf();
    path.extend([AUTOROUTE_PARENT, AUTOROUTE_DIRECTORY, CONFIG_FILE]);
    path
}

#[must_use]
pub fn log_path(home: &Path) -> PathBuf {
    let mut path = home.to_path_buf();
    path.extend([AUTOROUTE_PARENT, AUTOROUTE_DIRECTORY, LOG_FILE]);
    path
}

#[must_use]
pub fn pid_record_path(home: &Path) -> PathBuf {
    let mut path = home.to_path_buf();
    path.extend([AUTOROUTE_PARENT, AUTOROUTE_DIRECTORY, PID_RECORD_FILE]);
    path
}

mod tests;
