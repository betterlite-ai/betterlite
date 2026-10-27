#![cfg(test)]

use crate::records::{SearchFailed, SearchOutcome, SearchSucceeded, WebSearchToolResultErrorCode};
use crate::tools::{
    OpenAiFunctionName, SearchInputProperties, SearchInputSchema, SearchQueriesProperty,
    StringProperty, StringType, Tool, get_litellm_web_search_tool,
    get_litellm_web_search_tool_openai, get_litellm_web_search_tool_responses, is_web_search_tool,
    is_web_search_tool_chat_completion, is_web_search_tool_responses,
};

#[test]
fn web_search_builders_have_expected_dialects() {
    let anthropic = serde_json::to_value(get_litellm_web_search_tool()).unwrap();
    let openai = serde_json::to_value(get_litellm_web_search_tool_openai()).unwrap();
    let responses = serde_json::to_value(get_litellm_web_search_tool_responses()).unwrap();
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
    assert!(is_web_search_tool_chat_completion(&tool(
        None,
        Some("litellm_web_search"),
        None,
        false
    )));
    assert!(is_web_search_tool_chat_completion(&tool(
        Some("function"),
        None,
        Some("litellm_web_search"),
        false
    )));
    assert!(!is_web_search_tool_chat_completion(&tool(
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
