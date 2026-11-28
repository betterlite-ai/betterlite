#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::load_control_plane_url;
use crate::loader::ConfigError;

fn lookup() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "CONTROL_PLANE_URL".to_owned(),
        "https://cp.example.com".to_owned(),
    )])
}

fn load(source: &str) -> Option<String> {
    load_control_plane_url(source, &lookup()).unwrap()
}

#[test]
fn loads_control_plane_url() {
    assert_eq!(load("model_list: []\n"), None);
    assert_eq!(load("general_settings: null\n"), None);
    assert_eq!(load("general_settings:\n  control_plane_url: null\n"), None);
    assert_eq!(
        load("general_settings:\n  control_plane_url: https://cp.example.com\n").as_deref(),
        Some("https://cp.example.com")
    );
    assert_eq!(
        load("general_settings:\n  control_plane_url: os.environ/CONTROL_PLANE_URL\n").as_deref(),
        Some("https://cp.example.com")
    );
    // an explicit empty string stays configured, as general_settings.get returns "" rather than None
    assert_eq!(
        load("general_settings:\n  control_plane_url: \"\"\n").as_deref(),
        Some("")
    );
}

#[test]
fn loads_control_plane_url_rejects_a_non_string_value() {
    let error = load_control_plane_url("general_settings:\n  control_plane_url: 12\n", &lookup())
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. } if path == "general_settings.control_plane_url"
    ));
}

#[test]
fn loads_control_plane_url_names_an_absent_environment_variable() {
    let error = load_control_plane_url(
        "general_settings:\n  control_plane_url: os.environ/ABSENT_CONTROL_PLANE_URL\n",
        &lookup(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "ABSENT_CONTROL_PLANE_URL"
                && path == "general_settings.control_plane_url"
    ));
}

// this corpus replay runs under the gate's replays_the_config_yaml_corpus filter, as the server one does
#[rstest]
fn replays_the_config_yaml_corpus(
    #[files("fuzz/corpus/config_yaml/sso-return-target-*")] path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    drop(load_control_plane_url(&source, &lookup()));
}
