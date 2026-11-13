use super::support::value;
use crate::spend_bodies::messages::storage_messages;

#[test]
fn stores_realtime_messages() {
    let payload =
        value(r#"{"call_type": "_arealtime", "messages": [{"role": "user", "content": "hi"}]}"#);
    assert_eq!(
        storage_messages(Some(&payload), true),
        r#"[{"role": "user", "content": "hi"}]"#
    );
}

#[rstest::rstest]
#[case::disabled("_arealtime", Some(r#"[{"role": "user"}]"#), false, "{}")]
#[case::other_call_type("completion", Some(r#"[{"role": "user"}]"#), true, "{}")]
#[case::null_messages("_arealtime", Some("null"), true, "{}")]
#[case::missing_messages("_arealtime", None, true, "{}")]
fn stores_nothing_else(
    #[case] call_type: &str,
    #[case] messages: Option<&str>,
    #[case] store_bodies: bool,
    #[case] expected: &str,
) {
    let source = messages.map_or_else(
        || format!(r#"{{"call_type": "{call_type}"}}"#),
        |stored| format!(r#"{{"call_type": "{call_type}", "messages": {stored}}}"#),
    );
    assert_eq!(
        storage_messages(Some(&value(&source)), store_bodies),
        expected
    );
}

#[test]
fn a_missing_payload_stores_nothing() {
    assert_eq!(storage_messages(None, true), "{}");
}

#[test]
fn strips_nul_bytes_from_stored_messages() {
    let payload = value(
        r#"{"call_type": "_arealtime", "messages": [{"role": "user", "content": "a\u0000b"}]}"#,
    );
    assert_eq!(
        storage_messages(Some(&payload), true),
        r#"[{"role": "user", "content": "ab"}]"#
    );
}
