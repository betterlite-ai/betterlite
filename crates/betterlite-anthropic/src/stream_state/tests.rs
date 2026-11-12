#![cfg(test)]

mod scenarios;

use rstest::rstest;

#[rstest]
#[case::empty_tool_arguments(scenarios::empty_tool_arguments)]
#[case::partial_input_json(scenarios::partial_input_json)]
#[case::thinking_signature(scenarios::thinking_signature)]
#[case::redacted_thinking(scenarios::redacted_thinking)]
#[case::usage_start_and_delta(scenarios::usage_start_and_delta)]
#[case::web_search_snapshot(scenarios::web_search_snapshot)]
#[case::code_output_buffer(scenarios::code_output_buffer)]
#[case::stop_reason_state(scenarios::stop_reason_state)]
#[case::initial_state(scenarios::initial_state)]
fn stream_state_cases(#[case] scenario: fn()) {
    scenario();
}
