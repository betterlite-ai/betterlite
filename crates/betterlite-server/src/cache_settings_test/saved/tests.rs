#![cfg(test)]

use rstest::rstest;
use serde_json::Value;
use serde_json::json;

use crate::cache_settings_test::support;
use crate::cache_settings_test::support::SavedReply;

use super::saved_settings;

#[tokio::test]
async fn saved_settings_decrypts_only_string_values() {
    let stored = json!({
        "password": support::sealed(support::SAVED_SECRET),
        "port": 6379,
        "ssl": true,
        "redis_startup_nodes": [{"host": "127.0.0.1", "port": 7000}],
        "nested": {"password": "plain"},
    });
    let (_, repository) = support::fake_saved(SavedReply::Stored(stored));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert_eq!(settings["password"], support::SAVED_SECRET);
    assert_eq!(settings["port"], 6379);
    assert_eq!(settings["ssl"], true);
    assert_eq!(settings["redis_startup_nodes"][0]["host"], "127.0.0.1");
    assert_eq!(settings["nested"]["password"], "plain");
}

#[tokio::test]
async fn saved_settings_keeps_a_value_it_cannot_decrypt() {
    let stored = json!({
        "password": "not-a-ciphertext",
        "host": support::sealed("127.0.0.1"),
    });
    let (_, repository) = support::fake_saved(SavedReply::Stored(stored));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert_eq!(settings["password"], "not-a-ciphertext");
    assert_eq!(settings["host"], "127.0.0.1");
}

#[tokio::test]
async fn saved_settings_keeps_the_stored_text_without_a_signing_key() {
    let sealed = support::sealed(support::SAVED_SECRET);
    let (_, repository) =
        support::fake_saved(SavedReply::Stored(json!({"password": sealed.as_str()})));
    let settings = saved_settings(repository.as_deref(), &support::cipher_without_key()).await;
    assert_eq!(settings["password"].as_str(), Some(sealed.as_str()));
}

#[tokio::test]
async fn saved_settings_is_empty_without_a_repository() {
    let settings = saved_settings(None, &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn saved_settings_is_empty_when_the_row_is_missing() {
    let (_, repository) = support::fake_saved(SavedReply::Missing);
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn saved_settings_is_empty_when_the_repository_fails() {
    let (saved_row, repository) = support::fake_saved(SavedReply::Failed);
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
    assert_eq!(saved_row.calls(), 1);
}

#[tokio::test]
async fn saved_settings_is_empty_when_the_row_does_not_parse() {
    let (_, repository) = support::fake_saved(SavedReply::Stored(json!(r#"{"password":"#)));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
}

#[tokio::test]
async fn saved_settings_parses_a_stored_json_string() {
    let sealed = support::sealed(support::SAVED_SECRET);
    let text = format!(r#"{{"password":"{sealed}","port":6379}}"#);
    let (_, repository) = support::fake_saved(SavedReply::Stored(json!(text)));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert_eq!(settings["password"], support::SAVED_SECRET);
    assert_eq!(settings["port"], 6379);
}

#[rstest]
#[case::number(json!(5))]
#[case::array(json!([1]))]
#[case::flag(json!(true))]
#[case::quoted(json!(r#""text""#))]
#[tokio::test]
async fn saved_settings_is_empty_for_a_stored_non_object(#[case] stored: Value) {
    let (_, repository) = support::fake_saved(SavedReply::Stored(stored));
    let settings = saved_settings(repository.as_deref(), &support::cipher()).await;
    assert!(settings.is_empty());
}
