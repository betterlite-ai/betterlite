#![cfg(test)]

use std::collections::BTreeMap;

use super::env_overlay;
use crate::config_boot::test_support::{
    capture_logs, config_from_yaml, managed_state, secret_lookup,
};

fn process_env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn blocked_keys_are_skipped_with_a_warning() {
    let config = config_from_yaml(
        "environment_variables:\n  PATH: /evil\n  no_proxy: evil.example\n  HOME: /evil\n",
    );
    let (logs, _guard) = capture_logs();
    let overlay = env_overlay(
        &config,
        &process_env(&[("PATH", "/usr/bin")]),
        &secret_lookup(None),
    )
    .unwrap();
    assert_eq!(overlay.get("PATH").map(String::as_str), Some("/usr/bin"));
    assert!(!overlay.contains_key("no_proxy"));
    assert!(!overlay.contains_key("HOME"));
    let logged = logs.contents();
    assert!(logged.contains("Skipping blocked environment variable key: PATH"));
    assert!(logged.contains("Skipping blocked environment variable key: no_proxy"));
    assert!(logged.contains("Skipping blocked environment variable key: HOME"));
}

#[test]
fn blocked_keys_are_matched_case_sensitively() {
    let config = config_from_yaml("environment_variables:\n  Path: /custom\n");
    let (logs, _guard) = capture_logs();
    let overlay = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap();
    assert_eq!(overlay.get("Path").map(String::as_str), Some("/custom"));
    assert!(
        !logs
            .contents()
            .contains("Skipping blocked environment variable key")
    );
}

#[test]
fn every_blocked_key_is_refused() {
    let config = config_from_yaml(
        "environment_variables:\n  LD_PRELOAD: a\n  LD_LIBRARY_PATH: b\n  DYLD_LIBRARY_PATH: c\n  DYLD_INSERT_LIBRARIES: d\n  PYTHONPATH: e\n  PYTHONSTARTUP: f\n  PYTHONHOME: g\n  USER: h\n  SHELL: i\n  LOGNAME: j\n  NO_PROXY: k\n",
    );
    let overlay = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap();
    assert!(overlay.is_empty());
}

#[test]
fn the_mcp_stdio_flag_is_skipped_in_any_case() {
    let config = config_from_yaml("environment_variables:\n  litellm_enable_mcp_stdio: true\n");
    let (logs, _guard) = capture_logs();
    let overlay = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap();
    assert!(overlay.is_empty());
    assert!(
        logs.contents().contains(
            "Ignoring LITELLM_ENABLE_MCP_STDIO set in the config file. Set it in the proxy's environment instead"
        )
    );
}

#[test]
fn resolved_references_see_earlier_entries_in_the_overlay() {
    let config =
        config_from_yaml("environment_variables:\n  FIRST: literal\n  SECOND: os.environ/FIRST\n");
    let overlay = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap();
    assert_eq!(overlay.get("SECOND").map(String::as_str), Some("literal"));
}

#[test]
fn unresolved_references_leave_the_key_unset() {
    let config =
        config_from_yaml("environment_variables:\n  MISSING: os.environ/NOT_SET_ANYWHERE\n");
    let overlay = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap();
    assert!(!overlay.contains_key("MISSING"));
    assert!(!overlay.contains_key("NOT_SET_ANYWHERE"));
}

#[test]
fn references_resolve_through_the_secret_manager() {
    let config = config_from_yaml("environment_variables:\n  ORDER: os.environ/ORDER\n");
    let secrets = secret_lookup(Some(managed_state(Some("managed-value"))));
    let overlay = env_overlay(&config, &process_env(&[]), &secrets).unwrap();
    assert_eq!(
        overlay.get("ORDER").map(String::as_str),
        Some("managed-value")
    );
}

#[test]
fn scalar_values_take_python_text() {
    let config = config_from_yaml(
        "environment_variables:\n  INT: 3\n  FLOAT: 1.5\n  YES: true\n  NO: false\n  NOTHING: null\n  TEXT: hello\n",
    );
    let overlay = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap();
    assert_eq!(overlay.get("INT").map(String::as_str), Some("3"));
    assert_eq!(overlay.get("FLOAT").map(String::as_str), Some("1.5"));
    assert_eq!(overlay.get("YES").map(String::as_str), Some("True"));
    assert_eq!(overlay.get("NO").map(String::as_str), Some("False"));
    assert_eq!(overlay.get("NOTHING").map(String::as_str), Some("None"));
    assert_eq!(overlay.get("TEXT").map(String::as_str), Some("hello"));
}

#[test]
fn a_missing_environment_variables_key_changes_nothing() {
    let config = config_from_yaml("model_list:\n  - model_name: gpt\n");
    let base = process_env(&[("KEEP", "yes")]);
    let overlay = env_overlay(&config, &base, &secret_lookup(None)).unwrap();
    assert_eq!(overlay, base);
}

#[test]
fn a_null_environment_variables_key_changes_nothing() {
    let config = config_from_yaml("environment_variables:\n");
    let base = process_env(&[("KEEP", "yes")]);
    let overlay = env_overlay(&config, &base, &secret_lookup(None)).unwrap();
    assert_eq!(overlay, base);
}

#[test]
fn a_non_mapping_environment_variables_is_refused() {
    let config = config_from_yaml("environment_variables: hello\n");
    let error = env_overlay(&config, &process_env(&[]), &secret_lookup(None)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "config field 'environment_variables' must be a mapping"
    );
}

#[test]
fn existing_process_values_survive_alongside_config_entries() {
    let config = config_from_yaml("environment_variables:\n  ADDED: value\n");
    let overlay = env_overlay(
        &config,
        &process_env(&[("KEEP", "yes")]),
        &secret_lookup(None),
    )
    .unwrap();
    assert_eq!(overlay.get("KEEP").map(String::as_str), Some("yes"));
    assert_eq!(overlay.get("ADDED").map(String::as_str), Some("value"));
}
