#![cfg(test)]

mod cases;
mod fixture;

use rstest::rstest;

#[rstest]
#[case::message_start_role(cases::message_start_role)]
#[case::text_delta(cases::text_delta)]
#[case::tool_use_and_argument_delta(cases::tool_use_and_argument_delta)]
#[case::thinking_and_signature_events(cases::thinking_and_signature_events)]
#[case::citation_event(cases::citation_event)]
#[case::json_mode_chunks(cases::json_mode_chunks)]
#[case::ping_no_content(cases::ping_no_content)]
#[case::error_event(cases::error_event)]
#[case::content_start_reset(cases::content_start_reset)]
#[case::web_search_stream_lifecycle(cases::web_search_stream_lifecycle)]
#[case::compaction_and_container_events(cases::compaction_and_container_events)]
#[case::accumulated_json_values(cases::accumulated_json_values)]
fn stream_events_cases(#[case] scenario: fn()) {
    scenario();
}
