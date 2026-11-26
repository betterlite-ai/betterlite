use std::collections::BTreeMap;

use crate::settings_services::config::{SettingsConfigError, settings_block};

use super::support::{SETTING, json, values};

#[test]
fn settings_block_reads_the_selected_block_and_treats_absent_or_null_as_empty() {
    let selected = values(&[("user_role", json!("internal_user_viewer"))]);
    let config = values(&[(
        "litellm_settings",
        json!({"other_setting": {"enabled": true}, "default_internal_user_params": {"user_role": "internal_user_viewer"}}),
    )]);
    assert_eq!(settings_block(&config, SETTING.as_str()).unwrap(), selected);
    let missing = BTreeMap::new();
    assert_eq!(
        settings_block(&missing, SETTING.as_str()).unwrap(),
        BTreeMap::new()
    );
    let null_section = values(&[("litellm_settings", json!(null))]);
    assert_eq!(
        settings_block(&null_section, SETTING.as_str()).unwrap(),
        BTreeMap::new()
    );
    let empty_section = values(&[("litellm_settings", json!({}))]);
    assert_eq!(
        settings_block(&empty_section, SETTING.as_str()).unwrap(),
        BTreeMap::new()
    );
    let null_selected = values(&[(
        "litellm_settings",
        json!({"default_internal_user_params": null}),
    )]);
    assert_eq!(
        settings_block(&null_selected, SETTING.as_str()).unwrap(),
        BTreeMap::new()
    );
}

#[test]
fn settings_block_refuses_a_non_object_litellm_settings_or_block() {
    let config = values(&[("litellm_settings", json!([1]))]);
    assert_eq!(
        settings_block(&config, SETTING.as_str()),
        Err(SettingsConfigError::NotAnObject {
            field: "litellm_settings".to_owned(),
        })
    );
    let config = values(&[(
        "litellm_settings",
        json!({"default_internal_user_params": "internal"}),
    )]);
    assert_eq!(
        settings_block(&config, SETTING.as_str()),
        Err(SettingsConfigError::NotAnObject {
            field: SETTING.as_str().to_owned(),
        })
    );
}
