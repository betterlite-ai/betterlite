#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_websearch::call_transformation::TransformationError;
use betterlite_websearch::call_transformation::WebSearchTransformation;
use betterlite_websearch::call_transformation::wire::{Call, Response};
use betterlite_websearch::native_blocks::{
    NATIVE_BLOCKS_METADATA_KEY, ResponseJson, build_native_result_blocks, inject_native_blocks,
    native_result_pair, post_agentic_response_hook, tool_call_query,
};
use betterlite_websearch::records::{SearchOutcome, SearchSucceeded};

fn parsed(body: &str) -> ResponseJson {
    serde_json::from_str(body).unwrap()
}

fn call_with_input(id: &str, input_json: &str) -> Call {
    let body = format!(
        r#"{{"content":[{{"type":"tool_use","id":"{id}","name":"web_search","input":{input_json}}}]}}"#
    );
    let response: Response = serde_json::from_str(&body).unwrap();
    let (_, calls) =
        WebSearchTransformation::transform_request(&response, false, "anthropic").unwrap();
    calls.into_iter().next().unwrap()
}

fn succeeded(text: &str) -> SearchOutcome {
    SearchOutcome::Succeeded(SearchSucceeded {
        text: text.to_owned(),
        response: None,
    })
}

#[test]
fn tool_call_query_extracts_the_query_string() {
    let call = call_with_input("call-1", r#"{"query": "rust programming"}"#);
    assert_eq!(tool_call_query(&call), "rust programming");
}

#[test]
fn tool_call_query_returns_empty_when_the_query_is_missing() {
    let call = call_with_input("call-1", "{}");
    assert_eq!(tool_call_query(&call), "");
}

#[test]
fn tool_call_query_returns_empty_when_the_query_is_not_a_string() {
    let call = call_with_input("call-1", r#"{"query": 42}"#);
    assert_eq!(tool_call_query(&call), "");
}

#[test]
fn native_result_pair_mints_a_fresh_srvtoolu_id() {
    let (server_tool_use, result) = native_result_pair("rust", &succeeded("hi"));
    assert!(server_tool_use.id.starts_with("srvtoolu_"));
    assert_eq!(server_tool_use.input.query, "rust");
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(encoded["tool_use_id"], server_tool_use.id);
}

#[test]
fn native_result_pair_mints_a_different_id_on_every_call() {
    let (first, _) = native_result_pair("rust", &succeeded("hi"));
    let (second, _) = native_result_pair("rust", &succeeded("hi"));
    assert_ne!(first.id, second.id);
}

#[test]
fn build_native_result_blocks_pairs_each_call_with_its_own_outcome() {
    let call = call_with_input("call-1", r#"{"query": "rust"}"#);
    let outcomes = vec![succeeded("hi")];
    let pairs = build_native_result_blocks(&[call], &outcomes).unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0.input.query, "rust");
}

#[test]
fn build_native_result_blocks_errors_when_outcome_count_does_not_match() {
    let call = call_with_input("call-1", r#"{"query": "rust"}"#);
    let outcomes = vec![succeeded("hi"), succeeded("bye")];
    assert!(matches!(
        build_native_result_blocks(&[call], &outcomes),
        Err(TransformationError::MismatchedOutcomes {
            calls: 1,
            outcomes: 2
        })
    ));
}

#[test]
fn inject_native_blocks_prepends_to_existing_content() {
    let response = parsed(r#"{"role": "assistant", "content": [{"type": "text", "text": "hi"}]}"#);
    let blocks = vec![parsed(r#"{"type": "server_tool_use"}"#)];
    let injected = inject_native_blocks(response, &blocks);
    assert_eq!(injected["content"][0]["type"], "server_tool_use");
    assert_eq!(injected["content"][1]["text"], "hi");
}

#[test]
fn inject_native_blocks_is_a_noop_when_there_are_no_blocks() {
    let response = parsed(r#"{"content": []}"#);
    let injected = inject_native_blocks(response.clone(), &[]);
    assert_eq!(injected, response);
}

#[test]
fn inject_native_blocks_leaves_a_non_object_response_unchanged() {
    let response = parsed(r#""not an object""#);
    let blocks = vec![parsed(r#"{"type": "server_tool_use"}"#)];
    let injected = inject_native_blocks(response.clone(), &blocks);
    assert_eq!(injected, response);
}

#[test]
fn post_agentic_response_hook_injects_blocks_found_in_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        NATIVE_BLOCKS_METADATA_KEY.to_owned(),
        parsed(r#"[{"type": "server_tool_use"}]"#),
    );
    let response = parsed(r#"{"content": []}"#);
    let injected = post_agentic_response_hook(response, &metadata);
    assert_eq!(injected["content"][0]["type"], "server_tool_use");
}

#[test]
fn post_agentic_response_hook_is_a_noop_without_the_metadata_key() {
    let metadata = BTreeMap::new();
    let response = parsed(r#"{"content": []}"#);
    let injected = post_agentic_response_hook(response.clone(), &metadata);
    assert_eq!(injected, response);
}

#[test]
fn post_agentic_response_hook_is_a_noop_when_the_metadata_blocks_are_empty() {
    let mut metadata = BTreeMap::new();
    metadata.insert(NATIVE_BLOCKS_METADATA_KEY.to_owned(), parsed("[]"));
    let response = parsed(r#"{"content": []}"#);
    let injected = post_agentic_response_hook(response.clone(), &metadata);
    assert_eq!(injected, response);
}
