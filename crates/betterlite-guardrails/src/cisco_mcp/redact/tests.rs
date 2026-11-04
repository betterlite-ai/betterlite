#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::redact;

fn assert_response(response: &redact::RewrittenResponse, expected: &str) {
    let expected_raw = RawValue::from_string(expected.to_owned()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::to_value(expected_raw).unwrap()
    );
}

#[test]
fn set_text_mirrors_model_without_structured_content() {
    let response = Json::parse(r#"{"content":[{"type":"text","text":"old"}]}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_response(
        &update.response,
        r#"{"content":[{"type":"text","text":"new"}],"structuredContent":{"result":"new"}}"#,
    );
}

#[test]
fn set_text_leaves_non_text_items_unchanged() {
    let response = Json::parse(r#"{"content":[{"type":"image","data":"abc"}]}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_response(
        &update.response,
        r#"{"content":[{"type":"image","data":"abc"}],"structuredContent":{"result":"new"}}"#,
    );
}

#[test]
fn set_text_rewrites_mirror_under_result() {
    let response =
        Json::parse(r#"{"jsonrpc":"2.0","id":1,"result":{"structuredContent":{"result":"old"}}}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_response(
        &update.response,
        r#"{"jsonrpc":"2.0","id":1,"result":{"structuredContent":{"result":"new"}}}"#,
    );
}

#[test]
fn set_text_keeps_wrapper_shape() {
    let response = Json::parse(
        r#"{"mcp_tool_call_response":{"content":[{"type":"text","text":"old"}],"structuredContent":{"result":"old"}}}"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_response(
        &update.response,
        r#"{"mcp_tool_call_response":{"content":[{"type":"text","text":"new"}],"structuredContent":{"result":"new"}}}"#,
    );
}

#[test]
fn set_text_keeps_pair_list_shape() {
    let response = Json::parse(
        r#"[["content",[{"type":"text","text":"old"}]],["structuredContent",{"result":"old"}],["isError",false]]"#,
    );
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_response(
        &update.response,
        r#"[["content",[{"type":"text","text":"new"}]],["structuredContent",{"result":"new"}],["isError",false]]"#,
    );
}

#[test]
fn set_text_rewrites_bare_content_list() {
    let response = Json::parse(r#"[{"type":"text","text":"old"}]"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(update.replaced);
    assert_response(&update.response, r#"[{"type":"text","text":"new"}]"#);
}

#[test]
fn set_text_keeps_response_without_matching_target() {
    let response = Json::parse(r#"{"other":1}"#);
    let update = redact::set_tool_response_text(&response, "new");
    assert!(!update.replaced);
    assert_response(&update.response, r#"{"other":1}"#);
}

#[rstest]
#[case::all_pair_mirrors(
    r#"[["structuredContent",{}],["structured_content",{}],["structuredContent",{}]]"#,
    r#"[["structuredContent",{"result":"new"}],["structured_content",{"result":"new"}],["structuredContent",{"result":"new"}]]"#,
    true
)]
#[case::last_content_pair(
    r#"[["content",[{"type":"text","text":"first"}]],["content",[{"type":"text","text":"last"}]]]"#,
    r#"[["content",[{"type":"text","text":"first"}]],["content",[{"type":"text","text":"new"}]]]"#,
    true
)]
#[case::model_snake_mirror(
    r#"{"content":[],"structured_content":null,"structuredContent":{"keep":1},"isError":false}"#,
    r#"{"content":[],"structured_content":{"result":"new"},"structuredContent":{"keep":1},"isError":false}"#,
    true
)]
#[case::dict_prefers_snake_mirror(
    r#"{"structured_content":{},"structuredContent":{"keep":1}}"#,
    r#"{"structured_content":{"result":"new"},"structuredContent":{"keep":1}}"#,
    true
)]
#[case::result_hides_outer_mirror(
    r#"{"result":{},"structuredContent":{"keep":1}}"#,
    r#"{"result":{},"structuredContent":{"keep":1}}"#,
    false
)]
#[case::dict_does_not_rewrite_nested_content(
    r#"{"result":{"content":[{"type":"text","text":"old"}]}}"#,
    r#"{"result":{"content":[{"type":"text","text":"old"}]}}"#,
    false
)]
#[case::non_object_result_uses_outer_mirror(
    r#"{"result":null,"structuredContent":{}}"#,
    r#"{"result":null,"structuredContent":{"result":"new"}}"#,
    true
)]
#[case::wrapped_list_has_no_mirror(
    r#"{"mcp_tool_call_response":[{"type":"text","text":"old"}],"hidden_params":{"response_cost":1}}"#,
    r#"{"mcp_tool_call_response":[{"type":"text","text":"new"}],"hidden_params":{"response_cost":1}}"#,
    true
)]
#[case::null_wrapper_falls_through(
    r#"{"mcp_tool_call_response":null,"content":[]}"#,
    r#"{"mcp_tool_call_response":null,"content":[],"structuredContent":{"result":"new"}}"#,
    true
)]
#[case::text_items_only(
    r#"[{"type":"text","text":"old","annotations":{"priority":1}},{"type":"image","data":"abc"},null]"#,
    r#"[{"type":"text","text":"new","annotations":{"priority":1}},{"type":"image","data":"abc"},null]"#,
    true
)]
#[case::null("null", "null", false)]
#[case::boolean("true", "true", false)]
#[case::number("1.25", "1.25", false)]
#[case::string(r#""old""#, r#""old""#, false)]
#[case::empty_list("[]", "[]", false)]
#[case::pair_without_content(r#"[["isError",false]]"#, r#"[["isError",false]]"#, false)]
#[case::pair_image_no_mirror(
    r#"[["content",[{"type":"image","data":"abc"}]]]"#,
    r#"[["content",[{"type":"image","data":"abc"}]]]"#,
    false
)]
fn set_text_preserves_shapes_and_mirror_rules(
    #[case] input: &str,
    #[case] expected: &str,
    #[case] replaced: bool,
) {
    let update = redact::set_tool_response_text(&Json::parse(input), "new");
    assert_eq!(update.replaced, replaced);
    assert_response(&update.response, expected);
}
