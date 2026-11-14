use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::pre_call::{CONVERTED_STREAM_KEY, pre_call_deployment_hook};
use super::support;

fn request(pairs: &[(&str, &str)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), serde_json::from_str(value).unwrap()))
        .collect()
}

fn listed(tool: &str) -> String {
    format!("[{tool}]")
}

fn streaming() -> IndexMap<String, Json> {
    let tools = listed(support::RETRIEVE_TOOL);
    request(&[("stream", "true"), ("tools", &tools)])
}

fn assert_converted(converted: &IndexMap<String, Json>, source: &IndexMap<String, Json>) {
    assert_eq!(converted.get("stream"), Some(&Json::Bool(false)));
    assert_eq!(converted.get(CONVERTED_STREAM_KEY), Some(&Json::Bool(true)));
    assert_eq!(converted.get("tools"), source.get("tools"));
}

#[test]
fn the_converted_stream_key_is_the_litellm_constant() {
    assert_eq!(
        CONVERTED_STREAM_KEY,
        "_headroom_interception_converted_stream"
    );
}

#[test]
fn a_streaming_completion_with_the_retrieve_tool_is_converted() {
    let kwargs = streaming();
    let converted = pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)).unwrap();
    assert_converted(&converted, &kwargs);
    assert_eq!(converted.len(), kwargs.len() + 1);
}

#[rstest]
#[case::completion(CallType::Completion)]
#[case::acompletion(CallType::Acompletion)]
#[case::responses(CallType::Responses)]
#[case::aresponses(CallType::Aresponses)]
fn every_convertible_call_type_is_converted(#[case] call_type: CallType) {
    let kwargs = streaming();
    assert!(pre_call_deployment_hook(None, &kwargs, Some(call_type)).is_some());
}

#[rstest]
#[case::embedding(CallType::Embedding)]
#[case::anthropic_messages(CallType::AnthropicMessages)]
#[case::moderation(CallType::Moderation)]
#[case::text_completion(CallType::TextCompletion)]
#[case::image_generation(CallType::ImageGeneration)]
fn other_call_types_are_unchanged(#[case] call_type: CallType) {
    let kwargs = streaming();
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(call_type)),
        None
    );
}

#[test]
fn a_missing_call_type_is_unchanged() {
    let kwargs = streaming();
    assert_eq!(pre_call_deployment_hook(None, &kwargs, None), None);
}

#[test]
fn a_non_streaming_call_is_unchanged() {
    let tools = listed(support::RETRIEVE_TOOL);
    let kwargs = request(&[("stream", "false"), ("tools", &tools)]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn a_call_without_stream_is_unchanged() {
    let tools = listed(support::RETRIEVE_TOOL);
    let kwargs = request(&[("tools", &tools)]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn a_string_stream_value_is_not_a_stream() {
    let tools = listed(support::RETRIEVE_TOOL);
    let kwargs = request(&[("stream", "\"true\""), ("tools", &tools)]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn a_background_stream_call_is_unchanged() {
    let tools = listed(support::RETRIEVE_TOOL);
    let kwargs = request(&[
        ("stream", "true"),
        ("background", "true"),
        ("tools", &tools),
    ]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn a_false_background_still_converts() {
    let tools = listed(support::RETRIEVE_TOOL);
    let kwargs = request(&[
        ("stream", "true"),
        ("background", "false"),
        ("tools", &tools),
    ]);
    let converted = pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)).unwrap();
    assert_converted(&converted, &kwargs);
}

#[test]
fn a_stream_call_without_tools_is_unchanged() {
    let kwargs = request(&[("stream", "true")]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn a_stream_call_with_another_tool_is_unchanged() {
    let tools = listed(r#"{"type":"function","function":{"name":"lookup"}}"#);
    let kwargs = request(&[("stream", "true"), ("tools", &tools)]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn tools_that_are_not_a_list_are_unchanged() {
    let kwargs = request(&[("stream", "true"), ("tools", "{}")]);
    assert_eq!(
        pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)),
        None
    );
}

#[test]
fn a_flat_named_retrieve_tool_is_found() {
    let tools = listed(r#"{"type":"function","name":"headroom_retrieve"}"#);
    let kwargs = request(&[("stream", "true"), ("tools", &tools)]);
    let converted = pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)).unwrap();
    assert_converted(&converted, &kwargs);
}

#[test]
fn the_base_result_is_what_gets_converted() {
    let base = streaming();
    let kwargs = request(&[("stream", "false")]);
    let converted =
        pre_call_deployment_hook(Some(&base), &kwargs, Some(CallType::Completion)).unwrap();
    assert_converted(&converted, &base);
}

#[test]
fn the_base_result_is_returned_when_it_cannot_convert() {
    let tools = listed(support::RETRIEVE_TOOL);
    let base = request(&[("stream", "false"), ("tools", &tools)]);
    let kwargs = streaming();
    let returned = pre_call_deployment_hook(Some(&base), &kwargs, Some(CallType::Completion));
    assert_eq!(returned, Some(base));
}

#[test]
fn the_base_result_is_returned_for_another_call_type() {
    let tools = listed(support::RETRIEVE_TOOL);
    let base = request(&[("stream", "false"), ("tools", &tools)]);
    let kwargs = request(&[("stream", "false")]);
    let returned = pre_call_deployment_hook(Some(&base), &kwargs, Some(CallType::Embedding));
    assert_eq!(returned, Some(base));
}

#[test]
fn the_conversion_keeps_the_other_request_keys() {
    let tools = listed(support::RETRIEVE_TOOL);
    let kwargs = request(&[
        ("stream", "true"),
        ("tools", &tools),
        ("model", "\"gpt-4o\""),
        ("metadata", "{\"team\":\"a\"}"),
    ]);
    let converted = pre_call_deployment_hook(None, &kwargs, Some(CallType::Completion)).unwrap();
    assert_eq!(converted.get("model"), kwargs.get("model"));
    assert_eq!(converted.get("metadata"), kwargs.get("metadata"));
    assert_eq!(converted.len(), kwargs.len() + 1);
}
