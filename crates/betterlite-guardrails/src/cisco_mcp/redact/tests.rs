#![cfg(test)]

use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::redact;
use serde_json::json;

#[test]
fn set_text_mirrors_model_without_structured_content() {
    let response = Json::parse(r#"{"content":[{"type":"text","text":"old"}]}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_eq!(update.response["content"][0]["text"], "new");
    assert_eq!(
        update.response["structuredContent"],
        json!({"result":"new"})
    );
}

#[test]
fn set_text_leaves_non_text_items_unchanged() {
    let response = Json::parse(r#"{"content":[{"type":"image","data":"abc"}]}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_eq!(update.response["content"][0]["data"], "abc");
    assert_eq!(
        update.response["structuredContent"],
        json!({"result":"new"})
    );
}

#[test]
fn set_text_rewrites_mirror_under_result() {
    let response =
        Json::parse(r#"{"jsonrpc":"2.0","id":1,"result":{"structuredContent":{"result":"old"}}}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert_eq!(
        update.response,
        json!({"jsonrpc":"2.0","id":1,"result":{"structuredContent":{"result":"new"}}})
    );
}

#[test]
fn set_text_keeps_wrapper_shape() {
    let response = Json::parse(
        r#"{"mcp_tool_call_response":{"content":[{"type":"text","text":"old"}],"structuredContent":{"result":"old"}}}"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_eq!(
        update.response["mcp_tool_call_response"]["content"][0]["text"],
        "new"
    );
    assert_eq!(
        update.response["mcp_tool_call_response"]["structuredContent"]["result"],
        "new"
    );
}

#[test]
fn set_text_keeps_pair_list_shape() {
    let response = Json::parse(
        r#"[["content",[{"type":"text","text":"old"}]],["structuredContent",{"result":"old"}],["isError",false]]"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_eq!(update.response[0][1][0]["text"], "new");
    assert_eq!(update.response[1][1], json!({"result":"new"}));
    assert_eq!(update.response[2], json!(["isError", false]));
}

#[test]
fn set_text_rewrites_bare_content_list() {
    let response = Json::parse(r#"[{"type":"text","text":"old"}]"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_eq!(update.response[0]["text"], "new");
}

#[test]
fn set_text_keeps_response_without_matching_target() {
    let response = Json::parse(r#"{"other":1}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(!update.replaced);
    assert_eq!(update.response, json!({"other":1}));
}
