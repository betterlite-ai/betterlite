#![cfg(test)]

use crate::api::container_contracts::config::BaseContainerConfig as _;

use super::support::{config_with, globals_with, settings_with};

const SECRET: &str = "sk-super-secret-fixture";
const ENV_SECRET: &str = "sk-env-secret-fixture";

#[test]
fn openai_containers_secret_safe_failure() {
    let config = config_with(
        settings_with("https://fixture.test/v1"),
        globals_with(Some(SECRET), None),
        Some(ENV_SECRET),
    );
    let debug = format!("{config:?}");
    assert!(!debug.contains(SECRET));
    assert!(!debug.contains(ENV_SECRET));
    assert!(debug.contains("[REDACTED]"));
    let openai_only = config_with(
        settings_with("https://fixture.test/v1"),
        globals_with(None, Some(SECRET)),
        None,
    );
    assert!(!format!("{openai_only:?}").contains(SECRET));
    let failure = config
        .transform_container_retrieve_response(br#"{"id": 5}"#)
        .unwrap_err();
    assert!(!failure.to_string().contains(SECRET));
    assert!(!failure.to_string().contains(ENV_SECRET));
    let failure_debug = format!("{failure:?}");
    assert!(!failure_debug.contains(SECRET));
    assert!(!failure_debug.contains(ENV_SECRET));
    let truncated = config
        .transform_container_file_list_response(br#"{"object":"list","data":"#)
        .unwrap_err();
    assert!(!truncated.to_string().contains(SECRET));
    assert!(!format!("{truncated:?}").contains(ENV_SECRET));
}
