#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

fn root(source: &str) -> String {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("config should load")
    .sections
    .pass_through_registry
    .server_root_path
}

#[test]
fn defaults_to_empty_root() {
    assert_eq!(root("{}"), "");
}

#[test]
fn loads_explicit_root_values() {
    assert_eq!(root("SERVER_ROOT_PATH: /proxy"), "/proxy");
    assert_eq!(root("SERVER_ROOT_PATH: /proxy/"), "/proxy/");
}

#[test]
fn root_coexists_with_general_settings() {
    assert_eq!(
        root("SERVER_ROOT_PATH: /proxy\ngeneral_settings:\n  master_key: test-master\n"),
        "/proxy"
    );
}

#[test]
fn preserves_missing_environment_error() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: os.environ/MISSING_MASTER_KEY\n")
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "MISSING_MASTER_KEY" && path == "general_settings.master_key"
    ));
}
