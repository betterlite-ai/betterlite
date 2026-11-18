#![cfg(test)]

use super::support::{json, settings};
use crate::coord_redis_boot::error::CoordRedisError;
use crate::coord_redis_boot::settings::CoordinationRedisSettings;
use crate::proxy_request_contracts::coordination_redis::BoolOrText;

#[test]
fn from_config_reads_the_two_sections_and_ignores_every_other_key() {
    let config = json(
        r#"{
            "general_settings": {
                "database_url": "postgres://proxy",
                "coordination_redis": {"host": "redis.internal", "port": 6379},
                "use_redis_transaction_buffer": true
            },
            "litellm_settings": {"drop_params": true, "enable_redis_auth_cache": true},
            "model_list": []
        }"#,
    );
    let read = CoordinationRedisSettings::from_config(&config).expect("the sections read");
    let block = read
        .general_settings
        .coordination_redis
        .expect("the block is kept");
    assert_eq!(
        block.value,
        json(r#"{"host": "redis.internal", "port": 6379}"#)
    );
    assert_eq!(
        read.general_settings.use_redis_transaction_buffer,
        Some(BoolOrText::Bool(true))
    );
    assert_eq!(read.litellm_settings.enable_redis_auth_cache, Some(true));
}

#[test]
fn from_config_takes_nulls_and_absence_as_the_default() {
    let empty = settings("{}");
    assert!(empty.general_settings.coordination_redis.is_none());
    assert!(
        empty
            .general_settings
            .use_redis_transaction_buffer
            .is_none()
    );
    assert!(empty.litellm_settings.enable_redis_auth_cache.is_none());
    let nulls = settings(r#"{"general_settings": null, "litellm_settings": null, "other": null}"#);
    assert!(nulls.general_settings.coordination_redis.is_none());
    let fields = settings(
        r#"{"general_settings": {"coordination_redis": null, "use_redis_transaction_buffer": null}, "litellm_settings": {"enable_redis_auth_cache": null}}"#,
    );
    assert!(fields.general_settings.coordination_redis.is_none());
    assert!(
        fields
            .general_settings
            .use_redis_transaction_buffer
            .is_none()
    );
    assert!(fields.litellm_settings.enable_redis_auth_cache.is_none());
}

#[test]
fn from_config_keeps_a_text_buffer_flag_for_the_str_to_bool_read() {
    let read = settings(r#"{"general_settings": {"use_redis_transaction_buffer": "TRUE"}}"#);
    assert_eq!(
        read.general_settings.use_redis_transaction_buffer,
        Some(BoolOrText::Text("TRUE".to_owned()))
    );
}

#[test]
fn from_config_rejects_a_general_settings_that_is_not_a_mapping() {
    let config = json(r#"{"general_settings": "postgres://proxy"}"#);
    let error = CoordinationRedisSettings::from_config(&config)
        .err()
        .expect("a text section does not read");
    assert!(matches!(error, CoordRedisError::Settings { .. }));
    let message = error.to_string();
    assert!(
        message.starts_with("the coordination Redis settings could not be read: "),
        "{message}"
    );
    assert!(message.contains("general_settings"), "{message}");
}

#[test]
fn from_config_rejects_a_litellm_settings_that_is_not_a_mapping() {
    let config = json(r#"{"general_settings": {}, "litellm_settings": ["drop_params"]}"#);
    let error = CoordinationRedisSettings::from_config(&config)
        .err()
        .expect("an array section does not read");
    assert!(matches!(error, CoordRedisError::Settings { .. }));
    assert!(error.to_string().contains("litellm_settings"), "{error}");
}

#[test]
fn from_config_rejects_a_text_auth_flag() {
    let config = json(r#"{"litellm_settings": {"enable_redis_auth_cache": "true"}}"#);
    let error = CoordinationRedisSettings::from_config(&config)
        .err()
        .expect("a text flag is not a bool");
    assert!(matches!(error, CoordRedisError::Settings { .. }));
    let message = error.to_string();
    assert!(
        message.contains("litellm_settings.enable_redis_auth_cache"),
        "{message}"
    );
}

#[test]
fn from_config_rejects_a_number_buffer_flag() {
    let config = json(r#"{"general_settings": {"use_redis_transaction_buffer": 1}}"#);
    let error = CoordinationRedisSettings::from_config(&config)
        .err()
        .expect("a number flag is not a bool or a text");
    assert!(matches!(error, CoordRedisError::Settings { .. }));
    assert!(
        error
            .to_string()
            .contains("general_settings.use_redis_transaction_buffer"),
        "{error}"
    );
}

#[test]
fn from_config_rejects_a_config_that_is_not_a_mapping() {
    let config = json(r#""coordination""#);
    let error = CoordinationRedisSettings::from_config(&config)
        .err()
        .expect("a text is not a config");
    assert!(matches!(error, CoordRedisError::Settings { .. }));
    let message = error.to_string();
    assert!(
        message.starts_with("the coordination Redis settings could not be read: "),
        "{message}"
    );
    assert!(message.contains("invalid type: string"), "{message}");
}

#[test]
fn from_config_rejects_an_object_buffer_flag() {
    let config = json(r#"{"general_settings": {"use_redis_transaction_buffer": {"on": true}}}"#);
    let error = CoordinationRedisSettings::from_config(&config)
        .err()
        .expect("an object flag is neither a bool nor a text");
    assert!(
        error
            .to_string()
            .contains("general_settings.use_redis_transaction_buffer"),
        "{error}"
    );
}
