#![cfg(test)]

use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::redact::{self, ContentItemUpdate};

#[test]
fn set_text_updates_content_list_and_structured_mirror() {
    let response = Json::parse(
        r#"{"content": [{"type": "text", "text": "old"}], "structuredContent": {"result": "old"}}"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    let content = update.content.expect("content present");
    assert_eq!(content.len(), 1);
    assert!(matches!(&content[0], ContentItemUpdate::TextReplaced { text, .. } if text == "new"));
    assert_eq!(
        update.structured_content.expect("mirror present").result,
        "new"
    );
}

#[test]
fn set_text_recurses_through_tool_call_response_wrapper() {
    let response = Json::parse(
        r#"{"mcp_tool_call_response": {"content": [{"type": "text", "text": "old"}], "structured_content": {}}}"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_eq!(update.structured_content.unwrap().result, "new");
}

#[test]
fn set_text_leaves_non_text_items_unchanged() {
    let response = Json::parse(r#"{"content": [{"type": "image", "data": "abc"}]}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(!update.replaced);
    let content = update.content.expect("content present");
    assert!(matches!(&content[0], ContentItemUpdate::Unchanged(_)));
}

#[test]
fn set_text_updates_result_dict_structured_mirror_without_content() {
    let response = Json::parse(r#"{"result": {"structured_content": {"result": "old"}}}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.content.is_none());
    assert!(update.replaced);
    assert_eq!(update.structured_content.unwrap().result, "new");
}

#[test]
fn set_text_updates_pair_list_structured_mirror() {
    let response = Json::parse(
        r#"[["content", [{"type": "text", "text": "old"}]], ["structuredContent", {"result": "old"}]]"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    let content = update.content.expect("content present");
    assert!(matches!(&content[0], ContentItemUpdate::TextReplaced { text, .. } if text == "new"));
    assert_eq!(update.structured_content.unwrap().result, "new");
}

#[test]
fn set_text_is_not_replaced_when_nothing_matches() {
    let response = Json::parse(r#"{"other": 1}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(!update.replaced);
    assert!(update.content.is_none());
    assert!(update.structured_content.is_none());
}

#[test]
fn coerce_to_content_list_treats_bare_list_as_content() {
    let response = Json::parse(r#"[{"type": "text", "text": "hi"}]"#);
    let content = redact::coerce_to_content_list(&response).expect("content list");
    assert_eq!(content.len(), 1);
}

#[test]
fn coerce_to_content_list_reads_nested_pair_list_content() {
    let response =
        Json::parse(r#"[["content", [{"type": "text", "text": "hi"}]], ["isError", false]]"#);
    let content = redact::coerce_to_content_list(&response).expect("content list");
    assert_eq!(content.len(), 1);
}

#[test]
fn coerce_to_content_list_none_when_absent() {
    let response = Json::parse(r#"{"other": 1}"#);
    assert!(redact::coerce_to_content_list(&response).is_none());
}

#[test]
fn replace_text_items_only_touches_text_type_items() {
    let response =
        Json::parse(r#"[{"type": "text", "text": "old"}, {"type": "image", "data": "x"}]"#);
    let Json::List(items) = response else {
        panic!("expected a list")
    };
    let updated = redact::replace_text_items(&items, "new");
    assert!(matches!(&updated[0], ContentItemUpdate::TextReplaced { text, .. } if text == "new"));
    assert!(matches!(&updated[1], ContentItemUpdate::Unchanged(_)));
}
