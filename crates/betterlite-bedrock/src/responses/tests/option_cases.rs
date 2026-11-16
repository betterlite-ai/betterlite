use super::support::{MODEL, capture, map, mapped, object};
use crate::responses::config::SUPPORTS_NATIVE_FILE_SEARCH;

const BACKGROUND_WARNING: &str = "Bedrock Runtime Responses API: dropping unsupported parameter(s) ('background',) that the endpoint rejects.";
const SUPPORTED: &str =
    "['apply_patch', 'computer', 'custom', 'function', 'mcp', 'namespace', 'tool_search']";

fn tool_warning(dropped: &str) -> String {
    format!(
        "Bedrock Runtime Responses API: dropping unsupported tool type(s) {dropped} (supported: {SUPPORTED})."
    )
}

pub(super) fn response_option_gates() {
    background_cases();
    tool_cases();
}

fn background_cases() {
    let (messages, _guard) = capture();
    let kept = mapped(r#"{"background": false, "temperature": 0.5}"#);
    assert_eq!(kept, object(r#"{"temperature": 0.5}"#));
    let nulled = mapped(r#"{"background": null}"#);
    assert_eq!(nulled, object("{}"));
    let always = map(MODEL, r#"{"background": true}"#, true).expect("drop_params maps");
    assert_eq!(always, object("{}"));
    let never = map(MODEL, r#"{"background": true}"#, false).expect("no drop_params maps");
    assert_eq!(never, object("{}"));
    assert_eq!(messages.warnings(), vec![BACKGROUND_WARNING.to_owned(); 4]);
    let without = mapped(r#"{"temperature": 0.5}"#);
    assert_eq!(without, object(r#"{"temperature": 0.5}"#));
    assert_eq!(messages.warnings().len(), 4);
}

fn tool_cases() {
    let (messages, _guard) = capture();
    let untouched = [r#"{"tools": 0}"#, r#"{"tools": null}"#, r#"{"tools": []}"#];
    for source in untouched {
        assert_eq!(mapped(source), object(source));
    }
    let every_supported = r#"{"tools": [{"type": "function"}, {"type": "mcp"}, {"type": "custom"}, {"type": "apply_patch"}, {"type": "namespace"}, {"type": "tool_search"}, {"type": "computer"}]}"#;
    assert_eq!(mapped(every_supported), object(every_supported));
    assert!(messages.warnings().is_empty());
    let mixed = mapped(
        r#"{"tools": [{"type": "function", "name": "first"}, 7, {"type": "web_search"}, {"name": "typeless"}, {"type": "mcp", "server_label": "s"}, {"type": "web_search"}], "temperature": 0.5}"#,
    );
    let expected = object(
        r#"{"temperature": 0.5, "tools": [{"type": "function", "name": "first"}, 7, {"type": "mcp", "server_label": "s"}]}"#,
    );
    assert_eq!(mixed, expected);
    assert_eq!(
        messages.warnings(),
        vec![tool_warning("['None', 'web_search']")]
    );
    let emptied = mapped(r#"{"tools": [{"type": "code_interpreter"}], "temperature": 0.5}"#);
    assert_eq!(emptied, object(r#"{"temperature": 0.5}"#));
    assert_eq!(messages.warnings().len(), 2);
}

pub(super) fn native_file_search_flag() {
    assert!(!SUPPORTS_NATIVE_FILE_SEARCH);
    let (messages, _guard) = capture();
    let dropped = mapped(
        r#"{"tools": [{"type": "file_search", "vector_store_ids": ["vs"]}, {"type": "web_search"}]}"#,
    );
    assert_eq!(dropped, object("{}"));
    assert_eq!(
        messages.warnings(),
        vec![tool_warning("['file_search', 'web_search']")]
    );
}
