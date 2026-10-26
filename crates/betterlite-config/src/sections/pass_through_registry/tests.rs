#![cfg(test)]

use rstest::rstest;
use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

#[rstest]
#[case::without_environment(None)]
#[case::with_environment(Some("/environment/"))]
fn rejects_top_level_root(#[case] environment: Option<&str>) {
    let lookup = environment
        .map(|value| ("SERVER_ROOT_PATH".to_owned(), value.to_owned()))
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let error = ConfigLoader { lookup }
        .load("SERVER_ROOT_PATH: /yaml/")
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Unsupported { paths } if paths == ["SERVER_ROOT_PATH"]
    ));
}

fn root(source: &str, environment: Option<&str>) -> String {
    let lookup = environment
        .map(|value| ("SERVER_ROOT_PATH".to_owned(), value.to_owned()))
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
    .load(source)
    .expect("config should load")
    .sections
    .pass_through_registry
    .server_root_path
}

#[rstest]
#[case::root_default_empty(None, "")]
#[case::root_slash(Some("/"), "/")]
#[case::root_prefix(Some("/proxy"), "/proxy")]
#[case::root_trailing_slash(Some("/proxy/"), "/proxy/")]
fn loads_root(#[case] environment: Option<&str>, #[case] expected: &str) {
    assert_eq!(root("{}", environment), expected);
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
        root("general_settings:\n  master_key: test-master\n", Some("/proxy")),
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
