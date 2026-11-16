#![cfg(test)]

use rstest::rstest;

use crate::api::messages_chat_output::response::response_message;
use crate::api::messages_chat_output::usage::project_usage;
use crate::api::messages_context_clear::result::PolyfillResult;

use super::support::{RESPONSE_ID, assert_rendered, polyfill_result, response, usage};

#[rstest]
#[case(
    r#"{"prompt_tokens": 100, "completion_tokens": 7, "cache_read_input_tokens": 20,
           "cache_creation_input_tokens": 30}"#,
    r#"{"input_tokens": 50, "output_tokens": 7, "cache_creation_input_tokens": 30,
           "cache_read_input_tokens": 20}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_read_input_tokens": 20, "_cache_read_input_tokens": 9}"#,
    r#"{"input_tokens": 80, "output_tokens": 0, "cache_read_input_tokens": 20}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "_cache_read_input_tokens": 9}"#,
    r#"{"input_tokens": 91, "output_tokens": 0, "cache_read_input_tokens": 9}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "_cache_read_input_tokens": true,
           "prompt_tokens_details": {"cached_tokens": 20}}"#,
    r#"{"input_tokens": 80, "output_tokens": 0, "cache_read_input_tokens": 20}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_read_input_tokens": "3",
           "prompt_tokens_details": {"cached_tokens": 20}}"#,
    r#"{"input_tokens": 80, "output_tokens": 0, "cache_read_input_tokens": 20}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_read_input_tokens": 1.5,
           "prompt_tokens_details": {"cached_tokens": 20}}"#,
    r#"{"input_tokens": 80, "output_tokens": 0, "cache_read_input_tokens": 20}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_read_input_tokens": 3.0}"#,
    r#"{"input_tokens": 97, "output_tokens": 0, "cache_read_input_tokens": 3}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_read_input_tokens": 0, "_cache_read_input_tokens": 9}"#,
    r#"{"input_tokens": 91, "output_tokens": 0, "cache_read_input_tokens": 9}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_read_input_tokens": 0, "_cache_read_input_tokens": -2,
           "prompt_tokens_details": {"cached_tokens": 0}}"#,
    r#"{"input_tokens": 100, "output_tokens": 0}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "cache_creation_input_tokens": 30,
           "_cache_creation_input_tokens": 40}"#,
    r#"{"input_tokens": 70, "output_tokens": 0, "cache_creation_input_tokens": 30}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "_cache_creation_input_tokens": 40}"#,
    r#"{"input_tokens": 60, "output_tokens": 0, "cache_creation_input_tokens": 40}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "prompt_tokens_details": {"cache_creation_tokens": 30,
           "cache_write_tokens": 40}}"#,
    r#"{"input_tokens": 70, "output_tokens": 0, "cache_creation_input_tokens": 30}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "prompt_tokens_details": {"cache_write_tokens": 40}}"#,
    r#"{"input_tokens": 60, "output_tokens": 0, "cache_creation_input_tokens": 40}"#
)]
#[case(
    r#"{"prompt_tokens": 100, "prompt_tokens_details": {"cache_creation_tokens": 0,
           "cache_write_tokens": 40}}"#,
    r#"{"input_tokens": 60, "output_tokens": 0, "cache_creation_input_tokens": 40}"#
)]
#[case(
    r#"{"prompt_tokens": 10, "cache_read_input_tokens": 20, "cache_creation_input_tokens": 30}"#,
    r#"{"input_tokens": 0, "output_tokens": 0, "cache_creation_input_tokens": 30,
           "cache_read_input_tokens": 20}"#
)]
#[case(
    r#"{"prompt_tokens": null, "completion_tokens": null}"#,
    r#"{"input_tokens": 0, "output_tokens": 0}"#
)]
fn usage_projects_cache_counters(#[case] source: &str, #[case] expected: &str) {
    assert_rendered(&project_usage(&usage(source)), expected);
}

#[rstest]
#[case(
    r#"{"server_tool_use": {"web_search_requests": 2},
           "prompt_tokens_details": {"web_search_requests": 9}}"#,
    Some(2)
)]
#[case(
    r#"{"server_tool_use": {"web_search_requests": 0},
           "prompt_tokens_details": {"web_search_requests": 9}}"#,
    Some(9)
)]
#[case(
    r#"{"server_tool_use": {"web_search_requests": -1},
           "prompt_tokens_details": {"web_search_requests": 9}}"#,
    Some(9)
)]
#[case(
    r#"{"server_tool_use": {"web_search_requests": null},
           "prompt_tokens_details": {"web_search_requests": 9}}"#,
    Some(9)
)]
#[case(r#"{"prompt_tokens_details": {"web_search_requests": 9}}"#, Some(9))]
#[case(r#"{"server_tool_use": {"web_search_requests": 0}}"#, None)]
#[case(r#"{"server_tool_use": {}}"#, None)]
#[case("{}", None)]
fn usage_projects_web_search_counts(#[case] source: &str, #[case] expected: Option<i64>) {
    let projected = project_usage(&usage(source));
    assert_eq!(
        projected
            .server_tool_use
            .map(|server| server.web_search_requests),
        expected
    );
}

#[test]
fn iterations_append_a_final_message_record() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{"content": "hi"}}}}],
            "usage": {{"prompt_tokens": 100, "completion_tokens": 7, "cache_read_input_tokens": 20,
            "cache_creation_input_tokens": 30}}}}"#
    ));
    let translated = response_message(&record, None, Some(&polyfill_result())).expect("translates");
    assert_rendered(
        &translated.usage,
        r#"{"input_tokens": 50, "output_tokens": 7, "cache_creation_input_tokens": 30,
            "cache_read_input_tokens": 20,
            "iterations": [{"type": "compaction", "input_tokens": 10, "output_tokens": 1},
                           {"type": "message", "input_tokens": 50, "output_tokens": 7}]}"#,
    );
    assert_rendered(
        &translated.context_management,
        r#"{"applied_edits": [{"type": "clear_tool_uses_20250919", "cleared_input_tokens": 5}]}"#,
    );
}

#[test]
fn empty_iterations_still_append_the_final_record() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{}}}}], "usage": {{"prompt_tokens": 50, "completion_tokens": 7}}}}"#
    ));
    let polyfill = PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: Some(Vec::new()),
    };
    let translated = response_message(&record, None, Some(&polyfill)).expect("translates");
    assert_rendered(
        &translated.usage,
        r#"{"input_tokens": 50, "output_tokens": 7,
            "iterations": [{"type": "message", "input_tokens": 50, "output_tokens": 7}]}"#,
    );
    assert_rendered(&translated.context_management, "null");
}

#[test]
fn null_iterations_and_empty_edits_are_omitted() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{}}}}], "usage": {{"prompt_tokens": 50}}}}"#
    ));
    let polyfill = PolyfillResult::default();
    let translated = response_message(&record, None, Some(&polyfill)).expect("translates");
    assert_rendered(
        &translated.usage,
        r#"{"input_tokens": 50, "output_tokens": 0}"#,
    );
    assert_rendered(&translated.context_management, "null");
}

#[test]
fn negative_completion_omits_the_iteration_output() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{}}}}],
            "usage": {{"prompt_tokens": 5, "completion_tokens": -5}}}}"#
    ));
    let polyfill = PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: Some(Vec::new()),
    };
    let translated = response_message(&record, None, Some(&polyfill)).expect("translates");
    assert_rendered(
        &translated.usage,
        r#"{"input_tokens": 5, "output_tokens": -5,
            "iterations": [{"type": "message", "input_tokens": 5}]}"#,
    );
}

#[test]
fn missing_usage_projects_zeros() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{}}}}]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_rendered(
        &translated.usage,
        r#"{"input_tokens": 0, "output_tokens": 0}"#,
    );
}
