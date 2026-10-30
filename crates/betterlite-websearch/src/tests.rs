#![cfg(test)]

use crate::call_transformation::{SearchErrorKind, WebSearchTransformation};
use crate::records::{SearchFailed, SearchOutcome, SearchSucceeded, WebSearchToolResultErrorCode};
use crate::tools::{
    OpenAiFunctionName, SearchInputProperties, SearchInputSchema, SearchQueriesProperty,
    StringProperty, StringType, Tool, get_litellm_web_search_tool, is_chat_web_search_tool,
    is_web_search_tool, is_web_search_tool_responses, openai_web_search_tool,
    responses_web_search_tool,
};
use betterlite_openai::search::records::{SearchResponse, SearchResult};

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
    let response = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "reason"},
            {"type": "tool_use", "id": "call-1", "name": "web_search", "input": {"query": "rust"}}
        ]
    });
    let (found, calls) = WebSearchTransformation::transform_request(&response, false, "anthropic");
    assert!(found);
    assert_eq!(calls[0]["id"], "call-1");
    assert_eq!(calls[0]["input"]["query"], "rust");
    let (assistant, user) = WebSearchTransformation::transform_response(
        &calls,
        &[String::from(
            "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe",
        )],
        "anthropic",
        Some(&[serde_json::json!({"type": "thinking", "thinking": "reason"})]),
    );
    assert_eq!(assistant["content"][0]["type"], "thinking");
    assert_eq!(assistant["content"][1]["type"], "tool_use");
    assert_eq!(user["content"][0]["tool_use_id"], "call-1");
}

#[test]
fn call_transformation_openai_dict_and_string_arguments() {
    let response = serde_json::json!({"choices": [{"message": {"tool_calls": [
        {"id": "call-1", "type": "function", "function": {"name": "litellm_web_search", "arguments": "{\"query\":\"rust\"}"}},
        {"id": "call-2", "type": "function", "function": {"name": "web_search", "arguments": {"query": "cargo"}}}
    ]}}]});
    let (found, calls) = WebSearchTransformation::transform_request(&response, false, "openai");
    assert!(found);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["input"]["query"], "rust");
    let (assistant, tools) = WebSearchTransformation::transform_response(
        &calls,
        &[String::from("one"), String::from("two")],
        "openai",
        None,
    );
    assert_eq!(
        assistant["tool_calls"][0]["function"]["arguments"],
        "{\"query\":\"rust\"}"
    );
    assert_eq!(tools[0]["tool_call_id"], "call-1");
    assert_eq!(tools[1]["tool_call_id"], "call-2");
}

#[test]
fn call_transformation_responses_and_no_tool() {
    let response = serde_json::json!({"output": [
        {"type": "web_search_call", "name": "web_search_preview"},
        {"type": "function_call", "call_id": "call-1", "name": "litellm_web_search", "arguments": {"query": "rust"}}
    ]});
    let (found, calls) = WebSearchTransformation::transform_request(&response, false, "responses");
    assert!(found);
    assert_eq!(calls[0]["type"], "function_call");
    assert_eq!(calls[0]["input"]["query"], "rust");
    assert_eq!(
        WebSearchTransformation::transform_request(
            &serde_json::json!({"output": []}),
            false,
            "responses"
        ),
        (false, Vec::new())
    );
    assert_eq!(
        WebSearchTransformation::transform_request(&response, true, "responses"),
        (false, Vec::new())
    );
}

#[test]
fn call_transformation_result_blocks_error_codes_and_format_search_response() {
    let response = SearchResponse {
        results: vec![SearchResult {
            title: String::from("Rust"),
            url: String::from("https://rust-lang.org"),
            snippet: String::from("safe"),
            date: Some(String::from("today")),
            last_updated: None,
            extra: Default::default(),
        }],
        object: Default::default(),
        extra: Default::default(),
        hidden_params: Default::default(),
    };
    let block = WebSearchTransformation::result_block("call-1", Some(&response));
    assert_eq!(block["tool_use_id"], "call-1");
    assert_eq!(block["content"][0]["page_age"], "today");
    assert_eq!(block["content"][0]["encrypted_content"], "");
    assert_eq!(
        WebSearchTransformation::format_search_response(&response),
        "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe"
    );
    assert_eq!(
        WebSearchTransformation::search_error_code(SearchErrorKind::RateLimit),
        WebSearchToolResultErrorCode::TooManyRequests
    );
}
