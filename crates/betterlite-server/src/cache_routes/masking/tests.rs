#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_redact::structured::SensitiveDataMasker;
use secrecy::SecretString;
use serde_json::Number;

use super::super::support_tests::connection_settings;
use super::redis_kwargs;

#[test]
fn an_empty_connection_settings_has_no_kwargs() {
    assert!(redis_kwargs(&connection_settings()).is_empty());
}

#[test]
fn the_text_and_number_settings_appear_as_kwargs() {
    let mut settings = connection_settings();
    settings.host = Some("redis.internal".to_owned());
    settings.port = Some(6379_i64);
    settings.db = Some(2_i64);
    settings.username = Some("proxy".to_owned());
    settings.max_connections = Some(20_i64);
    settings.health_check_interval = Some(30_i64);
    let kwargs = redis_kwargs(&settings);
    assert_eq!(
        kwargs.get("host"),
        Some(&Json::Text("redis.internal".to_owned()))
    );
    assert_eq!(
        kwargs.get("port"),
        Some(&Json::Number(Number::from(6379_i64)))
    );
    assert_eq!(kwargs.get("db"), Some(&Json::Number(Number::from(2_i64))));
    assert_eq!(
        kwargs.get("username"),
        Some(&Json::Text("proxy".to_owned()))
    );
    assert_eq!(
        kwargs.get("max_connections"),
        Some(&Json::Number(Number::from(20_i64)))
    );
    assert_eq!(
        kwargs.get("health_check_interval"),
        Some(&Json::Number(Number::from(30_i64)))
    );
    assert_eq!(kwargs.len(), 6);
}

#[test]
fn the_bool_and_float_settings_appear_as_kwargs() {
    let mut settings = connection_settings();
    settings.ssl = Some(true);
    settings.socket_timeout = Some(5.0_f64);
    settings.socket_connect_timeout = Some(1.5_f64);
    settings.socket_keepalive = Some(false);
    let kwargs = redis_kwargs(&settings);
    assert_eq!(kwargs.get("ssl"), Some(&Json::Bool(true)));
    assert_eq!(
        kwargs.get("socket_timeout"),
        Some(&Json::Number(Number::from_f64(5.0_f64).unwrap()))
    );
    assert_eq!(
        kwargs.get("socket_connect_timeout"),
        Some(&Json::Number(Number::from_f64(1.5_f64).unwrap()))
    );
    assert_eq!(kwargs.get("socket_keepalive"), Some(&Json::Bool(false)));
    assert_eq!(kwargs.len(), 4);
}

#[test]
fn the_password_is_masked() {
    let mut settings = connection_settings();
    settings.password = Some(SecretString::from("hunter2-and-more-text".to_owned()));
    let kwargs = redis_kwargs(&settings);
    let expected = SensitiveDataMasker::default().mask_value("hunter2-and-more-text");
    assert_eq!(kwargs.get("password"), Some(&Json::Text(expected)));
    assert_ne!(
        kwargs.get("password"),
        Some(&Json::Text("hunter2-and-more-text".to_owned()))
    );
}

#[test]
fn a_short_password_is_fully_masked() {
    let mut settings = connection_settings();
    settings.password = Some(SecretString::from("abc".to_owned()));
    assert_eq!(
        redis_kwargs(&settings).get("password"),
        Some(&Json::Text("***".to_owned()))
    );
}

#[test]
fn every_secret_connection_field_is_masked() {
    let mut settings = connection_settings();
    settings.sentinel_password = Some(SecretString::from("sentinel-secret-value".to_owned()));
    settings.ssl_password = Some(SecretString::from("ssl-secret-value-here".to_owned()));
    settings.service_name = Some("mymaster".to_owned());
    let kwargs = redis_kwargs(&settings);
    let masker = SensitiveDataMasker::default();
    assert_eq!(
        kwargs.get("sentinel_password"),
        Some(&Json::Text(masker.mask_value("sentinel-secret-value")))
    );
    assert_eq!(
        kwargs.get("ssl_password"),
        Some(&Json::Text(masker.mask_value("ssl-secret-value-here")))
    );
    assert_eq!(
        kwargs.get("service_name"),
        Some(&Json::Text("mymaster".to_owned()))
    );
}

#[test]
fn a_value_that_is_not_a_finite_number_is_left_out() {
    let mut settings = connection_settings();
    settings.socket_timeout = Some(f64::NAN);
    let kwargs = redis_kwargs(&settings);
    assert_eq!(kwargs.get("socket_timeout"), None);
    assert!(kwargs.is_empty());
}
