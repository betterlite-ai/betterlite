#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::ErrorType;
use betterlite_models::decisions::Json;

use super::{CacheSettingsManager, params_equal};
use crate::cache_settings_update::support;

fn string(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

#[test]
fn params_equal_skips_redis_type_and_nulls() {
    let left = support::params(r#"{"host":"h","port":null,"redis_type":"node"}"#);
    let right = support::params(r#"{"host":"h","redis_type":"cluster"}"#);
    assert!(params_equal(&left, &right));
    let missing_port = support::params(r#"{"host":"h"}"#);
    assert!(params_equal(&left, &missing_port));
}

#[test]
fn params_equal_compares_python_str_forms() {
    let typed = support::params(r#"{"ttl":1,"flag":true,"ratio":1.5,"name":"x"}"#);
    let textual = support::params(r#"{"ttl":"1","flag":"True","ratio":"1.5","name":"x"}"#);
    assert!(params_equal(&typed, &textual));
    let lower_flag = support::params(r#"{"ttl":"1","flag":"true","ratio":"1.5","name":"x"}"#);
    assert!(!params_equal(&typed, &lower_flag));
    let other_ratio = support::params(r#"{"ttl":"1","flag":"True","ratio":"1.25","name":"x"}"#);
    assert!(!params_equal(&typed, &other_ratio));
    let float_ttl = support::params(r#"{"ttl":1.0}"#);
    let int_ttl = support::params(r#"{"ttl":1}"#);
    assert!(!params_equal(&float_ttl, &int_ttl));
    let float_text = support::params(r#"{"ttl":"1.0"}"#);
    assert!(params_equal(&float_ttl, &float_text));
}

#[test]
fn params_equal_keeps_lists_and_objects_structural() {
    let nodes = support::params(r#"{"redis_startup_nodes":[{"host":"a","port":1}]}"#);
    let same = support::params(r#"{"redis_startup_nodes":[{"host":"a","port":1}]}"#);
    let other = support::params(r#"{"redis_startup_nodes":[{"host":"a","port":2}]}"#);
    assert!(params_equal(&nodes, &same));
    assert!(!params_equal(&nodes, &other));
    let text = support::params(r#"{"redis_startup_nodes":"[{\"host\":\"a\",\"port\":1}]"}"#);
    assert!(!params_equal(&nodes, &text));
    let float_port = support::params(r#"{"redis_startup_nodes":[{"host":"a","port":1.0}]}"#);
    assert!(!params_equal(&nodes, &float_port));
}

#[test]
fn params_equal_treats_a_missing_key_as_a_difference() {
    let host_only = support::params(r#"{"host":"h"}"#);
    let host_and_port = support::params(r#"{"host":"h","port":6379}"#);
    assert!(!params_equal(&host_only, &host_and_port));
}

#[test]
fn update_params_stores_a_copy() {
    let manager = CacheSettingsManager::default();
    let mut first = support::params(r#"{"host":"h"}"#);
    manager.update_params(first.clone());
    first.insert("host".to_owned(), Json::Text("other".to_owned()));
    let stored = manager.last_params().expect("the params were stored");
    assert_eq!(stored.get("host"), Some(&Json::Text("h".to_owned())));
}

#[tokio::test]
async fn init_from_db_skips_equal_params() {
    let (logs, _guard) = support::capture_logs();
    let (fake, erased) = support::fake_repository();
    fake.push_read(Ok(Some(support::stored_row(&[
        ("host", string("h")),
        ("redis_type", string("node")),
    ]))));
    let manager = CacheSettingsManager::default();
    manager.update_params(support::params(r#"{"host":"h"}"#));
    let reinit = support::reinit();
    let cipher = support::cipher();
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            None::<&mut support::FakeReconnect>,
        )
        .await;
    assert!(reinit.calls().is_empty());
    assert!(
        logs.text()
            .contains("Cache settings unchanged, skipping reinitialization")
    );
}

#[tokio::test]
async fn init_from_db_reinitializes_changed_params() {
    let (logs, _guard) = support::capture_logs();
    let (fake, erased) = support::fake_repository();
    fake.push_read(Ok(Some(support::stored_row(&[
        ("type", string("redis")),
        ("host", string("h")),
        ("password", string(&support::sealed("saved-secret"))),
        ("redis_type", string("node")),
    ]))));
    let manager = CacheSettingsManager::default();
    manager.update_params(support::params(r#"{"host":"old"}"#));
    let reinit = support::reinit();
    let cipher = support::cipher();
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            None::<&mut support::FakeReconnect>,
        )
        .await;
    assert_eq!(
        reinit.calls(),
        vec![
            "init_cache".to_owned(),
            "switch_on_response_caching".to_owned()
        ]
    );
    let expected = support::params(r#"{"type":"redis","host":"h","password":"saved-secret"}"#);
    let initialized = &reinit.params()[0];
    assert!(params_equal(initialized, &expected));
    let stored = manager.last_params().expect("the params were stored");
    assert!(params_equal(&stored, &expected));
    let debugged = logs.text();
    assert!(debugged.contains("Cache settings initialized from database"));
}

#[tokio::test]
async fn init_from_db_ignores_a_missing_or_empty_row() {
    let (fake, erased) = support::fake_repository();
    fake.push_read(Ok(None));
    fake.push_read(Ok(Some(support::json("{}"))));
    let manager = CacheSettingsManager::default();
    let reinit = support::reinit();
    let cipher = support::cipher();
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            None::<&mut support::FakeReconnect>,
        )
        .await;
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            None::<&mut support::FakeReconnect>,
        )
        .await;
    assert!(reinit.calls().is_empty());
    assert!(manager.last_params().is_none());
    assert_eq!(fake.read_count(), 2);
}

#[tokio::test]
async fn init_from_db_logs_and_skips_a_stored_value_that_is_not_an_object() {
    let (logs, _guard) = support::capture_logs();
    let (fake, erased) = support::fake_repository();
    fake.push_read(Ok(Some(JsonValue::String("null".to_owned()))));
    fake.push_read(Ok(Some(support::json(r#"[["host","h"]]"#))));
    let manager = CacheSettingsManager::default();
    let reinit = support::reinit();
    let cipher = support::cipher();
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            None::<&mut support::FakeReconnect>,
        )
        .await;
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            None::<&mut support::FakeReconnect>,
        )
        .await;
    assert!(reinit.calls().is_empty());
    assert!(manager.last_params().is_none());
    assert_eq!(fake.read_count(), 2);
    let text = logs.text();
    assert!(text.contains(
        "cache_settings_endpoints.py::CacheSettingsManager::init_cache_settings_in_db - "
    ));
    assert!(text.contains("stored cache settings are not a JSON object"));
}

#[tokio::test]
async fn init_from_db_logs_and_swallows_a_failed_lookup() {
    let (logs, _guard) = support::capture_logs();
    let (fake, erased) = support::fake_repository();
    fake.push_read(Err(support::database_error(
        ErrorType::Other,
        "the row is unreadable",
    )));
    let manager = CacheSettingsManager::default();
    let reinit = support::reinit();
    let cipher = support::cipher();
    let mut reconnect = support::FakeReconnect::new(true);
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            Some(&mut reconnect),
        )
        .await;
    assert!(reinit.calls().is_empty());
    assert_eq!(reconnect.attempts(), 0);
    assert_eq!(fake.read_count(), 1);
    let text = logs.text();
    assert!(text.contains(
        "cache_settings_endpoints.py::CacheSettingsManager::init_cache_settings_in_db - "
    ));
    assert!(text.contains("the row is unreadable"));
}

#[tokio::test]
async fn init_from_db_retries_once_on_a_transport_failure() {
    let (fake, erased) = support::fake_repository();
    fake.push_read(Err(support::database_error(
        ErrorType::Connect,
        "connection closed",
    )));
    fake.push_read(Ok(Some(support::stored_row(&[("host", string("h"))]))));
    let manager = CacheSettingsManager::default();
    let reinit = support::reinit();
    let cipher = support::cipher();
    let mut reconnect = support::FakeReconnect::new(true);
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            Some(&mut reconnect),
        )
        .await;
    assert_eq!(reconnect.attempts(), 1);
    assert_eq!(
        reconnect.reasons(),
        vec!["init_cache_settings_in_db_lookup_failure".to_owned()]
    );
    assert_eq!(fake.read_count(), 2);
    assert_eq!(
        reinit.calls(),
        vec![
            "init_cache".to_owned(),
            "switch_on_response_caching".to_owned()
        ]
    );
}

#[tokio::test]
async fn init_from_db_gives_up_when_the_reconnect_fails() {
    let (logs, _guard) = support::capture_logs();
    let (fake, erased) = support::fake_repository();
    fake.push_read(Err(support::database_error(
        ErrorType::Connect,
        "connection closed",
    )));
    let manager = CacheSettingsManager::default();
    let reinit = support::reinit();
    let cipher = support::cipher();
    let mut reconnect = support::FakeReconnect::new(false);
    manager
        .init_from_db(
            erased.as_ref(),
            &cipher,
            reinit.as_ref(),
            Some(&mut reconnect),
        )
        .await;
    assert_eq!(reconnect.attempts(), 1);
    assert_eq!(fake.read_count(), 1);
    assert!(reinit.calls().is_empty());
    assert!(logs.text().contains("connection closed"));
}
