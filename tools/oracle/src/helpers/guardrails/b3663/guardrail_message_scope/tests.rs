#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::helpers::{HelperRun, run_case};

#[rstest::rstest]
#[case::anthropic_sse_multiline_blocks("anthropic-sse-multiline-blocks")]
#[case::anthropic_sse_parse_data_prefix("anthropic-sse-parse-data-prefix")]
#[case::anthropic_sse_skip_invalid_json("anthropic-sse-skip-invalid-json")]
#[case::assistant_text_from_anthropic_response("assistant-text-from-anthropic-response")]
#[case::assistant_text_from_chat_response("assistant-text-from-chat-response")]
#[case::assistant_text_from_responses_output("assistant-text-from-responses-output")]
#[case::blocked_usage_chat_stream_final_usage("blocked-usage-chat-stream-final-usage")]
#[case::blocked_usage_chat_stream_no_final_usage("blocked-usage-chat-stream-no-final-usage")]
#[case::blocked_usage_fallback_to_zero("blocked-usage-fallback-to-zero")]
#[case::blocked_usage_response_prompt_completion_fallback(
    "blocked-usage-response-prompt-completion-fallback"
)]
#[case::blocked_usage_responses_api_total_fallback("blocked-usage-responses-api-total-fallback")]
#[case::blocked_usage_responses_stream_completed_event(
    "blocked-usage-responses-stream-completed-event"
)]
#[case::content_to_text_part_list_join("content-to-text-part-list-join")]
#[case::fastapi_exception_detail_dict_enrichment("fastapi-exception-detail-dict-enrichment")]
#[case::is_all_text_parts_empty_list("is-all-text-parts-empty-list")]
#[case::is_all_text_parts_text_only("is-all-text-parts-text-only")]
#[case::is_http_exception_fastapi_absent("is-http-exception-fastapi-absent")]
#[case::is_http_exception_installed_blocked_status("is-http-exception-installed-blocked-status")]
#[case::merge_messages_append_extras_after_last_scoped(
    "merge-messages-append-extras-after-last-scoped"
)]
#[case::merge_text_parts_cache_control_last("merge-text-parts-cache-control-last")]
#[case::merge_tools_anthropic_first_name("merge-tools-anthropic-first-name")]
#[case::merge_tools_preserve_request_duplicates("merge-tools-preserve-request-duplicates")]
#[case::message_slot_mismatch_returns_none("message-slot-mismatch-returns-none")]
#[case::message_slot_string_content("message-slot-string-content")]
#[case::message_slot_text_part_join("message-slot-text-part-join")]
#[case::metadata_guardrails_callback_selected("metadata-guardrails-callback-selected")]
#[case::metadata_guardrails_per_name_flags("metadata-guardrails-per-name-flags")]
#[case::metadata_guardrails_unknown_name("metadata-guardrails-unknown-name")]
#[case::metadata_team_modify_guardrails_lock("metadata-team-modify-guardrails-lock")]
#[case::metadata_team_without_lock("metadata-team-without-lock")]
#[case::metadata_without_guardrails_key("metadata-without-guardrails-key")]
#[case::pre_call_rejection_400_status("pre-call-rejection-400-status")]
#[case::pre_call_rejection_fastapi_installed("pre-call-rejection-fastapi-installed")]
#[case::scoped_indices_global_flag_overridden("scoped-indices-global-flag-overridden")]
#[case::scoped_indices_scan_only_tool_results("scoped-indices-scan-only-tool-results")]
#[case::scoped_indices_skip_system_and_tool("scoped-indices-skip-system-and-tool")]
#[case::tool_name_anthropic_dual_names("tool-name-anthropic-dual-names")]
#[case::tool_name_openai_flat_and_function("tool-name-openai-flat-and-function")]
#[case::unappliable_request_rewrite_exception("unappliable-request-rewrite-exception")]
#[case::unappliable_request_rewrite_unknown_name("unappliable-request-rewrite-unknown-name")]
fn cases_match_litellm(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/guardrail-message-scope")
        .join(format!("{name}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let result = run_case(&case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}

#[test]
fn replays_the_guardrail_message_scope_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("guardrail-message-scope-"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            crate::case::helper::validate_helper(&path, &case).unwrap();
        })
        .count();
    assert!(replayed >= 1);
}
