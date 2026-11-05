#![cfg(test)]

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::path::Path;

#[rstest]
#[case::artifact_extraction("pydantic-artifact-extraction")]
#[case::history_fallback("pydantic-history-fallback")]
#[case::message_fallback("pydantic-message-fallback")]
#[case::multipart_join("pydantic-multipart-join")]
#[case::transform_response("pydantic-transform-response")]
#[case::fake_streaming("pydantic-fake-streaming")]
#[case::empty_text("pydantic-empty-text")]
#[case::empty_result_transform("pydantic-empty-result-transform")]
#[case::empty_result_extract("pydantic-empty-result-extract")]
#[case::history_generated_id("pydantic-history-generated-id")]
#[case::history_skips_blank_agent("pydantic-history-skips-blank-agent")]
#[case::message_without_id("pydantic-message-without-id")]
#[case::stream_unicode("pydantic-stream-unicode")]
#[case::stream_custom_pacing("pydantic-stream-custom-pacing")]
#[case::stream_no_user_message("pydantic-stream-no-user-message")]
#[case::stream_user_without_id("pydantic-stream-user-without-id")]
#[case::stream_user_without_parts("pydantic-stream-user-without-parts")]
#[case::stream_exact_chunks("pydantic-stream-exact-chunks")]
fn pydantic_ai_transformation(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("cases/a2a/{name}.yaml"));
    let cases = load_cases(&[path]).unwrap();
    let LoadedCase::Helper(case) = &cases[0] else {
        panic!("expected helper");
    };
    assert!(matches!(run_case(case, false).unwrap(), HelperRun::Match));
}
