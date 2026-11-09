#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use serde_json::{from_str, to_value};

use super::build_message_send_params;

fn json(text: &str) -> AgentJson {
    from_str(text).expect("valid JSON")
}

#[rstest]
#[case::v03_camel_case(
    r#"{
        "message": {
            "messageId": "m1",
            "role": "user",
            "parts": [{"kind": "text", "text": "hi"}],
            "contextId": "c1",
            "taskId": "t1",
            "referenceTaskIds": ["r1"],
            "extensions": ["e1"],
            "metadata": {"k": 1}
        },
        "configuration": {
            "acceptedOutputModes": ["text"],
            "blocking": false,
            "historyLength": 3,
            "pushNotificationConfig": {
                "url": "https://x",
                "id": "p1",
                "token": "tok",
                "authentication": {"schemes": ["Bearer"], "credentials": "cred"}
            }
        },
        "metadata": {"a": 1}
    }"#,
    Some(
        r#"{
            "message": {
                "kind": "message",
                "messageId": "m1",
                "role": "user",
                "parts": [{"kind": "text", "text": "hi"}],
                "contextId": "c1",
                "taskId": "t1",
                "referenceTaskIds": ["r1"],
                "metadata": {"k": 1},
                "extensions": ["e1"]
            },
            "configuration": {
                "acceptedOutputModes": ["text"],
                "blocking": false,
                "historyLength": 3,
                "pushNotificationConfig": {
                    "url": "https://x",
                    "id": "p1",
                    "token": "tok",
                    "authentication": {"schemes": ["Bearer"], "credentials": "cred"}
                }
            },
            "metadata": {"a": 1}
        }"#
    )
)]
#[case::v03_snake_case_and_extra_keys(
    r#"{"message": {"message_id": "m1", "role": "agent", "parts": [{"text": "hi", "extra": 1}], "unknown": true}, "unknown": "x"}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v03_file_part_bytes_and_uri(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [
        {"kind": "file", "file": {"bytes": "aGk=", "mime_type": "text/plain", "name": "hi.txt"}},
        {"kind": "file", "file": {"uri": "https://f"}, "metadata": {"z": 2}},
        {"kind": "data", "data": {"n": 5}}
    ]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [
            {"kind": "file", "file": {"bytes": "aGk=", "mimeType": "text/plain", "name": "hi.txt"}},
            {"kind": "file", "file": {"uri": "https://f"}, "metadata": {"z": 2}},
            {"kind": "data", "data": {"n": 5}}
        ]}}"#
    )
)]
#[case::v03_invalid_role_no_fallback_match(
    r#"{"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"kind": "text"}]}}"#,
    None
)]
#[case::v03_number_for_string(
    r#"{"message": {"messageId": 5, "role": "user", "parts": [{"text": "hi"}]}}"#,
    None
)]
#[case::v03_part_without_content(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"kind": "text"}]}}"#,
    None
)]
#[case::v10_text_message(
    r#"{"message": {"role": "ROLE_USER", "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_role_unspecified(
    r#"{"message": {"role": "ROLE_UNSPECIFIED", "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_role_agent_string(
    r#"{"message": {"role": "ROLE_AGENT", "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_role_user_integer(
    r#"{"message": {"role": 1, "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_role_unknown_integer(
    r#"{"message": {"role": 7, "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_unknown_role_string(
    r#"{"message": {"role": "ROLE_BOGUS", "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_message_absent(
    "{}",
    Some(r#"{"message": {"kind": "message", "messageId": "", "role": "agent", "parts": []}}"#)
)]
#[case::v10_data_part_object_and_scalar(
    r#"{"message": {"messageId": "m1", "parts": [
        {"data": {"a": 1}},
        {"data": "scalar", "metadata": {"x": 1}},
        {"data": null}
    ]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [
            {"kind": "data", "data": {"a": 1.0}},
            {"kind": "data", "data": {"value": "scalar"}, "metadata": {"x": 1.0, "data_part_compat": true}},
            {"kind": "data", "data": {"value": null}, "metadata": {"data_part_compat": true}}
        ]}}"#
    )
)]
#[case::v10_raw_and_url_parts(
    r#"{"message": {"messageId": "m1", "parts": [
        {"raw": "aGk=", "mediaType": "text/plain", "filename": "hi.txt"},
        {"url": "https://f", "mediaType": "image/png", "filename": ""},
        {"raw": "-_8"}
    ]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [
            {"kind": "file", "file": {"bytes": "aGk=", "mimeType": "text/plain", "name": "hi.txt"}},
            {"kind": "file", "file": {"uri": "https://f", "mimeType": "image/png"}},
            {"kind": "file", "file": {"bytes": "+/8="}}
        ]}}"#
    )
)]
#[case::v10_part_without_content(
    r#"{"message": {"messageId": "m1", "parts": [{"mediaType": "text/plain"}]}}"#,
    None
)]
#[case::v10_part_with_two_contents(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "a", "url": "u"}]}}"#,
    None
)]
#[case::v10_bad_base64(r#"{"message": {"messageId": "m1", "parts": [{"raw": "a"}]}}"#, None)]
#[case::v10_configuration_blocking(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {"acceptedOutputModes": [], "historyLength": 0, "returnImmediately": true}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": false, "historyLength": 0}}"#
    )
)]
#[case::v10_configuration_empty_object(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": true}}"#
    )
)]
#[case::v10_push_config_authentication(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {"taskPushNotificationConfig": {"url": "", "id": "p1", "token": "", "authentication": {"scheme": "Bearer", "credentials": "cred"}}}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": true, "pushNotificationConfig": {"url": "", "id": "p1", "authentication": {"schemes": ["Bearer"], "credentials": "cred"}}}}"#
    )
)]
#[case::v10_empty_ids_and_metadata(
    r#"{"message": {"messageId": "m1", "contextId": "", "taskId": "", "referenceTaskIds": [], "extensions": [], "parts": [{"text": "hi"}], "metadata": {}}, "metadata": {}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "metadata": {}}"#
    )
)]
#[case::v10_message_metadata(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "hi"}], "metadata": {"n": 5}}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}], "metadata": {"n": 5.0}}}"#
    )
)]
#[case::v10_text_part_metadata_empty(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "hi", "metadata": {}}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi", "metadata": {}}]}}"#
    )
)]
#[case::v10_history_length_string(
    r#"{"message": {"messageId": "m1"}, "configuration": {"historyLength": "3"}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": []}, "configuration": {"blocking": true, "historyLength": 3}}"#
    )
)]
#[case::v10_history_length_fractional(
    r#"{"message": {"messageId": "m1"}, "configuration": {"historyLength": 3.5}}"#,
    None
)]
#[case::params_not_an_object("[1, 2]", None)]
fn send_params_tests_behavior(#[case] input: &str, #[case] want: Option<&str>) {
    match (build_message_send_params(&json(input)), want) {
        (Ok(parsed), Some(text)) => {
            assert_eq!(to_value(parsed).expect("serialized params"), json(text));
        }
        (Err(error), None) => assert!(
            error
                .to_string()
                .starts_with("Invalid message/send params: "),
            "{error}"
        ),
        (Ok(parsed), None) => panic!("expected an error, parsed {parsed:?}"),
        (Err(error), Some(text)) => panic!("expected {text}, got {error}"),
    }
}
