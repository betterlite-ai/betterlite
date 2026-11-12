#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    Annotation, AnnotationType, ChatChoice, ChatMessage, ChatView, OtherView, OutputItem,
    OutputItemType, RawView, ResponsesView, ToolBillingResponse, anthropic_search_usage,
    includes_annotation_type, includes_file_search_call, includes_output_type,
    includes_web_search_call,
};
use crate::cost_usage::CostUsage;
use crate::cost_usage::server_tool_projection::ServerToolUse;
use crate::cost_usage::tool_projection::ToolUsage;
use crate::cost_usage::{CompletionDetails, PromptDetails};
use crate::decisions::Json;

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is valid JSON")
}

fn cost(prompt_tokens: i64, prompt_details: Option<PromptDetails>) -> CostUsage {
    CostUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_details,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn usage(cost_usage: CostUsage) -> ToolUsage {
    ToolUsage {
        cost: cost_usage,
        server_tool_use: None,
        server_side_details: None,
    }
}

fn with_server_tool(mut usage: ToolUsage, requests: i64) -> ToolUsage {
    usage.server_tool_use = Some(ServerToolUse {
        web_search_requests: Some(requests),
        tool_search_requests: None,
        browser_open_requests: None,
    });
    usage
}

fn with_side_details(mut usage: ToolUsage, source: &str) -> ToolUsage {
    usage.server_side_details =
        Some(serde_json::from_str(source).expect("the details are a mapping"));
    usage
}

fn prompts(requests: i64) -> PromptDetails {
    PromptDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: Some(requests),
    }
}

fn choice(message: Option<ChatMessage>) -> ChatChoice {
    ChatChoice { message }
}

fn annotations(items: &[&str]) -> ChatMessage {
    ChatMessage {
        annotations: Some(
            items
                .iter()
                .map(|source| Annotation {
                    annotation_type: Some(json(source)),
                })
                .collect(),
        ),
    }
}

fn chat(choices: Vec<ChatChoice>, usage: Option<ToolUsage>) -> ToolBillingResponse {
    ToolBillingResponse::Chat(ChatView { choices, usage })
}

fn raw(body: &str, usage: Option<ToolUsage>) -> ToolBillingResponse {
    ToolBillingResponse::Raw(RawView {
        body: json(body),
        usage,
    })
}

fn responses(output: Vec<OutputItem>, usage: Option<ToolUsage>) -> ToolBillingResponse {
    ToolBillingResponse::Responses(ResponsesView {
        output,
        tool_usage: None,
        usage,
    })
}

fn item(source: &str) -> OutputItem {
    OutputItem::Mapping(serde_json::from_str(source).expect("the item is a mapping"))
}

fn round_trip(view: &ToolBillingResponse) {
    let encoded = serde_json::to_value(view).expect("the response serializes");
    let decoded: ToolBillingResponse =
        serde_json::from_value(encoded).expect("the response deserializes");
    assert_eq!(&decoded, view);
}

#[rstest]
#[case::chat(r#"{"chat":{"choices":[],"usage":null}}"#)]
#[case::responses(r#"{"responses":{"output":[],"tool_usage":null,"usage":null}}"#)]
#[case::raw(r#"{"raw":{"body":null,"usage":null}}"#)]
#[case::other(r#"{"other":{"usage":null}}"#)]
fn the_case_input_decodes_every_response_tag(#[case] body: &str) {
    serde_json::from_str::<ToolBillingResponse>(body).expect("a snake_case view tag decodes");
}

#[rstest]
#[case::mapping(r#"{"mapping":{"type":"web_search_call"}}"#)]
#[case::attribute(r#"{"attribute":{"type":"web_search_call"}}"#)]
fn the_case_input_decodes_every_output_item_tag(#[case] body: &str) {
    serde_json::from_str::<OutputItem>(body).expect("a snake_case item tag decodes");
}

#[test]
fn every_response_view_round_trips() {
    round_trip(&chat(
        vec![choice(Some(annotations(&[r#"{"type":"url_citation"}"#])))],
        Some(usage(cost(4, Some(prompts(0))))),
    ));
    round_trip(&responses(
        vec![item(r#"{"type":"web_search_call"}"#)],
        None,
    ));
    round_trip(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":2}}}"#,
        None,
    ));
    round_trip(&ToolBillingResponse::Other(OtherView {
        usage: Some(usage(cost(1, None))),
    }));
}

#[test]
fn a_raw_search_count_including_zero_signals_web_search() {
    assert!(includes_web_search_call(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":0}}}"#,
        None
    )));
    assert!(includes_web_search_call(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":3}}}"#,
        None
    )));
}

#[test]
fn a_raw_mapping_without_a_probe_or_supplied_usage_is_not_a_search() {
    assert!(!includes_web_search_call(&raw(
        r#"{"usage":{},"output":[{"type":"web_search_call"}]}"#,
        None
    )));
}

#[test]
fn supplied_usage_signals_apply_to_a_raw_mapping() {
    let usage = with_server_tool(usage(cost(1, None)), 0);
    assert!(includes_web_search_call(&raw(
        r#"{"usage":{}}"#,
        Some(usage)
    )));
}

#[test]
fn chat_url_citations_signal_web_search() {
    let response = chat(
        vec![choice(Some(annotations(&[r#"{"type":"url_citation"}"#])))],
        None,
    );
    assert!(includes_web_search_call(&response));
    assert!(!includes_file_search_call(&response));
}

#[test]
fn chat_file_citations_signal_file_search_only() {
    let response = chat(
        vec![
            choice(Some(annotations(&[r#"{"type":"file_citation"}"#]))),
            choice(None),
            choice(Some(annotations(&[]))),
        ],
        None,
    );
    assert!(!includes_web_search_call(&response));
    assert!(includes_file_search_call(&response));
}

#[test]
fn annotation_scanning_skips_missing_messages_and_non_string_types() {
    let response = chat(
        vec![
            choice(None),
            choice(Some(ChatMessage { annotations: None })),
            choice(Some(annotations(&["{}", r#"{"type":5}"#]))),
        ],
        None,
    );
    assert!(!includes_web_search_call(&response));
    assert!(!includes_file_search_call(&response));
}

#[test]
fn a_matching_annotation_after_an_empty_one_is_found() {
    let response = chat(
        vec![
            choice(Some(ChatMessage { annotations: None })),
            choice(Some(annotations(&[r#"{"type":"file_citation"}"#]))),
        ],
        None,
    );
    assert!(includes_file_search_call(&response));
}

#[test]
fn chat_prompt_web_search_requests_including_zero_signal_web_search() {
    let response = chat(Vec::new(), Some(usage(cost(2, Some(prompts(0))))));
    assert!(includes_web_search_call(&response));
}

#[test]
fn chat_server_tool_counts_including_zero_signal_web_search() {
    let response = chat(Vec::new(), Some(with_server_tool(usage(cost(2, None)), 0)));
    assert!(includes_web_search_call(&response));
}

#[rstest]
#[case::positive(r#"{"web_search_calls":3}"#, true)]
#[case::boolean_true(r#"{"web_search_calls":true}"#, true)]
#[case::zero(r#"{"web_search_calls":0}"#, false)]
#[case::boolean_false(r#"{"web_search_calls":false}"#, false)]
#[case::negative(r#"{"web_search_calls":-1}"#, false)]
#[case::numeric_string(r#"{"web_search_calls":"3"}"#, false)]
#[case::integral_float(r#"{"web_search_calls":3.0}"#, false)]
#[case::beyond_i64(r#"{"web_search_calls":99999999999999999999}"#, true)]
#[case::negative_beyond_i64(r#"{"web_search_calls":-99999999999999999999}"#, false)]
#[case::exponent(r#"{"web_search_calls":1e2}"#, false)]
#[case::null(r#"{"web_search_calls":null}"#, false)]
#[case::missing("{}", false)]
fn the_server_side_counter_accepts_only_positive_integers(
    #[case] details: &str,
    #[case] expected: bool,
) {
    let response = chat(
        Vec::new(),
        Some(with_side_details(usage(cost(1, None)), details)),
    );
    assert_eq!(includes_web_search_call(&response), expected);
}

#[test]
fn a_chat_response_without_choices_or_usage_is_not_a_search() {
    assert!(!includes_web_search_call(&chat(Vec::new(), None)));
}

#[test]
fn a_responses_object_needs_a_web_search_call_output_item() {
    let with_item = responses(
        vec![item(r#"{"type":"web_search_call"}"#)],
        Some(with_side_details(
            usage(cost(1, None)),
            r#"{"web_search_calls":3}"#,
        )),
    );
    assert!(includes_web_search_call(&with_item));
    let without_item = responses(
        vec![item(r#"{"type":"message"}"#)],
        Some(with_side_details(
            usage(cost(1, None)),
            r#"{"web_search_calls":3}"#,
        )),
    );
    assert!(!includes_web_search_call(&without_item));
}

#[test]
fn responses_output_items_match_only_string_types() {
    let response = responses(
        vec![item(r#"{"type":"Web_Search_Call"}"#), item(r#"{"type":5}"#)],
        None,
    );
    assert!(!includes_web_search_call(&response));
    assert!(!includes_file_search_call(&response));
}

#[test]
fn responses_file_search_items_signal_file_search_only() {
    let response = responses(vec![item(r#"{"type":"file_search_call"}"#)], None);
    assert!(includes_file_search_call(&response));
    assert!(!includes_web_search_call(&response));
}

#[test]
fn other_kinds_use_the_supplied_usage_checks() {
    let other = ToolBillingResponse::Other(OtherView {
        usage: Some(with_server_tool(usage(cost(1, None)), 0)),
    });
    assert!(includes_web_search_call(&other));
    assert!(!includes_file_search_call(&other));
}

#[test]
fn annotation_and_output_checks_reject_the_other_kinds() {
    let raw_response = raw("{}", None);
    assert!(!includes_annotation_type(
        &raw_response,
        AnnotationType::UrlCitation
    ));
    assert!(!includes_output_type(
        &raw_response,
        OutputItemType::WebSearchCall
    ));
    let output_response = responses(Vec::new(), None);
    assert!(!includes_annotation_type(
        &output_response,
        AnnotationType::FileCitation
    ));
    assert!(!includes_output_type(
        &output_response,
        OutputItemType::FileSearchCall
    ));
}

#[test]
fn a_raw_zero_count_creates_usage_with_a_reported_zero() {
    let repaired = anthropic_search_usage(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":0}}}"#,
        None,
    ))
    .expect("the raw count creates usage");
    assert_eq!(repaired.cost, CostUsage::default());
    assert_eq!(
        repaired
            .server_tool_use
            .expect("the server tool use is set")
            .web_search_requests,
        Some(0)
    );
}

#[test]
fn an_existing_server_tool_count_wins_over_the_raw_probe() {
    let supplied = with_server_tool(usage(cost(4, None)), 5);
    let repaired = anthropic_search_usage(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":0}}}"#,
        Some(supplied),
    ))
    .expect("the supplied usage is returned");
    assert_eq!(
        repaired
            .server_tool_use
            .expect("the server tool use is kept")
            .web_search_requests,
        Some(5)
    );
}

#[test]
fn an_already_reported_zero_is_never_replaced() {
    let supplied = with_server_tool(usage(cost(4, None)), 0);
    let repaired = anthropic_search_usage(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":3}}}"#,
        Some(supplied),
    ))
    .expect("the supplied usage is returned");
    assert_eq!(
        repaired
            .server_tool_use
            .expect("the server tool use is kept")
            .web_search_requests,
        Some(0)
    );
}

#[test]
fn a_raw_count_repairs_supplied_usage_and_keeps_its_other_fields() {
    let supplied = with_side_details(
        usage(cost(6, Some(prompts(0)))),
        r#"{"web_search_calls":2}"#,
    );
    let repaired = anthropic_search_usage(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":4}}}"#,
        Some(supplied),
    ))
    .expect("the supplied usage is repaired");
    assert_eq!(repaired.cost.prompt_tokens, 6);
    assert_eq!(repaired.cost.prompt_details, Some(prompts(0)));
    assert_eq!(
        repaired.server_side_details,
        Some(serde_json::from_str(r#"{"web_search_calls":2}"#).expect("the details are a mapping"))
    );
    assert_eq!(
        repaired
            .server_tool_use
            .expect("the server tool use is replaced")
            .web_search_requests,
        Some(4)
    );
}

#[test]
fn without_a_raw_count_the_supplied_usage_is_returned_unchanged() {
    let supplied = usage(cost(6, None));
    let repaired = anthropic_search_usage(&raw(r#"{"usage":{}}"#, Some(supplied.clone())))
        .expect("the supplied usage is returned");
    assert_eq!(repaired, supplied);
    assert_eq!(anthropic_search_usage(&raw(r#"{"usage":{}}"#, None)), None);
}

#[test]
fn a_chat_response_never_reads_the_raw_probe() {
    let supplied = usage(cost(1, None));
    let repaired = anthropic_search_usage(&chat(Vec::new(), Some(supplied.clone())))
        .expect("the supplied usage is returned");
    assert_eq!(repaired, supplied);
    assert_eq!(
        anthropic_search_usage(&ToolBillingResponse::Other(OtherView { usage: None })),
        None
    );
}

#[test]
fn malformed_raw_counts_leave_the_usage_alone() {
    let repaired = anthropic_search_usage(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":"many"}}}"#,
        None,
    ));
    assert_eq!(repaired, None);
}

#[test]
fn completion_details_do_not_change_the_repaired_usage() {
    let mut supplied = usage(cost(2, None));
    supplied.cost.completion_details = Some(CompletionDetails {
        text_tokens: Some(4),
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        reasoning_tokens: Some(1),
    });
    let repaired = anthropic_search_usage(&raw(
        r#"{"usage":{"server_tool_use":{"web_search_requests":1}}}"#,
        Some(supplied.clone()),
    ))
    .expect("the supplied usage is repaired");
    assert_eq!(
        repaired.cost.completion_details,
        supplied.cost.completion_details
    );
}
