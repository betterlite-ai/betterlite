use tracing::Level;

use crate::mantle_responses::repr::quoted_list;
use crate::mantle_responses::tools::{
    SUPPORTED_TOOL_TYPES, SUPPORTED_TOOL_TYPES_REPR, filter_unsupported_tools,
};

use super::support;

const MODEL: &str = "openai.gpt-oss-120b";

const TOOL_WARNING: &str = "Bedrock Mantle Responses API: dropping unsupported tool type(s) ['None', 'web_search_preview'] (supported: ['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']).";

pub(super) fn mantle_tool_filter() {
    allowlist_cases();
    drop_cases();
    pass_through_cases();
    mapped_cases();
}

fn allowlist_cases() {
    let all = support::items(
        r#"[{"type": "web_search"}, {"type": "tool_search"}, {"type": "namespace"}, {"type": "mcp"}, {"type": "custom"}, {"type": "function", "name": "kept"}]"#,
    );
    let (messages, _guard) = support::capture();
    let filtered = filter_unsupported_tools(&all).expect("every entry filters");
    assert_eq!(filtered, all);
    assert!(messages.at(Level::WARN).is_empty());
    let names: Vec<String> = SUPPORTED_TOOL_TYPES.map(str::to_owned).to_vec();
    assert_eq!(quoted_list(&names), SUPPORTED_TOOL_TYPES_REPR);
    assert_eq!(
        SUPPORTED_TOOL_TYPES_REPR,
        "['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']"
    );
}

fn drop_cases() {
    let tools = support::items(
        r#"[{"type": "function", "name": "kept"}, {"type": "web_search_preview"}, {"name": "typeless"}, 7, {"type": "function", "name": "second"}]"#,
    );
    let (messages, _guard) = support::capture();
    let filtered = filter_unsupported_tools(&tools).expect("the list filters");
    assert_eq!(
        filtered,
        support::items(
            r#"[{"type": "function", "name": "kept"}, 7, {"type": "function", "name": "second"}]"#
        )
    );
    assert_eq!(messages.at(Level::WARN), vec![TOOL_WARNING.to_owned()]);
    let duplicate = support::items(
        r#"[{"type": "web_search_preview"}, {"type": "web_search_preview"}, {"type": "code_interpreter"}]"#,
    );
    let emptied = filter_unsupported_tools(&duplicate).expect("the list filters");
    assert!(emptied.is_empty());
    assert_eq!(
        messages.at(Level::WARN),
        vec![
            TOOL_WARNING.to_owned(),
            "Bedrock Mantle Responses API: dropping unsupported tool type(s) ['code_interpreter', 'web_search_preview'] (supported: ['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']).".to_owned(),
        ]
    );
}

fn pass_through_cases() {
    let (messages, _guard) = support::capture();
    let scalars = support::items(r#"[7, "text", null, [1]]"#);
    let filtered = filter_unsupported_tools(&scalars).expect("non-objects pass");
    assert_eq!(filtered, scalars);
    assert!(messages.at(Level::WARN).is_empty());
    let typeless = support::items(r#"[{"nested": true}]"#);
    let dropped = filter_unsupported_tools(&typeless).expect("objects without a type drop");
    assert!(dropped.is_empty());
    assert_eq!(
        messages.at(Level::WARN),
        vec!["Bedrock Mantle Responses API: dropping unsupported tool type(s) ['None'] (supported: ['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']).".to_owned()]
    );
}

fn mapped_cases() {
    let (messages, _guard) = support::capture();
    let dropped = support::mapped(
        MODEL,
        r#"{"tools": [{"type": "function", "name": "kept"}, {"type": "web_search_preview"}]}"#,
        false,
    );
    assert_eq!(
        dropped,
        support::object(r#"{"tools": [{"type": "function", "name": "kept"}]}"#)
    );
    assert_eq!(
        messages.at(Level::WARN),
        vec![
            "Bedrock Mantle Responses API: dropping unsupported tool type(s) ['web_search_preview'] (supported: ['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']).".to_owned(),
        ]
    );
    mapped_shape_cases();
}

fn mapped_shape_cases() {
    assert_eq!(
        support::mapped(MODEL, r#"{"tools": []}"#, false),
        support::object(r#"{"tools": []}"#)
    );
    assert_eq!(
        support::mapped(MODEL, r#"{"tools": null}"#, false),
        support::object(r#"{"tools": null}"#)
    );
    assert_eq!(
        support::mapped(MODEL, r#"{"tools": 0}"#, false),
        support::object(r#"{"tools": 0}"#)
    );
    let single = support::mapped(
        MODEL,
        r#"{"tools": {"type": "function", "name": "one"}}"#,
        false,
    );
    assert_eq!(
        single,
        support::object(r#"{"tools": [{"type": "function", "name": "one"}]}"#)
    );
    let gone = support::mapped(MODEL, r#"{"tools": {"type": "code_interpreter"}}"#, false);
    assert_eq!(gone, support::object("{}"));
    let listed_gone = support::mapped(
        MODEL,
        r#"{"tools": [{"type": "code_interpreter"}, {"type": "file_search"}]}"#,
        false,
    );
    assert_eq!(listed_gone, support::object("{}"));
}
