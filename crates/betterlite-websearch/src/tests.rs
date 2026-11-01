#![cfg(test)]

use std::fs;
use std::path::Path;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::with_default;

use betterlite_error::error::ErrorClass;

use crate::call_transformation::native;
use crate::call_transformation::wire::{Response, Thinking};
use crate::call_transformation::{TransformationError, WebSearchTransformation};
use crate::loop_eligibility::{
    CHAT_COMPLETION_SURFACE, RESPONSES_AGENTIC_SURFACE, WebSearchInterceptionState,
};
use crate::records::{SearchFailed, SearchOutcome, SearchSucceeded, WebSearchToolResultErrorCode};
use crate::tools::{
    OpenAiFunctionName, SearchInputProperties, SearchInputSchema, SearchQueriesProperty,
    StringProperty, StringType, Tool, get_litellm_web_search_tool, is_chat_web_search_tool,
    is_web_search_tool, is_web_search_tool_responses, openai_web_search_tool,
    responses_web_search_tool,
};
use betterlite_openai::search::records::SearchResponse;

#[test]
fn web_search_builders_have_expected_dialects() {
    let anthropic = serde_json::to_value(get_litellm_web_search_tool()).unwrap();
    let openai = serde_json::to_value(openai_web_search_tool()).unwrap();
    let responses = serde_json::to_value(responses_web_search_tool()).unwrap();
    assert_eq!(anthropic["name"], "litellm_web_search");
    assert_eq!(openai["function"]["name"], "litellm_web_search");
    assert_eq!(responses["name"], "litellm_web_search");
    assert_eq!(anthropic["input_schema"]["required"][0], "query");
}

#[test]
fn web_search_tool_responses() {
    assert!(is_web_search_tool_responses(&tool(
        Some("web_search_preview_2025_03_11"),
        None,
        None,
        false
    )));
    assert!(is_web_search_tool_responses(&tool(
        Some("function"),
        Some("litellm_web_search"),
        None,
        false
    )));
    assert!(!is_web_search_tool_responses(&tool(
        Some("function"),
        Some("other"),
        None,
        false
    )));
}

#[test]
fn web_search_tool_chat_completion() {
    assert!(is_chat_web_search_tool(&tool(
        None,
        Some("litellm_web_search"),
        None,
        false
    )));
    assert!(is_chat_web_search_tool(&tool(
        Some("function"),
        None,
        Some("litellm_web_search"),
        false
    )));
    assert!(!is_chat_web_search_tool(&tool(
        Some("web_search_20250305"),
        Some("web_search"),
        None,
        false
    )));
}

#[test]
fn web_search_tool() {
    assert!(is_web_search_tool(&tool(
        Some("web_search_20250305"),
        Some("web_search"),
        None,
        false
    )));
    assert!(is_web_search_tool(&tool(
        None,
        Some("WebSearch"),
        None,
        false
    )));
    assert!(!is_web_search_tool(&tool(
        None,
        Some("WebSearch"),
        None,
        true
    )));
}

fn tool(
    tool_type: Option<&str>,
    name: Option<&str>,
    function_name: Option<&str>,
    schema: bool,
) -> Tool {
    Tool {
        tool_type: tool_type.map(str::to_owned),
        name: name.map(str::to_owned),
        function: function_name.map(|function_text| OpenAiFunctionName {
            name: Some(function_text.to_owned()),
        }),
        input_schema: schema.then(|| SearchInputSchema {
            schema_type: "object".to_owned(),
            properties: SearchInputProperties {
                query: StringProperty {
                    property_type: "string".to_owned(),
                    description: String::new(),
                },
                objective: StringProperty {
                    property_type: "string".to_owned(),
                    description: String::new(),
                },
                search_queries: SearchQueriesProperty {
                    property_type: "array".to_owned(),
                    items: StringType {
                        value_type: "string".to_owned(),
                    },
                    description: String::new(),
                },
            },
            required: Vec::new(),
        }),
    }
}

#[test]
fn web_search_types() {
    let succeeded = SearchSucceeded {
        text: "text".to_owned(),
        response: None,
    };
    let failed = SearchFailed {
        error_code: WebSearchToolResultErrorCode::Unavailable,
        message: "down".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(SearchOutcome::Succeeded(succeeded)).unwrap()["text"],
        "text"
    );
    assert_eq!(
        serde_json::to_value(SearchOutcome::Failed(failed)).unwrap()["error_code"],
        "unavailable"
    );
}

#[test]
fn call_transformation_anthropic_detection_and_response() {
    let response = r#"{
        "content": [
            {"type": "thinking", "thinking": "reason"},
            {"type": "tool_use", "id": "call-1", "name": "web_search", "input": {"query": "rust"}}
        ]
    }"#;
    let (found, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(response).unwrap(),
        false,
        "anthropic",
    )
    .unwrap();
    assert!(found);
    assert_eq!(calls[0].id.as_deref(), Some("call-1"));
    assert_eq!(
        serde_json::to_value(&calls[0].input).unwrap()["query"],
        "rust"
    );
    let (mut assistant, user) = WebSearchTransformation::transform_response(
        &calls,
        &[String::from(
            "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe",
        )],
        "anthropic",
    )
    .unwrap();
    assistant.prepend_thinking(&[Thinking::Thinking {
        thinking: "reason".to_owned(),
        signature: "signed".to_owned(),
    }]);
    let assistant_json = serde_json::to_value(assistant).unwrap();
    let user_json = serde_json::to_value(user).unwrap();
    assert_eq!(assistant_json["content"][0]["type"], "thinking");
    assert_eq!(assistant_json["content"][1]["type"], "tool_use");
    assert_eq!(user_json["content"][0]["tool_use_id"], "call-1");
}

#[test]
fn call_transformation_openai_dict_and_string_arguments() {
    let response = r#"{"choices": [{"message": {"tool_calls": [
        {"id": "call-1", "type": "function", "function": {"name": "litellm_web_search", "arguments": "{\"query\":\"rust\"}"}},
        {"id": "call-2", "type": "function", "function": {"name": "web_search", "arguments": {"query": "cargo"}}}
    ]}}]}"#;
    let (found, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert!(found);
    assert_eq!(calls.len(), 2);
    assert_eq!(
        serde_json::to_value(&calls[0].input).unwrap()["query"],
        "rust"
    );
    let (assistant, tools) = WebSearchTransformation::transform_response(
        &calls,
        &[String::from("one"), String::from("two")],
        "openai",
    )
    .unwrap();
    let assistant_json = serde_json::to_value(assistant).unwrap();
    let tools_json = serde_json::to_value(tools).unwrap();
    assert_eq!(
        assistant_json["tool_calls"][0]["function"]["arguments"],
        "{\"query\": \"rust\"}"
    );
    assert_eq!(tools_json[0]["tool_call_id"], "call-1");
    assert_eq!(tools_json[1]["tool_call_id"], "call-2");
}

#[test]
fn call_transformation_responses_and_no_tool() {
    let response = r#"{"output": [
        {"type": "web_search_call", "name": "web_search_preview"},
        {"type": "function_call", "call_id": "call-1", "name": "litellm_web_search", "arguments": {"query": "rust"}}
    ]}"#;
    let (found, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(response).unwrap(),
        false,
        "responses",
    )
    .unwrap();
    assert!(found);
    assert_eq!(
        serde_json::to_value(calls[0].kind).unwrap(),
        "function_call"
    );
    assert_eq!(
        serde_json::to_value(&calls[0].input).unwrap()["query"],
        "rust"
    );
}

#[test]
fn call_transformation_arguments_depth_limit_errors() {
    let depth = 40;
    let nested = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    let response = format!(
        r#"{{"choices": [{{"message": {{"tool_calls": [
            {{"id": "call-1", "type": "function", "function": {{"name": "litellm_web_search", "arguments": {{"query": {nested}}}}}}}
        ]}}}}]}}"#
    );
    let (found, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(&response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert!(found);
    assert!(matches!(
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "openai"),
        Err(TransformationError::ArgumentsTooComplex)
    ));
}

#[test]
fn call_transformation_arguments_byte_limit_rejects_wide_arrays() {
    let items = "0,".repeat(600_000);
    let response = format!(
        r#"{{"choices": [{{"message": {{"tool_calls": [
            {{"id": "call-1", "type": "function", "function": {{"name": "litellm_web_search", "arguments": {{"query": [{items}0]}}}}}}
        ]}}}}]}}"#
    );
    let (_, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(&response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert!(matches!(
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "openai"),
        Err(TransformationError::ArgumentsTooComplex)
    ));
}

#[test]
fn call_transformation_arguments_byte_limit_rejects_oversized_string_and_key() {
    let value = "x".repeat(1_100_000);
    let key = "k".repeat(1_100_000);
    let response = format!(
        r#"{{"choices": [{{"message": {{"tool_calls": [
            {{"id": "call-1", "type": "function", "function": {{"name": "litellm_web_search", "arguments": {{"{key}": "{value}"}}}}}}
        ]}}}}]}}"#
    );
    let (_, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(&response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert!(matches!(
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "openai"),
        Err(TransformationError::ArgumentsTooComplex)
    ));
}

fn assert_false_value_normalizes_to_empty_object(raw: &str) {
    let response = format!(
        r#"{{"choices": [{{"message": {{"tool_calls": [
            {{"id": "call-1", "type": "function", "function": {{"name": "litellm_web_search", "arguments": {raw}}}}}
        ]}}}}]}}"#
    );
    let (found, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(&response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert!(found);
    assert_eq!(serde_json::to_string(&calls[0].input).unwrap(), "{}");
    let (assistant, _) =
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "openai")
            .unwrap();
    assert_eq!(
        serde_json::to_value(assistant).unwrap()["tool_calls"][0]["function"]["arguments"],
        "{}"
    );
}

#[test]
fn call_transformation_arguments_normalize_empty_array_by_value() {
    assert_false_value_normalizes_to_empty_object("[]");
}

#[test]
fn call_transformation_arguments_normalize_negative_zero_by_value() {
    assert_false_value_normalizes_to_empty_object("-0");
}

#[test]
fn call_transformation_arguments_normalize_decimal_zero_by_value() {
    assert_false_value_normalizes_to_empty_object("0.0");
}

#[test]
fn call_transformation_arguments_keep_string_encoded_empty_array() {
    let response = r#"{"choices": [{"message": {"tool_calls": [
        {"id": "call-1", "type": "function", "function": {"name": "litellm_web_search", "arguments": "[]"}}
    ]}}]}"#;
    let (_, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert_eq!(serde_json::to_string(&calls[0].input).unwrap(), "[]");
}

#[test]
fn call_transformation_arguments_byte_limit_rejects_oversized_encoded_string() {
    let value = "x".repeat(1_100_000);
    let inner = serde_json::to_string(&value).unwrap();
    let encoded = serde_json::to_string(&inner).unwrap();
    let response = format!(
        r#"{{"choices": [{{"message": {{"tool_calls": [
            {{"id": "call-1", "type": "function", "function": {{"name": "litellm_web_search", "arguments": {encoded}}}}}
        ]}}}}]}}"#
    );
    let (_, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(&response).unwrap(),
        false,
        "openai",
    )
    .unwrap();
    assert!(matches!(
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "openai"),
        Err(TransformationError::ArgumentsTooComplex)
    ));
}

#[test]
fn call_transformation_responses_skips_unparsable_items() {
    let response = r#"{"output": [
        null,
        {"type": "function_call", "call_id": "one", "name": "litellm_web_search", "arguments": {"query": "rust"}}
    ]}"#;
    let (found, calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(response).unwrap(),
        false,
        "responses",
    )
    .unwrap();
    assert!(found);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_deref(), Some("one"));
}

#[test]
fn call_transformation_invalid_and_stream() {
    let invalid = r#"{"output": [
        {"type": "function_call", "call_id": "call-2", "name": "litellm_web_search", "arguments": "not-json"}
    ]}"#;
    let (_, invalid_calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(invalid).unwrap(),
        false,
        "responses",
    )
    .unwrap();
    assert_eq!(invalid_calls[0].arguments.as_deref(), Some("not-json"));
    assert_eq!(
        serde_json::to_string(&invalid_calls[0].input).unwrap(),
        "{}"
    );
    let empty: Response = serde_json::from_str("{}").unwrap();
    let (found_empty, empty_calls) =
        WebSearchTransformation::transform_request(&empty, false, "responses").unwrap();
    assert!(!found_empty);
    assert!(empty_calls.is_empty());
    let (found_stream, stream_calls) = WebSearchTransformation::transform_request(
        &serde_json::from_str::<Response>(invalid).unwrap(),
        true,
        "responses",
    )
    .unwrap();
    assert!(!found_stream);
    assert!(stream_calls.is_empty());
}

#[test]
fn call_transformation_result_blocks_error_codes_and_format_search_response() {
    let response: SearchResponse = serde_json::from_str(r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","snippet":"safe","date":"today"}]}"#).unwrap();
    let block = serde_json::to_value(native::result_block("call-1", Some(&response))).unwrap();
    assert_eq!(block["tool_use_id"], "call-1");
    assert_eq!(block["content"][0]["page_age"], "today");
    assert_eq!(block["content"][0]["encrypted_content"], "");
    assert_eq!(
        native::format_search_response(&response).unwrap(),
        "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe"
    );
    assert_eq!(
        native::search_error_code(&rate_limit_error()),
        WebSearchToolResultErrorCode::TooManyRequests
    );
}

fn rate_limit_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"rate_limit","context":{"message":"slow down","model":null,"provider":null},"category":"vendor_rate_limit"}"#,
    )
    .unwrap()
}

fn bad_request_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"bad_request","message":"bad input","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn unclassified_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"unclassified","message":"boom","model":null,"provider":null}"#,
    )
    .unwrap()
}

#[test]
fn call_transformation_result_block_empty_date() {
    let response: SearchResponse = serde_json::from_str(r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","snippet":"safe","date":"","last_updated":"yesterday"}]}"#).unwrap();
    let block = serde_json::to_value(native::result_block("call", Some(&response))).unwrap();
    assert_eq!(block["content"][0]["page_age"], "yesterday");
}

#[test]
fn call_transformation_error_codes() {
    assert_eq!(
        native::search_error_code(&bad_request_error()),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
    assert_eq!(
        native::search_error_code(&unclassified_error()),
        WebSearchToolResultErrorCode::Unavailable
    );
    let error = native::result_error_block("failed", WebSearchToolResultErrorCode::TooManyRequests);
    let encoded = serde_json::to_value(error).unwrap();
    assert_eq!(encoded["type"], "web_search_tool_result");
    assert_eq!(encoded["tool_use_id"], "failed");
    assert_eq!(encoded["content"]["type"], "web_search_tool_result_error");
    assert_eq!(encoded["content"]["error_code"], "too_many_requests");
}

#[test]
fn call_transformation_result_block_outcomes() {
    let success = SearchOutcome::Succeeded(SearchSucceeded {
        text: "empty".to_owned(),
        response: None,
    });
    let success_block = serde_json::to_value(native::outcome_block("empty", &success)).unwrap();
    assert_eq!(success_block["content"].as_array().unwrap().len(), 0);
    assert_eq!(success_block["tool_use_id"], "empty");
    let failure = SearchOutcome::Failed(SearchFailed {
        message: "failed".to_owned(),
        error_code: WebSearchToolResultErrorCode::Unavailable,
    });
    let failure_block = serde_json::to_value(native::outcome_block("failed", &failure)).unwrap();
    assert_eq!(failure_block["content"]["error_code"], "unavailable");
    let response: SearchResponse = serde_json::from_str(r#"{"results":[]}"#).unwrap();
    let empty_block = serde_json::to_value(native::result_block("empty", Some(&response))).unwrap();
    assert_eq!(empty_block, success_block);
}

#[test]
fn call_transformation_format_search_empty() {
    let response: SearchResponse =
        serde_json::from_str(r#"{"results":[],"note":"empty","count":0}"#).unwrap();
    assert_eq!(
        native::format_search_response(&response).unwrap(),
        "results=[] object='search' count=0 note='empty'"
    );
}

#[test]
fn call_transformation_missing_results() {
    let response: Response = serde_json::from_str(
        r#"{"content":[{"type":"tool_use","name":"web_search","id":"one","input":{}}]}"#,
    )
    .unwrap();
    let (_, calls) =
        WebSearchTransformation::transform_request(&response, false, "anthropic").unwrap();
    assert!(matches!(
        WebSearchTransformation::transform_response(&calls, &[], "anthropic"),
        Err(TransformationError::MissingResults {
            calls: 1,
            results: 0
        })
    ));
}

#[test]
fn call_transformation_no_tool_anthropic() {
    let response: Response = serde_json::from_str(
        r#"{"content":[{"type":"tool_use","name":"WebSearch","input":{}},{"type":"text"}]}"#,
    )
    .unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "anthropic").unwrap();
    assert!(!found);
    assert!(calls.is_empty());
}

#[test]
fn call_transformation_no_tool_openai() {
    let response: Response = serde_json::from_str(r#"{"choices":[{"message":{"tool_calls":[{"type":"function","function":{"name":"WebSearch","arguments":"{}"}}]}}]}"#).unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "openai").unwrap();
    assert!(!found);
    assert!(calls.is_empty());
}

#[test]
fn call_transformation_choices_null_openai() {
    let response: Response = serde_json::from_str(r#"{"choices":null}"#).unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "openai").unwrap();
    assert!(!found);
    assert!(calls.is_empty());
}

const VALID_FIRST_CHOICE: &str = concat!(
    r#"{"message":{"tool_calls":[{"type":"function","function":"#,
    r#"{"name":"litellm_web_search","arguments":"{}"}}]}}"#
);

#[test]
fn call_transformation_choices_second_entry_null_openai() {
    let body = format!(r#"{{"choices":[{VALID_FIRST_CHOICE},null]}}"#);
    let response: Response = serde_json::from_str(&body).unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "openai").unwrap();
    assert!(found);
    assert_eq!(calls.len(), 1);
}

#[test]
fn call_transformation_choices_second_entry_malformed_openai() {
    let body = format!(r#"{{"choices":[{VALID_FIRST_CHOICE},{{"message":"not an object"}}]}}"#);
    let response: Response = serde_json::from_str(&body).unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "openai").unwrap();
    assert!(found);
    assert_eq!(calls.len(), 1);
}

#[test]
fn call_transformation_choices_not_a_list_is_error() {
    let err = serde_json::from_str::<Response>(r#"{"choices":"not-a-list"}"#).unwrap_err();
    assert!(err.to_string().contains("choices must be a list or null"));
}

#[test]
fn call_transformation_no_tool_responses() {
    let response: Response = serde_json::from_str(r#"{"output":[{"type":"function_call","name":"web_search"},{"type":"web_search_preview_2025"},{"type":"web_search_20250305"}]}"#).unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "responses").unwrap();
    assert!(!found);
    assert!(calls.is_empty());
}

fn call_transformation_outcome(response: &Response, format: &str) -> Option<(bool, bool)> {
    let (found, calls) =
        WebSearchTransformation::transform_request(response, false, format).ok()?;
    if calls.is_empty() {
        return Some((found, false));
    }
    let results: Vec<String> = calls.iter().map(|_| String::from("result")).collect();
    let budget_error = matches!(
        WebSearchTransformation::transform_response(&calls, &results, format),
        Err(TransformationError::ArgumentsTooComplex)
    );
    Some((found, budget_error))
}

fn call_transformation_outcomes(response: &Response) -> Vec<(bool, bool)> {
    ["anthropic", "openai", "responses"]
        .into_iter()
        .filter_map(|format| call_transformation_outcome(response, format))
        .collect()
}

#[test]
fn replays_the_call_transformation_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/call_transformation");
    let outcomes: Vec<(bool, bool)> = fs::read_dir(corpus)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|dir_entry| dir_entry.path().is_file())
        })
        .flat_map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let body = str::from_utf8(&bytes).unwrap_or_default();
            serde_json::from_str::<Response>(body)
                .map(|response| call_transformation_outcomes(&response))
                .unwrap_or_default()
        })
        .collect();
    assert!(
        outcomes.iter().any(|&(found, _)| found),
        "corpus must exercise a detected call"
    );
    assert!(
        outcomes.iter().any(|&(found, _)| !found),
        "corpus must exercise a response with no call"
    );
    assert!(
        outcomes.iter().any(|&(_, budget_error)| budget_error),
        "corpus must exercise the argument depth limit"
    );
}

#[derive(Clone)]
struct WarningCounter(Arc<AtomicUsize>);

impl tracing::Subscriber for WarningCounter {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
}

#[test]
fn call_transformation_stream_warns() {
    let counter = Arc::new(AtomicUsize::new(0));
    let subscriber = WarningCounter(Arc::clone(&counter));
    let response: Response =
        serde_json::from_str(r#"{"content":[{"type":"tool_use","name":"litellm_web_search"}]}"#)
            .unwrap();
    let (found, calls) = with_default(subscriber, || {
        WebSearchTransformation::transform_request(&response, true, "anthropic")
    })
    .unwrap();
    assert!(!found);
    assert!(calls.is_empty());
    assert_eq!(counter.load(Ordering::Relaxed), 1);
}

const ANTHROPIC_RESPONSE_WITH_THINKING: &str = r#"{"content":[
    {"type":"thinking","thinking":"reason","signature":"sig"},
    {"type":"tool_use","id":"call-1","name":"litellm_web_search","input":{"query":"rust"}}
]}"#;

const ANTHROPIC_RESPONSE_WITH_REDACTED_THINKING: &str = r#"{"content":[
    {"type":"redacted_thinking","data":"abc"},
    {"type":"tool_use","id":"call-1","name":"litellm_web_search","input":{"query":"rust"}}
]}"#;

const ANTHROPIC_RESPONSE_NO_TOOL: &str = r#"{"content":[{"type":"text","text":"hi"}]}"#;

const CHAT_RESPONSE_WITH_CALL: &str = r#"{"choices":[{"message":{"tool_calls":[
    {"id":"call-1","type":"function","function":{"name":"litellm_web_search","arguments":"{\"query\":\"rust\"}"}}
]}}]}"#;

const RESPONSES_RESPONSE_WITH_CALL: &str = r#"{"output":[
    {"type":"function_call","call_id":"call-1","name":"litellm_web_search","arguments":{"query":"rust"}}
]}"#;

fn search_tool() -> Tool {
    tool(Some("function"), Some("litellm_web_search"), None, false)
}

fn native_search_tool() -> Tool {
    tool(Some("web_search_20250305"), Some("web_search"), None, false)
}

#[test]
fn dispatcher_routes_to_chat_completions_gate_by_surface_kwarg() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            CHAT_RESPONSE_WITH_CALL,
            "gpt-4",
            &[],
            Some(&[search_tool()]),
            false,
            "openai",
            Some(CHAT_COMPLETION_SURFACE),
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "openai");
}

#[test]
fn dispatcher_routes_to_responses_gate_by_surface_kwarg() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            RESPONSES_RESPONSE_WITH_CALL,
            "gpt-4",
            &[],
            Some(&[search_tool()]),
            false,
            "openai",
            Some(RESPONSES_AGENTIC_SURFACE),
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "responses");
}

#[test]
fn dispatcher_defaults_to_anthropic_gate_for_absent_surface() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            "claude",
            &[],
            Some(&[search_tool()]),
            false,
            "anthropic",
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "anthropic");
}

#[test]
fn dispatcher_defaults_to_anthropic_gate_for_unrecognized_surface() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            "claude",
            &[],
            Some(&[search_tool()]),
            false,
            "anthropic",
            Some("unknown"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "anthropic");
}

#[test]
fn anthropic_gate_extracts_thinking_blocks_from_response_content() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    let thinking_blocks = dict.thinking_blocks.unwrap();
    assert_eq!(thinking_blocks.len(), 1);
    assert_eq!(thinking_blocks[0]["type"], "thinking");
}

#[test]
fn anthropic_gate_extracts_redacted_thinking_blocks() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_REDACTED_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    let thinking_blocks = dict.thinking_blocks.unwrap();
    assert_eq!(thinking_blocks.len(), 1);
    assert_eq!(thinking_blocks[0]["type"], "redacted_thinking");
}

#[test]
fn anthropic_gate_returns_empty_thinking_blocks_when_none_present() {
    let state = WebSearchInterceptionState::default();
    let response = r#"{"content":[
        {"type":"tool_use","id":"call-1","name":"litellm_web_search","input":{"query":"rust"}}
    ]}"#;
    let dict = state
        .anthropic_gate(response, Some(&[search_tool()]), false, "anthropic")
        .unwrap()
        .unwrap();
    assert_eq!(dict.thinking_blocks, Some(Vec::new()));
}

#[test]
fn chat_completion_gate_detects_tool_calls_via_openai_dialect() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.tool_calls.len(), 1);
    assert_eq!(dict.response_format, "openai");
}

#[test]
fn chat_completion_gate_rejects_native_anthropic_tool_type() {
    let state = WebSearchInterceptionState::default();
    let result = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[native_search_tool()]),
            false,
            "openai",
        )
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn responses_gate_detects_tool_calls_via_responses_dialect() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .responses_gate(
            RESPONSES_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.tool_calls.len(), 1);
    assert_eq!(dict.response_format, "responses");
}

#[test]
fn responses_gate_accepts_native_web_search_prefixed_tool_type() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .responses_gate(
            RESPONSES_RESPONSE_WITH_CALL,
            Some(&[native_search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "responses");
}

#[test]
fn provider_filter_blocks_a_provider_outside_the_enabled_list() {
    let state = WebSearchInterceptionState {
        enabled_providers: Some(vec!["bedrock".to_owned()]),
    };
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn provider_filter_allows_a_provider_present_in_the_enabled_list() {
    let state = WebSearchInterceptionState {
        enabled_providers: Some(vec!["anthropic".to_owned()]),
    };
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap();
    assert!(result.is_some());
}

#[test]
fn provider_filter_allows_every_provider_when_unset() {
    let state = WebSearchInterceptionState::default();
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "any_provider",
        )
        .unwrap();
    assert!(result.is_some());
}

#[test]
fn tool_check_requires_a_matching_web_search_tool_per_gate() {
    let state = WebSearchInterceptionState::default();
    assert!(
        state
            .anthropic_gate(ANTHROPIC_RESPONSE_WITH_THINKING, None, false, "anthropic")
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .anthropic_gate(
                ANTHROPIC_RESPONSE_WITH_THINKING,
                Some(&[]),
                false,
                "anthropic"
            )
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .chat_completion_gate(
                CHAT_RESPONSE_WITH_CALL,
                Some(&[native_search_tool()]),
                false,
                "openai"
            )
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .responses_gate(RESPONSES_RESPONSE_WITH_CALL, None, false, "openai")
            .unwrap()
            .is_none()
    );
}

#[test]
fn tool_check_blocks_a_response_with_no_tool_use_even_with_a_search_tool_offered() {
    let state = WebSearchInterceptionState::default();
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_NO_TOOL,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn return_format_includes_required_keys_for_every_gate() {
    let state = WebSearchInterceptionState::default();
    let anthropic = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    assert_eq!(anthropic.tool_type, "websearch");
    assert_eq!(anthropic.provider, "anthropic");
    assert_eq!(anthropic.response_format, "anthropic");
    assert!(anthropic.thinking_blocks.is_some());

    let chat = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(chat.tool_type, "websearch");
    assert_eq!(chat.provider, "openai");
    assert_eq!(chat.response_format, "openai");
    assert!(chat.thinking_blocks.is_none());

    let responses = state
        .responses_gate(
            RESPONSES_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(responses.tool_type, "websearch");
    assert_eq!(responses.response_format, "responses");
    assert!(responses.thinking_blocks.is_none());
}

#[test]
fn return_format_omits_thinking_blocks_keys_for_chat_and_responses_serialization() {
    let state = WebSearchInterceptionState::default();
    let chat = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    let encoded_chat = serde_json::to_value(&chat).unwrap();
    assert!(encoded_chat.get("thinking_blocks").is_none());

    let anthropic = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    let encoded_anthropic = serde_json::to_value(&anthropic).unwrap();
    assert!(encoded_anthropic.get("thinking_blocks").is_some());
}
