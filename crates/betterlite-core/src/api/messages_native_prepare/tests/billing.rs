use rstest::rstest;

use super::support::{json, object};
use crate::api::messages_native_prepare::billing::{filter_billing_headers, remove_cache_scope};

#[rstest]
#[case::string_block(r#""x-anthropic-billing-header: cc_version=1""#, None)]
#[case::leading_whitespace(
    r#"" x-anthropic-billing-header: cc_version=1""#,
    Some(r#"" x-anthropic-billing-header: cc_version=1""#)
)]
#[case::plain_string(r#""be brief""#, Some(r#""be brief""#))]
#[case::null_value("null", None)]
#[case::number_value("17", Some("17"))]
fn billing_and_cache_fields_keeps_or_drops_a_system_value(
    #[case] system: &str,
    #[case] expected: Option<&str>,
) {
    let filtered = filter_billing_headers(&json(system));
    assert_eq!(filtered, expected.map(json));
}

#[rstest]
#[case::only_blocks(
    r#"[{"type":"text","text":"x-anthropic-billing-header: a"},{"type":"text","text":"x-anthropic-billing-header: b"}]"#,
    None
)]
#[case::mixed_blocks(
    r#"[{"type":"text","text":"x-anthropic-billing-header: a"},{"type":"text","text":"keep"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}},{"type":"text","text":"x-anthropic-billing-header:"}]"#,
    Some(r#"[{"type":"text","text":"keep"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}]"#)
)]
#[case::non_dict_members(
    r#"["x-anthropic-billing-header: a",{"type":"text","text":"keep"}]"#,
    Some(r#"["x-anthropic-billing-header: a",{"type":"text","text":"keep"}]"#)
)]
#[case::text_without_prefix(
    r#"[{"type":"text","text":"x-anthropic-billing-header"}]"#,
    Some(r#"[{"type":"text","text":"x-anthropic-billing-header"}]"#)
)]
#[case::empty_list("[]", None)]
fn billing_and_cache_fields_filters_list_blocks(
    #[case] system: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(filter_billing_headers(&json(system)), expected.map(json));
}

#[rstest]
#[case::system_list(
    r#"{"system":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","scope":"global","ttl":"1h"}}]}"#,
    r#"{"system":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","ttl":"1h"}}]}"#
)]
#[case::message_content(
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"q","cache_control":{"type":"ephemeral","scope":"global"}}]}]}"#,
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"q","cache_control":{"type":"ephemeral"}}]}]}"#
)]
#[case::other_scope_value(
    r#"{"system":[{"type":"text","text":"s","cache_control":{"scope":"request"}}]}"#,
    r#"{"system":[{"type":"text","text":"s","cache_control":{}}]}"#
)]
#[case::message_level_control(
    r#"{"messages":[{"role":"user","content":"q","cache_control":{"type":"ephemeral","scope":"global"}}]}"#,
    r#"{"messages":[{"role":"user","content":"q","cache_control":{"type":"ephemeral","scope":"global"}}]}"#
)]
#[case::tool_control(
    r#"{"tools":[{"name":"lookup","cache_control":{"type":"ephemeral","scope":"global"}}]}"#,
    r#"{"tools":[{"name":"lookup","cache_control":{"type":"ephemeral","scope":"global"}}]}"#
)]
#[case::string_content(
    r#"{"messages":[{"role":"user","content":"q"}],"system":"s"}"#,
    r#"{"messages":[{"role":"user","content":"q"}],"system":"s"}"#
)]
#[case::non_dict_control(
    r#"{"system":[{"type":"text","text":"s","cache_control":"ephemeral"}]}"#,
    r#"{"system":[{"type":"text","text":"s","cache_control":"ephemeral"}]}"#
)]
fn billing_and_cache_fields_removes_only_the_scope(#[case] request: &str, #[case] expected: &str) {
    let mut given = object(request);
    remove_cache_scope(&mut given);
    assert_eq!(given, object(expected));
}
