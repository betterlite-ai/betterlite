#![cfg(test)]

use rstest::rstest;
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

#[rstest]
#[case::root_default_empty("{}", "")]
#[case::root_slash("SERVER_ROOT_PATH: /", "/")]
#[case::root_prefix("SERVER_ROOT_PATH: /proxy", "/proxy")]
#[case::root_trailing_slash("SERVER_ROOT_PATH: /proxy/", "/proxy/")]
fn loads_root(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(root(source), expected);
}

#[test]
fn reads_root_from_environment() {
    let config = ConfigLoader {
        lookup: BTreeMap::from([("SERVER_ROOT_PATH".to_owned(), "/environment/".to_owned())]),
    }
    .load("{}")
    .unwrap();
    assert_eq!(
        config.sections.pass_through_registry.server_root_path,
        "/environment/"
    );
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
