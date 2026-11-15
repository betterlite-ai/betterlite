#![cfg(test)]

mod cases;
mod corpus;
mod support;

use rstest::rstest;

use self::cases::{
    claude3_invoke_stream, deepseek_stream_chunks, empty_tool_input, framing_iteration_and_eof,
    json_mode_stop, json_mode_tool_chunk, openai_compatible_invoke_stream,
    stream_selector_variants,
};

#[rstest]
#[case::claude3_invoke_stream(claude3_invoke_stream)]
#[case::json_mode_tool_chunk(json_mode_tool_chunk)]
#[case::json_mode_stop(json_mode_stop)]
#[case::empty_tool_input(empty_tool_input)]
#[case::deepseek_stream_chunks(deepseek_stream_chunks)]
#[case::stream_selector_variants(stream_selector_variants)]
#[case::openai_compatible_invoke_stream(openai_compatible_invoke_stream)]
#[case::framing_iteration_and_eof(framing_iteration_and_eof)]
fn invoke_events_cases(#[case] case: fn()) {
    case();
}
