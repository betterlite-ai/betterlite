#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[rstest::rstest]
#[case::audit_missing_user("audit-missing-user")]
#[case::audit_prompt_and_response("audit-prompt-and-response")]
#[case::audit_prompt_error("audit-prompt-error")]
#[case::audit_text_completion_prompt("audit-text-completion-prompt")]
#[case::chat_pre_call_block("chat-pre-call-block")]
#[case::chat_pre_call_pass("chat-pre-call-pass")]
#[case::completion_text_block("completion-text-block")]
#[case::completion_token_id_reject("completion-token-id-reject")]
#[case::logging_hook_async_context("logging-hook-async-context")]
#[case::logging_hook_sync_context("logging-hook-sync-context")]
#[case::post_success_response_block("post-success-response-block")]
#[case::post_success_response_pass("post-success-response-pass")]
#[case::responses_api_stream_block("responses-api-stream-block")]
#[case::responses_api_stream_pass("responses-api-stream-pass")]
#[case::responses_pre_call_block("responses-pre-call-block")]
#[case::responses_pre_call_transform_error("responses-pre-call-transform-error")]
#[case::responses_stream_incomplete("responses-stream-incomplete")]
#[case::streaming_assembly_failed("streaming-assembly-failed")]
#[case::streaming_buffered_block("streaming-buffered-block")]
#[case::streaming_buffered_pass("streaming-buffered-pass")]
fn cases_match_litellm(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR)
        .join("cases/guardrails/purview-dlp-hooks")
        .join(format!("{name}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
