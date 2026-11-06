#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::request_root_paths::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .request_root_paths)
}

#[test]
fn root_path_defaults_to_empty() {
    let settings = load("{}", &[]).unwrap();
    assert_eq!(settings.server_root_path, "");
    assert!(settings.server_root_paths.is_empty());
}

#[test]
fn root_path_reads_general_settings() {
    let settings = load(
        "general_settings:\n  SERVER_ROOT_PATH: /gateway\n  SERVER_ROOT_PATHS: /a, /b/\n",
        &[],
    )
    .unwrap();
    assert_eq!(settings.server_root_path, "/gateway");
    assert_eq!(settings.server_root_paths, ["/a", " /b/"]);
}

#[test]
fn root_path_reads_environment() {
    let settings = load(
        "{}",
        &[
            ("SERVER_ROOT_PATH", "/gateway"),
            ("SERVER_ROOT_PATHS", "/a,/b"),
        ],
    )
    .unwrap();
    assert_eq!(settings.server_root_path, "/gateway");
    assert_eq!(settings.server_root_paths, ["/a", "/b"]);
}

#[test]
fn root_path_general_settings_win_over_environment() {
    let settings = load(
        "general_settings:\n  SERVER_ROOT_PATH: /yaml\n  SERVER_ROOT_PATHS: /yaml-a\n",
        &[
            ("SERVER_ROOT_PATH", "/environment"),
            ("SERVER_ROOT_PATHS", "/environment-a"),
        ],
    )
    .unwrap();
    assert_eq!(settings.server_root_path, "/yaml");
    assert_eq!(settings.server_root_paths, ["/yaml-a"]);
}

#[test]
fn root_path_resolves_environment_references() {
    let settings = load(
        "general_settings:\n  SERVER_ROOT_PATH: os.environ/GATEWAY_ROOT\n  SERVER_ROOT_PATHS: os.environ/GATEWAY_ROOTS\n",
        &[("GATEWAY_ROOT", "/gateway"), ("GATEWAY_ROOTS", "/a,/b")],
    )
    .unwrap();
    assert_eq!(settings.server_root_path, "/gateway");
    assert_eq!(settings.server_root_paths, ["/a", "/b"]);
}

#[rstest]
#[case::scalar("SERVER_ROOT_PATH")]
#[case::list("SERVER_ROOT_PATHS")]
fn root_path_missing_environment_reference_names_the_variable(#[case] key: &str) {
    let source = format!("general_settings:\n  {key}: os.environ/ABSENT_ROOT\n");
    let error = load(&source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "ABSENT_ROOT" && path == format!("general_settings.{key}")
    ));
}

#[rstest]
#[case::empty("", &[])]
#[case::blank("  ", &[])]
#[case::single("/a", &["/a"])]
#[case::empty_entries_kept("/a,,/b,", &["/a", "", "/b", ""])]
fn root_path_list_splits_on_commas(#[case] text: &str, #[case] expected: &[&str]) {
    let settings = load("{}", &[("SERVER_ROOT_PATHS", text)]).unwrap();
    assert_eq!(settings.server_root_paths, expected);
}

#[test]
fn root_path_empty_environment_counts_as_unset() {
    let settings = load("{}", &[("SERVER_ROOT_PATH", "")]).unwrap();
    assert_eq!(settings.server_root_path, "");
}

#[rstest]
#[case::scalar("SERVER_ROOT_PATH: [1]")]
#[case::list("SERVER_ROOT_PATHS: {a: 1}")]
fn root_path_rejects_wrong_types(#[case] entry: &str) {
    let error = load(&format!("general_settings:\n  {entry}\n"), &[]).unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}
