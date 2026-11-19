#![cfg(test)]

use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    NOW, ScriptError, applied, apply, cache as content, compressed_output, fixed_clock, harness,
    list, number, object, output, request, request_with_tools, text,
};
use crate::api::compression_interception::engine::CompressionOutput;
use crate::api::compression_interception::hook::CompressionHookError;
use crate::routes::CallType;

fn compressed(cache: IndexMap<String, String>) -> Vec<Result<CompressionOutput, ScriptError>> {
    vec![Ok(compressed_output(cache))]
}

fn compressed_messages() -> Json {
    list(vec![object(vec![
        ("role", text("user")),
        ("content", text("compressed")),
    ])])
}

fn search_tool() -> Json {
    object(vec![("type", text("custom")), ("name", text("search"))])
}

fn savings_of(body: &IndexMap<String, Json>) -> Option<Json> {
    match body.get("litellm_metadata") {
        Some(Json::Object(metadata)) => metadata.get("compression_savings").cloned(),
        _ => None,
    }
}

#[tokio::test]
async fn compressed_messages_replace_the_request_messages() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request();
    body.insert("litellm_call_id".to_owned(), text("call-1"));
    applied(&harness, &mut body).await;
    assert_eq!(body.get("messages"), Some(&compressed_messages()));
    assert_eq!(body.get("litellm_call_id"), Some(&text("call-1")));
    assert_eq!(
        harness.interception.get_cache(Some("call-1")),
        content(&[("key", "value")])
    );
}

#[tokio::test]
async fn no_tools_are_injected_when_the_request_had_none() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    assert!(!body.contains_key("tools"));
}

#[tokio::test]
async fn an_existing_tools_list_is_untouched_when_compression_adds_none() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request_with_tools(list(vec![search_tool()]));
    applied(&harness, &mut body).await;
    assert_eq!(body.get("tools"), Some(&list(vec![search_tool()])));
}

#[tokio::test]
async fn compressed_tools_are_appended_to_existing_tools() {
    let retrieve = object(vec![
        ("type", text("custom")),
        ("name", text("litellm_content_retrieve")),
    ]);
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(output(
            vec![object(vec![("role", text("user"))])],
            vec![retrieve.clone()],
            number(100),
            number(40),
            content(&[("key", "value")]),
        ))],
    );
    let mut body = request_with_tools(list(vec![search_tool()]));
    applied(&harness, &mut body).await;
    assert_eq!(
        body.get("tools"),
        Some(&list(vec![search_tool(), retrieve]))
    );
}

#[tokio::test]
async fn a_non_list_tools_field_is_an_error() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(output(
            vec![object(vec![("role", text("user"))])],
            vec![search_tool()],
            number(100),
            number(40),
            content(&[("key", "value")]),
        ))],
    );
    let mut body = request_with_tools(text("search"));
    let result = apply(&harness, &mut body, Some(CallType::AnthropicMessages)).await;
    assert!(matches!(
        result,
        Err(CompressionHookError::ToolsNotAList { field: "tools" })
    ));
}

#[tokio::test]
async fn a_missing_call_id_is_generated() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    let Some(Json::Text(call_id)) = body.get("litellm_call_id") else {
        panic!("the hook writes a call id");
    };
    assert!(!call_id.is_empty());
    assert_eq!(
        harness.interception.get_cache(Some(call_id.as_str())),
        content(&[("key", "value")])
    );
}

#[rstest]
#[case::empty(text(""))]
#[case::number(number(4))]
#[case::null(Json::Null)]
#[tokio::test]
async fn a_falsy_call_id_is_replaced(#[case] call_id: Json) {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request();
    body.insert("litellm_call_id".to_owned(), call_id);
    applied(&harness, &mut body).await;
    let Some(Json::Text(replaced)) = body.get("litellm_call_id") else {
        panic!("the hook writes a call id");
    };
    assert!(!replaced.is_empty());
    assert_eq!(
        harness.interception.get_cache(Some(replaced.as_str())),
        content(&[("key", "value")])
    );
}

fn expected_savings(before: u64, after: u64, saved: u64) -> Json {
    object(vec![
        (
            "tokens_before",
            number(i64::try_from(before).expect("small")),
        ),
        ("tokens_after", number(i64::try_from(after).expect("small"))),
        ("tokens_saved", number(i64::try_from(saved).expect("small"))),
        ("source", text("compression_interception")),
    ])
}

#[tokio::test]
async fn savings_are_created_when_no_metadata_exists() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    assert_eq!(savings_of(&body), Some(expected_savings(100, 40, 60)));
}

#[tokio::test]
async fn savings_attach_in_place_to_an_existing_metadata_object() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        compressed(content(&[("key", "value")])),
    );
    let mut body = request();
    body.insert(
        "litellm_metadata".to_owned(),
        object(vec![("team", text("research"))]),
    );
    applied(&harness, &mut body).await;
    let Some(Json::Object(metadata)) = body.get("litellm_metadata") else {
        panic!("the metadata stays an object");
    };
    assert_eq!(metadata.get("team"), Some(&text("research")));
    assert_eq!(savings_of(&body), Some(expected_savings(100, 40, 60)));
}

#[tokio::test]
async fn zero_counts_record_zero_savings() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(output(
            vec![object(vec![("role", text("user"))])],
            Vec::new(),
            number(0),
            number(0),
            content(&[("key", "value")]),
        ))],
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    assert_eq!(savings_of(&body), Some(expected_savings(0, 0, 0)));
}

#[tokio::test]
async fn invalid_counts_record_no_savings() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(output(
            vec![object(vec![("role", text("user"))])],
            Vec::new(),
            Json::Bool(true),
            number(0),
            content(&[("key", "value")]),
        ))],
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    assert!(!body.contains_key("litellm_metadata"));
}
