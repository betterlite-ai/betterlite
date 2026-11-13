#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use uuid::Uuid;

use super::{mcp_slug, resolve_call_id};
use crate::panw_prisma_airs::audit::Direction;
use crate::prisma_airs::request_data::{LoggingObject, RequestData};
use crate::prisma_airs::support::{fixed_id, logging, request_data};

fn resolve(
    data: &mut RequestData,
    logging_obj: Option<&LoggingObject>,
    direction: Direction,
) -> String {
    resolve_call_id(data, logging_obj, direction, fixed_id)
}

#[test]
fn the_request_call_id_wins_over_the_logging_object() {
    let mut data = request_data();
    data.litellm_call_id = Some("from-request".to_owned());
    let mut logging_obj = logging();
    logging_obj.litellm_call_id = Some("from-logging".to_owned());
    assert_eq!(
        resolve(&mut data, Some(&logging_obj), Direction::Request),
        "from-request"
    );
    assert_eq!(data.litellm_call_id.as_deref(), Some("from-request"));
}

#[test]
fn the_logging_call_id_is_used_without_being_copied_into_the_request() {
    let mut data = request_data();
    let mut logging_obj = logging();
    logging_obj.litellm_call_id = Some("from-logging".to_owned());
    assert_eq!(
        resolve(&mut data, Some(&logging_obj), Direction::Request),
        "from-logging"
    );
    assert_eq!(data.litellm_call_id, None);
}

#[test]
fn an_empty_request_call_id_falls_through_to_the_logging_object() {
    let mut data = request_data();
    data.litellm_call_id = Some(String::new());
    let mut logging_obj = logging();
    logging_obj.litellm_call_id = Some("from-logging".to_owned());
    assert_eq!(
        resolve(&mut data, Some(&logging_obj), Direction::Request),
        "from-logging"
    );
}

#[test]
fn a_missing_id_is_synthesized_and_stored_on_the_request() {
    let mut data = request_data();
    let call_id = resolve(&mut data, None, Direction::Request);
    assert_eq!(call_id, Uuid::nil().to_string());
    assert_eq!(data.litellm_call_id.as_deref(), Some(call_id.as_str()));
}

#[test]
fn an_mcp_tool_call_gets_a_slugged_id() {
    let mut data = request_data();
    data.mcp_tool_name = Some("My Tool!".to_owned());
    assert_eq!(
        resolve(&mut data, None, Direction::Request),
        format!("my-tool-{}", Uuid::nil())
    );
}

#[test]
fn an_empty_tool_name_falls_back_to_the_name_field() {
    let mut data = request_data();
    data.mcp_tool_name = Some(String::new());
    data.name = Some("search".to_owned());
    data.arguments = Some(GuardrailValue::Null);
    assert_eq!(
        resolve(&mut data, None, Direction::Request),
        format!("search-{}", Uuid::nil())
    );
}

#[test]
fn a_whitespace_only_tool_name_falls_back_to_a_plain_uuid() {
    let mut data = request_data();
    data.mcp_tool_name = Some("   ".to_owned());
    assert_eq!(
        resolve(&mut data, None, Direction::Request),
        Uuid::nil().to_string()
    );
}

#[test]
fn the_name_fallback_needs_an_argument_field() {
    let mut without_arguments = request_data();
    without_arguments.name = Some("search".to_owned());
    assert_eq!(
        resolve(&mut without_arguments, None, Direction::Request),
        Uuid::nil().to_string()
    );
    let mut with_arguments = request_data();
    with_arguments.name = Some("search".to_owned());
    with_arguments.arguments = Some(GuardrailValue::Null);
    assert_eq!(
        resolve(&mut with_arguments, None, Direction::Request),
        format!("search-{}", Uuid::nil())
    );
}

#[test]
fn a_response_never_uses_the_tool_slug() {
    let mut data = request_data();
    data.mcp_tool_name = Some("search".to_owned());
    assert_eq!(
        resolve(&mut data, None, Direction::Response),
        Uuid::nil().to_string()
    );
}

#[test]
fn a_logging_object_skips_the_tool_slug() {
    let mut data = request_data();
    data.mcp_tool_name = Some("search".to_owned());
    assert_eq!(
        resolve(&mut data, Some(&logging()), Direction::Request),
        Uuid::nil().to_string()
    );
}

#[test]
fn two_synthesized_ids_differ() {
    let first = resolve_call_id(&mut request_data(), None, Direction::Request, Uuid::new_v4);
    let second = resolve_call_id(&mut request_data(), None, Direction::Request, Uuid::new_v4);
    assert_ne!(first, second);
}

#[rstest]
#[case::spaces("Get Weather", "get-weather")]
#[case::runs("a--b", "a-b")]
#[case::padded("  spaced  ", "spaced")]
#[case::nothing_usable("***", "mcp-tool")]
#[case::digits("tool 42", "tool-42")]
#[case::unicode("\u{dc}n\u{ef}code", "n-code")]
fn slugs_collapse_everything_but_ascii_alphanumerics(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(mcp_slug(name), expected);
}

#[test]
fn a_long_slug_is_truncated_to_forty_characters() {
    assert_eq!(mcp_slug(&"a".repeat(50)), "a".repeat(40));
}
