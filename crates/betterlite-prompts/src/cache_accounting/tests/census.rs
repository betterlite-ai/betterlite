use rstest::rstest;

use crate::arguments::json::Json;
use crate::cache_accounting::census::{
    RequestCacheMarks, count_cache_control_blocks, count_external_breakpoints,
    count_request_breakpoints, count_route_breakpoints, points_with_a_slot_left,
    request_has_cache_control, reserved_outside_messages,
};
use crate::cache_accounting::points::{
    CacheControlInjectionPoint, MessageInjectionPoint, ToolConfigInjectionPoint,
};
use crate::cache_accounting::tests::support::json;

fn list(text: &str) -> Vec<Json> {
    serde_json::from_str(text).unwrap()
}

fn optional(text: Option<&str>) -> Option<Json> {
    text.map(json)
}

fn message_point() -> CacheControlInjectionPoint {
    CacheControlInjectionPoint::Message(MessageInjectionPoint {
        role: None,
        index: None,
        control: None,
        openai_dialect: None,
        external_breakpoints: None,
    })
}

fn tool_config_point() -> CacheControlInjectionPoint {
    CacheControlInjectionPoint::ToolConfig(ToolConfigInjectionPoint {
        control: None,
        openai_dialect: None,
        external_breakpoints: None,
    })
}

#[rstest]
#[case::no_messages(&[], None, 0)]
#[case::top_level_mark(&[r#"{"role": "user", "content": "hi", "cache_control": {}}"#], None, 1)]
#[case::one_marked_block(&[r#"{"role": "user", "content": [{"type": "text", "cache_control": {}}, {"type": "text"}]}"#], None, 1)]
#[case::two_marked_blocks(&[r#"{"role": "user", "content": [{"type": "text", "cache_control": {}}, {"type": "image", "prompt_cache_breakpoint": {}}]}"#], None, 2)]
#[case::null_mark(&[r#"{"role": "user", "content": "hi", "cache_control": null}"#], None, 0)]
#[case::string_content(&[r#"{"role": "user", "content": "hi"}"#], None, 0)]
#[case::missing_content(&[r#"{"role": "assistant"}"#], None, 0)]
#[case::not_an_object_message(&["5"], None, 0)]
#[case::two_messages(&[r#"{"cache_control": {}}"#, r#"{"content": [{"cache_control": {}}]}"#], None, 2)]
#[case::marked_system_list(&[], Some(r#"[{"type": "text", "cache_control": {}}, {"type": "text"}]"#), 1)]
#[case::system_string(&[], Some(r#""a system prompt""#), 0)]
#[case::null_system(&[], Some("null"), 0)]
fn count_request_breakpoints_counts_messages_and_system(
    #[case] messages: &[&str],
    #[case] system: Option<&str>,
    #[case] expected: usize,
) {
    let parsed_messages: Vec<Json> = messages.iter().map(|message| json(message)).collect();
    let parsed_system = optional(system);
    assert_eq!(
        count_request_breakpoints(&parsed_messages, parsed_system.as_ref()),
        expected
    );
}

#[test]
fn count_cache_control_blocks_counts_the_message_and_its_blocks() {
    let message = json(
        r#"{"cache_control": {}, "content": [{"cache_control": {}}, {"prompt_cache_breakpoint": {}}, {"type": "text"}]}"#,
    );
    assert_eq!(count_cache_control_blocks(&message), 3);
}

#[rstest]
#[case::nothing(None, None, None, 0)]
#[case::top_level_tool_mark(Some(r#"[{"type": "function", "cache_control": {}}]"#), None, None, 1)]
#[case::function_tool_mark(
    Some(r#"[{"type": "function", "function": {"cache_control": {}}}]"#),
    None,
    None,
    1
)]
#[case::two_marked_tools(
    Some(r#"[{"cache_control": {}}, {"function": {"cache_control": {}}}, {"type": "function"}]"#),
    None,
    None,
    2
)]
#[case::automatic_cache_control(None, Some("{}"), None, 1)]
#[case::null_automatic_cache_control(None, Some("null"), None, 0)]
#[case::envelope_tools_replace_direct_tools(
    Some(r#"[{"cache_control": {}}]"#),
    None,
    Some(r#"{"extra_body": {"tools": []}}"#),
    0
)]
#[case::envelope_tools_not_a_list(
    Some(r#"[{"cache_control": {}}]"#),
    None,
    Some(r#"{"extra_body": {"tools": 5}}"#),
    0
)]
#[case::envelope_adds_marked_tools(
    None,
    None,
    Some(r#"{"extra_body": {"tools": [{"cache_control": {}}]}}"#),
    1
)]
#[case::envelope_without_tools(None, None, Some(r#"{"extra_body": {}}"#), 0)]
#[case::null_envelope_cache_control_suppresses_the_parameter(
    None,
    Some("{}"),
    Some(r#"{"extra_body": {"cache_control": null}}"#),
    0
)]
#[case::envelope_cache_control_counts(
    None,
    None,
    Some(r#"{"extra_body": {"cache_control": {}}}"#),
    1
)]
#[case::envelope_messages_count(None, None, Some(r#"{"extra_body": {"messages": [{"cache_control": {}}, {"content": [{"cache_control": {}}]}]}}"#), 2)]
#[case::envelope_messages_not_a_list(None, None, Some(r#"{"extra_body": {"messages": "x"}}"#), 0)]
#[case::envelope_system_counts(
    None,
    None,
    Some(r#"{"extra_body": {"system": [{"cache_control": {}}]}}"#),
    1
)]
#[case::non_object_envelope(None, Some("{}"), Some(r#"{"extra_body": []}"#), 1)]
#[case::everything_together(
    Some(r#"[{"cache_control": {}}]"#),
    Some("{}"),
    Some(r#"{"extra_body": {"messages": [{"cache_control": {}}]}}"#),
    3
)]
fn count_external_breakpoints_counts_tools_cache_control_and_envelope(
    #[case] tools: Option<&str>,
    #[case] cache_control: Option<&str>,
    #[case] request_kwargs: Option<&str>,
    #[case] expected: usize,
) {
    let tool_list = tools.map(list);
    let control = optional(cache_control);
    let kwargs = optional(request_kwargs);
    assert_eq!(
        count_external_breakpoints(tool_list.as_deref(), control.as_ref(), kwargs.as_ref()),
        expected
    );
}

#[test]
fn count_route_breakpoints_reserves_for_the_larger_census() {
    let direct = list(r#"[{"cache_control": {}}]"#);
    let replaced = json(r#"{"extra_body": {"tools": []}}"#);
    assert_eq!(
        count_route_breakpoints(Some(&direct), None, Some(&replaced)),
        1
    );
    let merged = json(r#"{"extra_body": {"tools": [{"cache_control": {}}]}}"#);
    assert_eq!(count_route_breakpoints(None, None, Some(&merged)), 1);
    assert_eq!(count_route_breakpoints(None, None, None), 0);
    let unmarked = json(r#"{"extra_body": {"tools": [{"type": "function"}]}}"#);
    assert_eq!(count_route_breakpoints(None, None, Some(&unmarked)), 0);
}

#[rstest]
#[case::openai_reserves_nothing(2, true, &[true, true], 0)]
#[case::anthropic_reserves_external_only(2, false, &[false, false], 2)]
#[case::tool_config_reserves_one_more(2, false, &[false, true], 3)]
#[case::zero_external_with_tool_config(0, false, &[true], 1)]
fn reserved_outside_messages_reserves_external_and_tool_config_slots(
    #[case] external: usize,
    #[case] openai_dialect: bool,
    #[case] tool_config_points: &[bool],
    #[case] expected: usize,
) {
    let points: Vec<CacheControlInjectionPoint> = tool_config_points
        .iter()
        .map(|tool_config| {
            if *tool_config {
                tool_config_point()
            } else {
                message_point()
            }
        })
        .collect();
    assert_eq!(
        reserved_outside_messages(&points, external, openai_dialect),
        expected
    );
}

#[test]
fn points_with_a_slot_left_forwards_everything_under_the_cap() {
    let points = vec![message_point(), tool_config_point()];
    assert_eq!(points_with_a_slot_left(&points, 3, false), points);
    assert_eq!(points_with_a_slot_left(&points, 4, true), points);
    assert_eq!(points_with_a_slot_left(&points, 10, true), points);
}

#[test]
fn points_with_a_slot_left_drops_tool_config_points_at_the_cap() {
    let points = vec![message_point(), tool_config_point()];
    assert_eq!(
        points_with_a_slot_left(&points, 4, false),
        vec![message_point()]
    );
}

#[rstest]
#[case::client_message(Some(r#"[{"cache_control": {}}]"#), None, None, false, true)]
#[case::client_tool(None, Some(r#"[{"cache_control": {}}]"#), None, false, true)]
#[case::no_mark(None, None, None, false, false)]
fn request_has_cache_control_sees_every_client_mark(
    #[case] messages: Option<&str>,
    #[case] tools: Option<&str>,
    #[case] cache_control: Option<&str>,
    #[case] on_messages_route: bool,
    #[case] expected: bool,
) {
    let message_list = list(messages.unwrap_or("[]"));
    let tool_list = tools.map(list);
    let control = optional(cache_control);
    let marks = RequestCacheMarks {
        messages: &message_list,
        system: None,
        tools: tool_list.as_deref(),
        cache_control: control.as_ref(),
        request_kwargs: None,
    };
    assert_eq!(
        request_has_cache_control(&marks, on_messages_route),
        expected
    );
}

#[test]
fn request_has_cache_control_takes_the_larger_route_census() {
    let direct = list(r#"[{"cache_control": {}}]"#);
    let request_kwargs = json(r#"{"extra_body": {"tools": []}}"#);
    let marks = RequestCacheMarks {
        messages: &[],
        system: None,
        tools: Some(&direct),
        cache_control: None,
        request_kwargs: Some(&request_kwargs),
    };
    assert!(request_has_cache_control(&marks, true));
    assert!(!request_has_cache_control(&marks, false));
}
