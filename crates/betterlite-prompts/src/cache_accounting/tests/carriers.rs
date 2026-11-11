use rstest::rstest;

use crate::cache_accounting::carriers::{
    accepts_cache_breakpoint, carries_cache_breakpoint, drops_tool_cache_control,
    tool_carries_cache_breakpoint,
};
use crate::cache_accounting::tests::support::json;

#[rstest]
#[case::cache_control(r#"{"cache_control": {"type": "ephemeral"}}"#, true)]
#[case::prompt_cache_breakpoint(r#"{"prompt_cache_breakpoint": {"mode": "explicit"}}"#, true)]
#[case::false_is_a_value(r#"{"cache_control": false}"#, true)]
#[case::empty_object_value(r#"{"cache_control": {}}"#, true)]
#[case::null_value(r#"{"cache_control": null}"#, false)]
#[case::both_null(r#"{"cache_control": null, "prompt_cache_breakpoint": null}"#, false)]
#[case::other_key(r#"{"type": "text"}"#, false)]
#[case::empty_object("{}", false)]
#[case::list("[]", false)]
#[case::text(r#""cache_control""#, false)]
#[case::null("null", false)]
fn carries_cache_breakpoint_needs_a_non_null_key(#[case] block: &str, #[case] expected: bool) {
    assert_eq!(carries_cache_breakpoint(&json(block)), expected);
}

#[rstest]
#[case::top_level(r#"{"cache_control": {"type": "ephemeral"}}"#, true)]
#[case::inside_function(r#"{"type": "function", "function": {"cache_control": {}}}"#, true)]
#[case::function_null(r#"{"type": "function", "function": null}"#, false)]
#[case::function_not_an_object(r#"{"type": "function", "function": "x"}"#, false)]
#[case::no_mark(r#"{"type": "function", "function": {"name": "weather"}}"#, false)]
#[case::not_an_object("[]", false)]
fn tool_carries_cache_breakpoint_reads_the_function_object(
    #[case] tool: &str,
    #[case] expected: bool,
) {
    assert_eq!(tool_carries_cache_breakpoint(&json(tool)), expected);
}

#[rstest]
#[case::regex_tool(r#"{"type": "tool_search_tool_regex_20251119"}"#, true)]
#[case::bm25_tool(r#"{"type": "tool_search_tool_bm25_20251119"}"#, true)]
#[case::function_tool(r#"{"type": "function"}"#, false)]
#[case::missing_type("{}", false)]
#[case::not_an_object("null", false)]
fn drops_tool_cache_control_only_for_the_search_tool_types(
    #[case] tool: &str,
    #[case] expected: bool,
) {
    assert_eq!(drops_tool_cache_control(&json(tool)), expected);
}

#[rstest]
#[case::text(r#"{"type": "text", "text": "hi"}"#, true)]
#[case::image(r#"{"type": "image"}"#, true)]
#[case::image_url(r#"{"type": "image_url"}"#, true)]
#[case::file(r#"{"type": "file"}"#, true)]
#[case::input_audio(r#"{"type": "input_audio"}"#, true)]
#[case::input_text(r#"{"type": "input_text"}"#, true)]
#[case::input_image(r#"{"type": "input_image"}"#, true)]
#[case::input_file(r#"{"type": "input_file"}"#, true)]
#[case::document(r#"{"type": "document"}"#, false)]
#[case::thinking(r#"{"type": "thinking"}"#, false)]
#[case::tool_use(r#"{"type": "tool_use"}"#, false)]
#[case::missing_type("{}", false)]
#[case::not_an_object("[]", false)]
fn accepts_cache_breakpoint_holds_a_closed_set_of_block_types(
    #[case] block: &str,
    #[case] expected: bool,
) {
    assert_eq!(accepts_cache_breakpoint(&json(block)), expected);
}
