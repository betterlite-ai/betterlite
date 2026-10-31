#![cfg(test)]

use std::fs;
use std::path::Path;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::with_default;

use crate::call_transformation::native::{self, SearchErrorKind};
use crate::call_transformation::wire::{Response, Thinking};
use crate::call_transformation::{TransformationError, WebSearchTransformation};
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
        native::search_error_code(SearchErrorKind::RateLimit),
        WebSearchToolResultErrorCode::TooManyRequests
    );
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
        native::search_error_code(SearchErrorKind::BadRequest),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
    assert_eq!(
        native::search_error_code(SearchErrorKind::Other),
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
