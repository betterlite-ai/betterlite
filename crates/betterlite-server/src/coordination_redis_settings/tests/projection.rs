use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::support::object;
use crate::coordination_redis_settings::projection::{
    merge_over_saved, redact_all_values, redact_credentials, scrub_credentials,
};

#[test]
fn redacts_each_credential_and_keeps_the_rest() {
    let settings = object(
        r#"{"host": "cache.internal", "port": 6379, "password": "pw", "sentinel_password": "sp", "url": "redis://:pw@cache.internal:6379/1"}"#,
    );
    assert_eq!(
        redact_credentials(Some(&settings)),
        object(
            r#"{"host": "cache.internal", "port": 6379, "password": "***REDACTED***", "sentinel_password": "***REDACTED***", "url": "***REDACTED***"}"#,
        ),
    );
}

#[test]
fn redacts_an_empty_credential_but_leaves_a_null_one() {
    let settings = object(r#"{"host": "", "password": "", "sentinel_password": null}"#);
    assert_eq!(
        redact_credentials(Some(&settings)),
        object(r#"{"host": "", "password": "***REDACTED***", "sentinel_password": null}"#),
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some("{}"))]
fn redacting_nothing_yields_an_empty_object(#[case] settings: Option<&str>) {
    let owned = settings.map(object);
    assert_eq!(redact_credentials(owned.as_ref()), object("{}"));
}

#[test]
fn merge_returns_only_incoming_keys_and_restores_saved_markers() {
    let incoming =
        object(r#"{"host": "new-host", "password": "***REDACTED***", "url": "***REDACTED***"}"#);
    let saved = object(
        r#"{"host": "old-host", "password": "saved-pw", "sentinel_password": "sp", "startup_nodes": []}"#,
    );
    assert_eq!(
        merge_over_saved(&incoming, Some(&saved)),
        object(r#"{"host": "new-host", "password": "saved-pw", "url": "***REDACTED***"}"#),
    );
}

#[test]
fn merge_without_a_saved_block_keeps_the_incoming_keys() {
    let incoming = object(r#"{"host": "h", "password": "***REDACTED***"}"#);
    assert_eq!(merge_over_saved(&incoming, None), incoming);
}

#[test]
fn audit_masks_every_value_and_keeps_the_keys() {
    let settings =
        object(r#"{"host": "h", "password": "p", "startup_nodes": [{"host": "a", "port": 1}]}"#);
    assert_eq!(
        redact_all_values(Some(&settings)),
        object(
            r#"{"host": "***REDACTED***", "password": "***REDACTED***", "startup_nodes": "***REDACTED***"}"#
        ),
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some("{}"))]
fn audit_of_nothing_yields_an_empty_object(#[case] settings: Option<&str>) {
    let owned = settings.map(object);
    assert_eq!(redact_all_values(owned.as_ref()), object("{}"));
}

#[test]
fn scrub_replaces_every_occurrence_of_a_supplied_credential() {
    let settings = object(
        r#"{"password": "hunter2", "sentinel_password": "sentinel-pass", "url": "redis://:hunter2@cache:6379/0", "host": "cache"}"#,
    );
    assert_eq!(
        scrub_credentials(
            "auth failed for hunter2 at redis://:hunter2@cache:6379/0",
            &settings
        ),
        "auth failed for ***REDACTED*** at redis://:***REDACTED***@cache:6379/0",
    );
}

#[test]
fn scrub_skips_empty_and_non_text_credentials() {
    let settings =
        object(r#"{"password": "", "sentinel_password": ["list"], "url": null, "host": "cache"}"#);
    assert_eq!(
        scrub_credentials("cache refused the connection", &settings),
        "cache refused the connection",
    );
}

#[test]
fn scrub_replaces_an_integer_credential_literally() {
    let settings = object(r#"{"password": 12345}"#);
    assert_eq!(
        scrub_credentials("auth 12345 rejected twice: 12345", &settings),
        "auth ***REDACTED*** rejected twice: ***REDACTED***",
    );
}

#[test]
fn scrub_ignores_a_fractional_credential() {
    let settings = object(r#"{"password": 1.5}"#);
    assert_eq!(scrub_credentials("value 1.5", &settings), "value 1.5");
}

#[test]
fn scrub_of_a_non_object_section_leaves_the_message_alone() {
    let settings = JsonValue::Array(Vec::new());
    assert_eq!(scrub_credentials("boom", &settings), "boom");
}
