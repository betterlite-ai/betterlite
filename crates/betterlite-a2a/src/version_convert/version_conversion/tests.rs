#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use serde_json::{from_str, to_value};

use crate::agent_card::ProtocolVersion;
use crate::version_convert::version_conversion::detect;
use crate::version_convert::version_conversion::request::map_task_state;
use crate::version_convert::{
    JsonObject, normalize_agent_card, normalize_jsonrpc_response, normalize_request_params,
    normalize_stream_event, request_id,
};

fn object(text: &str) -> JsonObject {
    from_str(text).expect("a JSON object")
}

fn value(text: &str) -> AgentJson {
    from_str(text).expect("a JSON value")
}

fn text_value(text: &str) -> AgentJson {
    AgentJson::String(text.to_owned())
}

fn number_value(number: i64) -> AgentJson {
    AgentJson::from(number)
}

fn result_of(converted: &JsonObject) -> &AgentJson {
    field(converted, "result")
}

fn field<'object>(object: &'object JsonObject, key: &str) -> &'object AgentJson {
    object.get(key).expect("a present member")
}

fn member<'value>(value: &'value AgentJson, key: &str) -> &'value AgentJson {
    value.get(key).expect("a present member")
}

#[rstest]
#[case::kind_field(r#"{"kind":"task","id":"t"}"#, Some(ProtocolVersion::V0_3))]
#[case::message_envelope(r#"{"message":{"role":"ROLE_AGENT"}}"#, Some(ProtocolVersion::V1_0))]
#[case::task_envelope(r#"{"task":{"id":"t"}}"#, Some(ProtocolVersion::V1_0))]
#[case::unknown_shape(r#"{"id":"t"}"#, None)]
fn send_result_detection(#[case] text: &str, #[case] expected: Option<ProtocolVersion>) {
    assert_eq!(
        detect::result_version(&value(text), &detect::SEND_ENVELOPE_KEYS),
        expected
    );
}

#[rstest]
#[case::status_update(r#"{"statusUpdate":{"taskId":"t"}}"#, Some(ProtocolVersion::V1_0))]
#[case::artifact_update(r#"{"artifactUpdate":{}}"#, Some(ProtocolVersion::V1_0))]
#[case::kind_wins(r#"{"kind":"message","task":{}}"#, Some(ProtocolVersion::V0_3))]
#[case::empty("{}", None)]
#[case::unrelated_keys(r#"{"id":"t","jsonrpc":"2.0"}"#, None)]
fn stream_result_detection(#[case] text: &str, #[case] expected: Option<ProtocolVersion>) {
    assert_eq!(
        detect::result_version(&value(text), &detect::STREAM_ENVELOPE_KEYS),
        expected
    );
}

#[rstest]
#[case::first_kind(r#"{"tasks":[{"kind":"task"}]}"#, Some(ProtocolVersion::V0_3))]
#[case::first_bare(r#"{"tasks":[{"id":"t"}]}"#, Some(ProtocolVersion::V1_0))]
#[case::empty_list(r#"{"tasks":[]}"#, None)]
#[case::first_not_object(r#"{"tasks":["t"]}"#, None)]
#[case::no_tasks(r#"{"nextPageToken":"n"}"#, None)]
fn list_tasks_detection(#[case] text: &str, #[case] expected: Option<ProtocolVersion>) {
    assert_eq!(detect::list_tasks_version(&value(text)), expected);
}

#[rstest]
#[case::protocol_version_wins(
    r#"{"protocolVersion":"0.3","supportedInterfaces":[]}"#,
    ProtocolVersion::V0_3
)]
#[case::semver_protocol_version(r#"{"protocolVersion":"1.0.0"}"#, ProtocolVersion::V1_0)]
#[case::interfaces_only(r#"{"supportedInterfaces":[]}"#, ProtocolVersion::V1_0)]
#[case::neither_signal(r#"{"name":"agent"}"#, ProtocolVersion::V0_3)]
#[case::unrecognized_version(r#"{"protocolVersion":"2.0"}"#, ProtocolVersion::V0_3)]
fn card_detection(#[case] text: &str, #[case] expected: ProtocolVersion) {
    assert_eq!(detect::card_version(&object(text)), expected);
}

#[test]
fn converts_a_03_message_result_to_10() {
    let content = object(
        r#"{"jsonrpc":"2.0","id":1,"result":{"kind":"message","messageId":"m1","role":"agent","parts":[{"kind":"text","text":"hi"}]}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V1_0, "message/send");
    let message = member(result_of(&converted), "message");
    assert_eq!(member(message, "messageId"), &text_value("m1"));
    assert_eq!(member(message, "role"), &text_value("ROLE_AGENT"));
    assert_eq!(member(message, "parts"), &value(r#"[{"text": "hi"}]"#));
    assert_eq!(field(&converted, "id"), &number_value(1));
}

#[test]
fn converts_a_10_task_result_to_03() {
    let content = object(
        r#"{"jsonrpc":"2.0","id":"1","result":{"task":{"id":"t1","contextId":"c1","status":{"state":"TASK_STATE_WORKING"}}}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V0_3, "message/send");
    let result = result_of(&converted);
    assert_eq!(member(result, "kind"), &text_value("task"));
    assert_eq!(member(result, "id"), &text_value("t1"));
    assert_eq!(member(result, "contextId"), &text_value("c1"));
    assert_eq!(
        member(member(result, "status"), "state"),
        &text_value("working")
    );
    let status = member(result, "status").as_object().expect("status");
    assert!(!status.contains_key("message"));
    assert!(!status.contains_key("timestamp"));
    assert!(!result.as_object().expect("task").contains_key("history"));
}

#[test]
fn converts_a_10_task_result_to_03_for_get() {
    let content = object(
        r#"{"jsonrpc":"2.0","id":"1","result":{"id":"t1","contextId":"c1","status":{"state":"TASK_STATE_AUTH_REQUIRED"},"history":[{"messageId":"m1","role":"ROLE_AGENT","parts":[{"text":"hi"}]}]}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V0_3, "tasks/get");
    let result = result_of(&converted);
    assert_eq!(member(result, "kind"), &text_value("task"));
    assert_eq!(
        member(member(result, "status"), "state"),
        &text_value("auth-required")
    );
    let history = member(result, "history").as_array().expect("history");
    assert_eq!(member(&history[0], "role"), &text_value("agent"));
    assert_eq!(
        member(&history[0], "parts"),
        &value(r#"[{"kind": "text", "text": "hi"}]"#)
    );
}

#[test]
fn converts_a_10_message_result_to_03() {
    let content = object(
        r#"{"jsonrpc":"2.0","id":"1","result":{"message":{"messageId":"m1","role":"ROLE_USER","parts":[{"text":"hi"}]}}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V0_3, "message/send");
    let result = result_of(&converted);
    assert_eq!(member(result, "kind"), &text_value("message"));
    assert_eq!(member(result, "messageId"), &text_value("m1"));
    assert_eq!(member(result, "role"), &text_value("user"));
    assert_eq!(
        member(result, "parts"),
        &value(r#"[{"kind": "text", "text": "hi"}]"#)
    );
}

#[test]
fn passes_an_error_response_through_untouched() {
    let content = object(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no"}}"#);
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V1_0, "message/send"),
        content
    );
}

#[rstest]
#[case::text(r#"{"jsonrpc":"2.0","id":1,"result":"done"}"#)]
#[case::array(r#"{"jsonrpc":"2.0","id":1,"result":[1,2]}"#)]
#[case::null(r#"{"jsonrpc":"2.0","id":1,"result":null}"#)]
fn passes_a_non_object_result_through(#[case] text: &str) {
    let content = object(text);
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V1_0, "message/send"),
        content
    );
}

#[test]
fn leaves_an_unknown_method_result_unchanged() {
    let content = object(r#"{"jsonrpc":"2.0","result":{"kind":"task","id":"t1"}}"#);
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V1_0, "tasks/resubscribe"),
        content
    );
}

#[test]
fn leaves_an_unrecognized_send_shape_unchanged() {
    let content = object(r#"{"jsonrpc":"2.0","result":{"id":"t1"}}"#);
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V1_0, "message/send"),
        content
    );
}

#[test]
fn keeps_a_send_result_that_cannot_be_converted() {
    let content = object(r#"{"jsonrpc":"2.0","result":{"kind":"message","messageId":"m1"}}"#);
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V1_0, "message/send"),
        content
    );
}

#[test]
fn converts_a_03_task_result_to_10() {
    let content = object(
        r#"{"jsonrpc":"2.0","result":{"kind":"task","id":"t1","contextId":"c1","status":{"state":"input-required"},"history":[{"kind":"message","messageId":"m1","role":"user","parts":[{"kind":"text","text":"hi"}]}]}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V1_0, "tasks/get");
    let task = result_of(&converted);
    assert!(task.get("task").is_none());
    assert_eq!(member(task, "id"), &text_value("t1"));
    assert_eq!(member(task, "contextId"), &text_value("c1"));
    assert_eq!(
        member(member(task, "status"), "state"),
        &text_value("TASK_STATE_INPUT_REQUIRED")
    );
    let history = member(task, "history").as_array().expect("history");
    assert_eq!(member(&history[0], "role"), &text_value("ROLE_USER"));
    assert_eq!(member(&history[0], "parts"), &value(r#"[{"text": "hi"}]"#));
}

#[test]
fn leaves_a_task_already_in_the_target_version_unchanged() {
    let content = object(r#"{"result":{"kind":"task","id":"t1","contextId":"c1"}}"#);
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V0_3, "tasks/get"),
        content
    );
}

#[test]
fn converts_a_03_task_list_to_10_and_keeps_loose_items() {
    let content = object(
        r#"{"result":{"tasks":[{"kind":"task","id":"t1","contextId":"c1","status":{"state":"working"}},"loose"],"nextPageToken":"n"}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V1_0, "tasks/list");
    let result = result_of(&converted);
    assert_eq!(member(result, "nextPageToken"), &text_value("n"));
    let tasks = member(result, "tasks").as_array().expect("tasks");
    assert_eq!(member(&tasks[0], "id"), &text_value("t1"));
    assert_eq!(
        member(member(&tasks[0], "status"), "state"),
        &text_value("TASK_STATE_WORKING")
    );
    assert_eq!(tasks[1], text_value("loose"));
}

#[test]
fn keeps_the_whole_task_list_when_one_task_cannot_be_converted() {
    let content = object(
        r#"{"result":{"tasks":[{"kind":"task","id":"t1","contextId":"c1","status":{"state":"working"}},{"kind":"task","id":"t2"}]}}"#,
    );
    assert_eq!(
        normalize_jsonrpc_response(content.clone(), ProtocolVersion::V1_0, "tasks/list"),
        content
    );
}

#[test]
fn converts_a_10_task_list_to_03() {
    let content = object(
        r#"{"result":{"tasks":[{"id":"t1","contextId":"c1","status":{"state":"TASK_STATE_COMPLETED"}}]}}"#,
    );
    let converted = normalize_jsonrpc_response(content, ProtocolVersion::V0_3, "tasks/list");
    let tasks = member(result_of(&converted), "tasks")
        .as_array()
        .expect("tasks");
    assert_eq!(member(&tasks[0], "kind"), &text_value("task"));
    assert_eq!(
        member(member(&tasks[0], "status"), "state"),
        &text_value("completed")
    );
}

#[test]
fn converts_a_03_status_update_event_to_10() {
    let event = object(
        r#"{"jsonrpc":"2.0","result":{"kind":"status-update","taskId":"t1","contextId":"c1","status":{"state":"working"},"final":false}}"#,
    );
    let converted = normalize_stream_event(event, ProtocolVersion::V1_0, None);
    let update = member(result_of(&converted), "statusUpdate");
    assert_eq!(member(update, "taskId"), &text_value("t1"));
    assert_eq!(member(update, "contextId"), &text_value("c1"));
    assert_eq!(
        member(member(update, "status"), "state"),
        &text_value("TASK_STATE_WORKING")
    );
}

#[test]
fn converts_a_10_artifact_update_event_to_03() {
    let event = object(
        r#"{"jsonrpc":"2.0","result":{"artifactUpdate":{"taskId":"t1","contextId":"c1","artifact":{"artifactId":"a1","parts":[{"text":"hi"}]},"append":true}}}"#,
    );
    let converted = normalize_stream_event(event, ProtocolVersion::V0_3, None);
    let update = result_of(&converted);
    assert_eq!(member(update, "kind"), &text_value("artifact-update"));
    assert_eq!(member(update, "taskId"), &text_value("t1"));
    assert_eq!(member(update, "append"), &AgentJson::Bool(true));
    assert_eq!(
        member(member(update, "artifact"), "parts"),
        &value(r#"[{"kind": "text", "text": "hi"}]"#)
    );
}

#[test]
fn converts_a_03_artifact_update_event_to_10() {
    let event = object(
        r#"{"jsonrpc":"2.0","result":{"kind":"artifact-update","taskId":"t1","contextId":"c1","artifact":{"artifactId":"a1","parts":[{"kind":"text","text":"hi"}]},"append":false,"lastChunk":false}}"#,
    );
    let converted = normalize_stream_event(event, ProtocolVersion::V1_0, None);
    let update = member(result_of(&converted), "artifactUpdate");
    assert_eq!(member(update, "taskId"), &text_value("t1"));
    assert!(update.get("append").is_none());
    assert!(update.get("lastChunk").is_none());
}

#[test]
fn passes_an_error_stream_event_through_untouched() {
    let event = object(r#"{"jsonrpc":"2.0","error":{"code":-1,"message":"no"}}"#);
    assert_eq!(
        normalize_stream_event(event.clone(), ProtocolVersion::V1_0, None),
        event
    );
}

#[test]
fn keeps_a_stream_event_whose_envelope_cannot_be_converted() {
    let event = object(r#"{"jsonrpc":"2.0","result":{"task":{"id":"t1","status":"broken"}}}"#);
    assert_eq!(
        normalize_stream_event(event.clone(), ProtocolVersion::V0_3, None),
        event
    );
}

#[rstest]
#[case::get(
    "tasks/get",
    r#"{"id":"t1","historyLength":4}"#,
    r#"{"id":"t1","historyLength":4}"#
)]
#[case::get_without_history("tasks/get", r#"{"id":"t1"}"#, r#"{"id":"t1"}"#)]
#[case::get_keeps_zero_history(
    "tasks/get",
    r#"{"id":"t1","historyLength":0}"#,
    r#"{"id":"t1","historyLength":0}"#
)]
#[case::cancel(
    "tasks/cancel",
    r#"{"id":"t1","metadata":{"k":"v"}}"#,
    r#"{"id":"t1","metadata":{"k":"v"}}"#
)]
#[case::cancel_defaults_id("tasks/cancel", "{}", r#"{"id":""}"#)]
#[case::resubscribe("tasks/resubscribe", r#"{"id":"t1","tenant":""}"#, r#"{"id":"t1"}"#)]
#[case::set_from_flat_fields(
    "tasks/pushNotificationConfig/set",
    r#"{"taskId":"T1","url":"https://flat","id":"c1"}"#,
    r#"{"taskId":"T1","pushNotificationConfig":{"url":"https://flat","id":"c1"}}"#
)]
#[case::set_accepts_a_non_object_config(
    "tasks/pushNotificationConfig/set",
    r#"{"config":"x","taskId":"T2","url":"https://flat"}"#,
    r#"{"taskId":"T2","pushNotificationConfig":{"url":"https://flat"}}"#
)]
#[case::get_from_string_integer(
    "tasks/get",
    r#"{"id":"t","historyLength":"4"}"#,
    r#"{"id":"t","historyLength":4}"#
)]
#[case::get_from_float_integer(
    "tasks/get",
    r#"{"id":"t","historyLength":4.0}"#,
    r#"{"id":"t","historyLength":4}"#
)]
#[case::get_from_float_string(
    "tasks/get",
    r#"{"id":"t","historyLength":"4.0"}"#,
    r#"{"id":"t","historyLength":4}"#
)]
#[case::get_from_spaced_string(
    "tasks/get",
    r#"{"id":"t","historyLength":" 4","tenant":"x"}"#,
    r#"{"id":"t","historyLength":" 4","tenant":"x"}"#
)]
#[case::set_from_parent(
    "tasks/pushNotificationConfig/set",
    r#"{"parent":"tasks/T1/config","configId":"c1","config":{"url":"https://cb","token":"tok"}}"#,
    r#"{"taskId":"T1","pushNotificationConfig":{"url":"https://cb","id":"c1","token":"tok"}}"#
)]
#[case::set_from_push_configuration(
    "tasks/pushNotificationConfig/set",
    r#"{"pushNotificationConfig":{"url":"https://cb"},"taskId":"T2"}"#,
    r#"{"taskId":"T2","pushNotificationConfig":{"url":"https://cb"}}"#
)]
#[case::set_keeps_authentication(
    "tasks/pushNotificationConfig/set",
    r#"{"config":{"url":"https://cb","authentication":{"scheme":"bearer","credentials":"c"}},"taskId":"T3"}"#,
    r#"{"taskId":"T3","pushNotificationConfig":{"url":"https://cb","authentication":{"schemes":["bearer"],"credentials":"c"}}}"#
)]
#[case::get_config(
    "tasks/pushNotificationConfig/get",
    r#"{"taskId":"T1","id":"c1"}"#,
    r#"{"id":"T1","pushNotificationConfigId":"c1"}"#
)]
#[case::get_config_without_id(
    "tasks/pushNotificationConfig/get",
    r#"{"taskId":"T1"}"#,
    r#"{"id":"T1"}"#
)]
#[case::list_configs(
    "tasks/pushNotificationConfig/list",
    r#"{"taskId":"T1","pageSize":5}"#,
    r#"{"id":"T1"}"#
)]
#[case::delete_config(
    "tasks/pushNotificationConfig/delete",
    r#"{"taskId":"T1","id":"c1"}"#,
    r#"{"id":"T1","pushNotificationConfigId":"c1"}"#
)]
fn lowers_request_params(#[case] method: &str, #[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        normalize_request_params(object(input), ProtocolVersion::V1_0, method),
        object(expected)
    );
}

#[rstest]
#[case::status(
    r#"{"contextId":"c","status":"TASK_STATE_WORKING","pageSize":0,"includeArtifacts":true}"#,
    r#"{"contextId":"c","status":"working","pageSize":0,"includeArtifacts":true}"#
)]
#[case::include_artifacts_false(r#"{"includeArtifacts":false}"#, r#"{"includeArtifacts":false}"#)]
#[case::string_page_size(r#"{"pageSize":"5"}"#, r#"{"pageSize":5}"#)]
#[case::non_proto_status_name(r#"{"status":"working"}"#, "{}")]
#[case::numeric_status(r#"{"status":2}"#, r#"{"status":"working"}"#)]
#[case::numeric_string_status(r#"{"status":"2"}"#, r#"{"status":"working"}"#)]
#[case::float_status(r#"{"status":2.0}"#, r#"{"status":"working"}"#)]
#[case::numeric_string_float_status(r#"{"status":"2.0","tenant":"x"}"#, r#"{"tenant":"x"}"#)]
#[case::spaced_numeric_status(r#"{"status":" 2"}"#, r#"{"status":"working"}"#)]
#[case::truncated_float_status(r#"{"status":2.5}"#, r#"{"status":"working"}"#)]
#[case::unmapped_status(r#"{"status":"TASK_STATE_BOGUS","tenant":"x"}"#, r#"{"tenant":"x"}"#)]
#[case::history_and_timestamp(
    r#"{"historyLength":3,"statusTimestampAfter":"2026-01-01T00:00:00Z"}"#,
    r#"{"historyLength":3,"statusTimestampAfter":"2026-01-01T00:00:00Z"}"#
)]
fn lowers_list_tasks_params(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        normalize_request_params(object(input), ProtocolVersion::V1_0, "tasks/list"),
        object(expected)
    );
}

#[test]
fn leaves_params_untouched_when_served_03() {
    let params = object(r#"{"id":"t1","historyLength":4}"#);
    assert_eq!(
        normalize_request_params(params.clone(), ProtocolVersion::V0_3, "tasks/get"),
        params
    );
}

#[test]
fn leaves_params_untouched_for_an_unhandled_method() {
    let params = object(r#"{"id":"t1"}"#);
    assert_eq!(
        normalize_request_params(params.clone(), ProtocolVersion::V1_0, "message/send"),
        params
    );
}

#[rstest]
#[case::submitted("TASK_STATE_SUBMITTED", Some("submitted"))]
#[case::working("TASK_STATE_WORKING", Some("working"))]
#[case::input_required("TASK_STATE_INPUT_REQUIRED", Some("input-required"))]
#[case::auth_required("TASK_STATE_AUTH_REQUIRED", Some("auth-required"))]
#[case::unspecified("TASK_STATE_UNSPECIFIED", None)]
#[case::unrecognized("TASK_STATE_BOGUS", None)]
#[case::already_lowercased("working", Some("working"))]
fn maps_proto_task_states(#[case] name: &str, #[case] expected: Option<&str>) {
    assert_eq!(map_task_state(name).as_deref(), expected);
}

#[test]
fn upgrades_a_03_card_to_the_10_shape() {
    let card = object(
        r#"{"name":"agent","description":"d","version":"1.2.3","url":"https://agent","preferredTransport":"JSONRPC","protocolVersion":"0.3","capabilities":{"streaming":true,"extensions":[{"uri":"urn:x"}]},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[{"id":"chat","name":"Chat","description":"chat","tags":["chat"],"security":[{"k":[]}]}],"security":[{"k":[]}],"securitySchemes":{"LiteLLMKey":{"type":"http","scheme":"bearer"}},"provider":{"organization":"o","url":"https://p"},"signatures":[{"protected":"p","signature":"s"}],"xUpstreamHint":{"kept":true}}"#,
    );
    let converted = normalize_agent_card(card, ProtocolVersion::V1_0);
    assert_eq!(
        field(&converted, "xUpstreamHint"),
        &value(r#"{"kept":true}"#)
    );
    assert_eq!(field(&converted, "name"), &text_value("agent"));
    assert_eq!(
        field(&converted, "supportedInterfaces"),
        &value(r#"[{"url":"https://agent","protocolBinding":"JSONRPC","protocolVersion":"0.3"}]"#)
    );
    assert!(!converted.contains_key("url"));
    assert!(!converted.contains_key("protocolVersion"));
    assert!(!converted.contains_key("security"));
    let capabilities = field(&converted, "capabilities");
    assert_eq!(member(capabilities, "streaming"), &AgentJson::Bool(true));
    assert_eq!(
        member(capabilities, "extensions"),
        &value(r#"[{"uri":"urn:x"}]"#)
    );
    assert_eq!(
        field(&converted, "securityRequirements"),
        &value(r#"[{"k":[]}]"#)
    );
    assert_eq!(
        member(field(&converted, "securitySchemes"), "LiteLLMKey"),
        &value(r#"{"type":"http","scheme":"bearer"}"#)
    );
    let skills = field(&converted, "skills").as_array().expect("skills");
    assert_eq!(member(&skills[0], "security"), &value(r#"[{"k":[]}]"#));
}

#[test]
fn keeps_a_card_that_lacks_required_03_members() {
    let card = object(r#"{"name":"agent","url":"https://a","capabilities":{}}"#);
    assert_eq!(
        normalize_agent_card(card.clone(), ProtocolVersion::V1_0),
        card
    );
}

#[rstest]
#[case::skill_missing_members(
    r#"{"name":"agent","description":"d","version":"1","url":"https://a","capabilities":{},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[{"id":"s","name":"n"}]}"#
)]
#[case::provider_missing_url(
    r#"{"name":"agent","description":"d","version":"1","url":"https://a","capabilities":{},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"provider":{"organization":"o"}}"#
)]
#[case::extension_missing_uri(
    r#"{"name":"agent","description":"d","version":"1","url":"https://a","capabilities":{"extensions":[{}]},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[]}"#
)]
#[case::signature_missing_signature(
    r#"{"name":"agent","description":"d","version":"1","url":"https://a","capabilities":{},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"signatures":[{"protected":"p"}]}"#
)]
#[case::additional_interface_missing_transport(
    r#"{"name":"agent","description":"d","version":"1","url":"https://a","capabilities":{},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"additionalInterfaces":[{"url":"https://b"}]}"#
)]
fn keeps_a_card_whose_nested_records_lack_required_members(#[case] text: &str) {
    let card = object(text);
    assert_eq!(
        normalize_agent_card(card.clone(), ProtocolVersion::V1_0),
        card
    );
}

#[test]
fn omits_empty_repeated_fields_from_the_upgraded_card() {
    let card = object(
        r#"{"name":"agent","description":"d","version":"1","url":"https://a","capabilities":{"streaming":true,"extensions":[]},"defaultInputModes":[],"defaultOutputModes":[],"skills":[],"security":[],"signatures":[]}"#,
    );
    let converted = normalize_agent_card(card, ProtocolVersion::V1_0);
    let interfaces = field(&converted, "supportedInterfaces")
        .as_array()
        .expect("interfaces");
    assert_eq!(
        member(&interfaces[0], "protocolVersion"),
        &text_value("0.3.0")
    );
    assert!(!converted.contains_key("defaultInputModes"));
    assert!(!converted.contains_key("defaultOutputModes"));
    assert!(!converted.contains_key("skills"));
    assert!(!converted.contains_key("securityRequirements"));
    assert!(!converted.contains_key("signatures"));
    let capabilities = field(&converted, "capabilities");
    assert!(capabilities.get("extensions").is_none());
}

#[test]
fn downgrades_a_10_card_and_strips_additional_interfaces() {
    let card = object(
        r#"{"name":"agent","description":"d","version":"1.2.3","supportedInterfaces":[{"url":"https://agent","protocolBinding":"JSONRPC","protocolVersion":"0.3"},{"url":"https://other","protocolBinding":"GRPC","protocolVersion":"1.0"}],"additionalInterfaces":[{"url":"https://backend","transport":"JSONRPC"}],"capabilities":{"streaming":true,"extendedAgentCard":true},"securityRequirements":[{"k":[]}],"skills":[{"id":"chat","name":"Chat","description":"chat","tags":["chat"],"securityRequirements":[{"k":[]}]}],"xUpstreamHint":{"secret":"s"}}"#,
    );
    let converted = normalize_agent_card(card, ProtocolVersion::V0_3);
    assert!(!converted.contains_key("xUpstreamHint"));
    assert_eq!(field(&converted, "url"), &text_value("https://agent"));
    assert_eq!(
        field(&converted, "preferredTransport"),
        &text_value("JSONRPC")
    );
    assert_eq!(field(&converted, "protocolVersion"), &text_value("0.3"));
    assert!(!converted.contains_key("supportedInterfaces"));
    assert!(!converted.contains_key("additionalInterfaces"));
    assert!(!converted.contains_key("securityRequirements"));
    assert_eq!(field(&converted, "security"), &value(r#"[{"k":[]}]"#));
    assert_eq!(
        field(&converted, "supportsAuthenticatedExtendedCard"),
        &AgentJson::Bool(true)
    );
    let capabilities = field(&converted, "capabilities")
        .as_object()
        .expect("capabilities");
    assert!(!capabilities.contains_key("extendedAgentCard"));
    let skills = field(&converted, "skills").as_array().expect("skills");
    assert_eq!(
        member(&skills[0], "securityRequirements"),
        &value(r#"[{"k":[]}]"#)
    );
}

#[test]
fn keeps_a_card_whose_downgrade_has_no_legacy_interface() {
    let card = object(
        r#"{"name":"agent","supportedInterfaces":[{"url":"https://agent","protocolBinding":"JSONRPC","protocolVersion":"1.0"}]}"#,
    );
    assert_eq!(
        normalize_agent_card(card.clone(), ProtocolVersion::V0_3),
        card
    );
}

#[test]
fn leaves_a_card_already_in_the_target_version_unchanged() {
    let card = object(r#"{"protocolVersion":"1.0","supportedInterfaces":[]}"#);
    assert_eq!(
        normalize_agent_card(card.clone(), ProtocolVersion::V1_0),
        card
    );
}

#[test]
fn downgrades_a_card_marked_03_that_still_carries_interfaces() {
    let card = object(
        r#"{"protocolVersion":"0.3","supportedInterfaces":[{"url":"https://agent","protocolBinding":"JSONRPC"}]}"#,
    );
    let converted = normalize_agent_card(card, ProtocolVersion::V0_3);
    assert_eq!(field(&converted, "protocolVersion"), &text_value("0.3"));
    assert!(!converted.contains_key("supportedInterfaces"));
    assert!(!converted.contains_key("additionalInterfaces"));
}

#[test]
fn extracts_string_and_integer_request_ids() {
    assert_eq!(
        to_value(request_id(&text_value("abc")).expect("a string id")).expect("JSON"),
        text_value("abc")
    );
    assert_eq!(
        to_value(request_id(&number_value(7)).expect("an integer id")).expect("JSON"),
        number_value(7)
    );
}

#[rstest]
#[case::fraction("1.5")]
#[case::null("null")]
#[case::object(r#"{"id": 1}"#)]
#[case::array("[1]")]
#[case::boolean("true")]
fn rejects_request_ids_that_are_not_strings_or_integers(#[case] text: &str) {
    assert!(request_id(&value(text)).is_none());
}
