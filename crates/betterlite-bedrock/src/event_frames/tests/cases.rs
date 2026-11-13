#![cfg(test)]

use rstest::rstest;

use super::{headers, parse, stream};

#[rstest]
#[case::split_frame(stream::split_frame)]
#[case::crc_mismatch(stream::crc_mismatch)]
#[case::header_type_values(stream::header_type_values)]
#[case::exception_frame(parse::exception_frame)]
#[case::unknown_frame(parse::unknown_frame)]
#[case::truncated_frame(stream::truncated_frame)]
#[case::json_payload_error(parse::json_payload_error)]
#[case::error_before_first_event(stream::error_before_first_event)]
#[case::shape_load_failure(parse::shape_load_failure)]
#[case::leftover_frame_after_events(stream::leftover_frame_after_events)]
#[case::anthropic_beta_header_forms(headers::anthropic_beta_header_forms)]
#[case::sse_record_and_json(headers::sse_record_and_json)]
fn event_frames_cases(#[case] check: fn()) {
    check();
}
