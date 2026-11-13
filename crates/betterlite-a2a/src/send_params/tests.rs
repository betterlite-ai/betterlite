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
#[case::v10_raw_part_trailing_bits(
    r#"{"message": {"messageId": "m1", "parts": [{"raw": "aGl="}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "file", "file": {"bytes": "aGk="}}]}}"#
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
#[case::v03_history_length_integral_float(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": {"historyLength": 3.0}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"historyLength": 3}}"#
    )
)]
#[case::v03_history_length_large_integer(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": {"historyLength": 9007199254740993}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"historyLength": 9007199254740993}}"#
    )
)]
#[case::v03_duplicate_spellings(
    r#"{"message": {"message_id": "m1", "messageId": "m2", "role": "user", "parts": [{"text": "hi"}], "context_id": "c1", "contextId": "c2"}, "configuration": {"historyLength": 1, "history_length": 2}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m2", "role": "user", "parts": [{"kind": "text", "text": "hi"}], "contextId": "c2"}, "configuration": {"historyLength": 1}}"#
    )
)]
#[case::v10_role_numeric_string(
    r#"{"message": {"role": "1", "messageId": "m1", "parts": [{"text": "hi"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v10_history_length_max(
    r#"{"message": {"messageId": "m1"}, "configuration": {"historyLength": 2147483647}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": []}, "configuration": {"blocking": true, "historyLength": 2147483647}}"#
    )
)]
#[case::v10_history_length_over(
    r#"{"message": {"messageId": "m1"}, "configuration": {"historyLength": 2147483648}}"#,
    None
)]
#[case::v10_history_length_quoted_over(
    r#"{"message": {"messageId": "m1"}, "configuration": {"historyLength": "2147483648"}}"#,
    None
)]
#[case::v10_history_length_below_min(
    r#"{"message": {"messageId": "m1"}, "configuration": {"historyLength": -2147483649}}"#,
    None
)]
#[case::params_array_of_message(
    r#"[{"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}]"#,
    None
)]
#[case::v03_part_with_non_object_file(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi", "file": "report.pdf"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v03_part_with_file_array(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi", "file": ["aGk=", "text/plain"]}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v03_configuration_ignores_task_push_config(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": {"taskPushNotificationConfig": "x"}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {}}"#
    )
)]
#[case::v10_configuration_ignores_push_config(
    r#"{"message": {"role": "ROLE_USER", "messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {"pushNotificationConfig": "x"}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": true}}"#
    )
)]
#[case::v03_push_config_array(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": {"pushNotificationConfig": ["https://x"]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": true}}"#
    )
)]
#[case::v03_push_config_authentication_array(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": {"pushNotificationConfig": {"url": "https://x", "authentication": [["Bearer"]]}}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": true}}"#
    )
)]
#[case::v03_push_config_snake_case_array(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": {"push_notification_config": ["https://x"]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "hi"}]}, "configuration": {"blocking": true}}"#
    )
)]
#[case::v10_task_push_config_array(
    r#"{"message": {"role": "ROLE_USER", "messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {"taskPushNotificationConfig": ["https://x"]}}"#,
    None
)]
#[case::v10_task_push_config_snake_case_array(
    r#"{"message": {"role": "ROLE_USER", "messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {"task_push_notification_config": ["https://x"]}}"#,
    None
)]
#[case::v10_task_push_config_authentication_array(
    r#"{"message": {"role": "ROLE_USER", "messageId": "m1", "parts": [{"text": "hi"}]}, "configuration": {"taskPushNotificationConfig": {"url": "https://x", "authentication": ["Bearer"]}}}"#,
    None
)]
#[case::v10_part_ignores_non_object_file(
    r#"{"message": {"role": "ROLE_USER", "messageId": "m1", "parts": [{"text": "hi", "file": "x"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}}"#
    )
)]
#[case::v03_configuration_array(
    r#"{"message": {"messageId": "m1", "role": "user", "parts": [{"text": "hi"}]}, "configuration": [["text"]]}"#,
    None
)]
#[case::v10_null_data_then_text(
    r#"{"message": {"messageId": "m1", "parts": [{"data": null, "text": "a"}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "a"}]}}"#
    )
)]
#[case::v10_text_then_null_data(
    r#"{"message": {"messageId": "m1", "parts": [{"text": "a", "data": null}]}}"#,
    Some(
        r#"{"message": {"kind": "message", "messageId": "m1", "role": "agent", "parts": [{"kind": "text", "text": "a"}]}}"#
    )
)]
#[case::v03_snake_case_every_key(
    r#"{
        "message": {
            "message_id": "m1",
            "role": "user",
            "parts": [
                {"text": "hi"},
                {"kind": "file", "file": {"bytes": "aGk=", "mime_type": "text/plain"}},
                {"kind": "file", "file": {"uri": "https://f", "mime_type": "image/png"}}
            ],
            "context_id": "c1",
            "task_id": "t1",
            "reference_task_ids": ["r1"],
            "extensions": ["e1"],
            "metadata": {"k": 1}
        },
        "configuration": {
            "accepted_output_modes": ["text"],
            "blocking": false,
            "history_length": 3,
            "push_notification_config": {
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
                "parts": [
                    {"kind": "text", "text": "hi"},
                    {"kind": "file", "file": {"bytes": "aGk=", "mimeType": "text/plain"}},
                    {"kind": "file", "file": {"uri": "https://f", "mimeType": "image/png"}}
                ],
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
#[case::v10_snake_case_every_key(
    r#"{
        "message": {
            "message_id": "m1",
            "context_id": "c1",
            "task_id": "t1",
            "role": "ROLE_USER",
            "parts": [
                {"text": "hi"},
                {"raw": "aGk=", "media_type": "text/plain"}
            ],
            "metadata": {"k": 1},
            "extensions": ["e1"],
            "reference_task_ids": ["r1"]
        },
        "configuration": {
            "accepted_output_modes": ["text"],
            "task_push_notification_config": {
                "url": "https://p",
                "id": "p1",
                "token": "tok",
                "authentication": {"scheme": "Bearer", "credentials": "cred"}
            },
            "history_length": 3,
            "return_immediately": true
        }
    }"#,
    Some(
        r#"{
            "message": {
                "kind": "message",
                "messageId": "m1",
                "role": "user",
                "parts": [
                    {"kind": "text", "text": "hi"},
                    {"kind": "file", "file": {"bytes": "aGk=", "mimeType": "text/plain"}}
                ],
                "contextId": "c1",
                "taskId": "t1",
                "referenceTaskIds": ["r1"],
                "metadata": {"k": 1.0},
                "extensions": ["e1"]
            },
            "configuration": {
                "acceptedOutputModes": ["text"],
                "blocking": false,
                "historyLength": 3,
                "pushNotificationConfig": {
                    "url": "https://p",
                    "id": "p1",
                    "token": "tok",
                    "authentication": {"schemes": ["Bearer"], "credentials": "cred"}
                }
            }
        }"#
    )
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

#[test]
fn send_params_errors_name_the_failing_part() {
    let missing = build_message_send_params(&json(
        r#"{"message": {"messageId": "m1", "parts": [{"mediaType": "text/plain"}]}}"#,
    ))
    .unwrap_err();
    assert!(
        missing.to_string().contains("carries no content"),
        "{missing}"
    );
    let multiple = build_message_send_params(&json(
        r#"{"message": {"messageId": "m1", "parts": [{"text": "a", "url": "u"}]}}"#,
    ))
    .unwrap_err();
    assert!(
        multiple.to_string().contains("more than one content key"),
        "{multiple}"
    );
}
