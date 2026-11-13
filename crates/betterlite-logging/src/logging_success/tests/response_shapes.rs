use rstest::rstest;

use super::support::{parse, payload};
use crate::logging_success::request_redaction::redact_payload;

const OUTPUT: &str = r#"{"id": "r", "output": [
    {"type": "reasoning", "summary": [{"type": "summary_text", "text": "thought"}]},
    {"type": "function_call", "name": "f", "arguments": "{}"},
    {"type": "function_call", "name": "g"},
    {"type": "custom_tool_call", "name": "c", "input": "secret"},
    {"type": "custom_tool_call", "name": "d"},
    {"type": "message", "text": "t", "content": [{"type": "refusal", "refusal": "no", "text": "x"}]}
  ],
  "vertex_ai_grounding_metadata": [1], "vertex_ai_url_context_metadata": [1],
  "vertex_ai_safety_ratings": [1], "vertex_ai_safety_results": [1],
  "vertex_ai_citation_metadata": [1],
  "_hidden_params": {"vertex_ai_grounding_metadata": [1], "vertex_ai_url_context_metadata": [1],
    "vertex_ai_safety_ratings": [1], "vertex_ai_safety_results": [1],
    "vertex_ai_citation_metadata": [1], "keep": 1}}"#;
const OUTPUT_REDACTED: &str = r#"{"id": "r", "output": [
    {"type": "reasoning", "summary": [{"type": "summary_text", "text": "redacted-by-litellm"}]},
    {"type": "function_call", "name": "f", "arguments": "redacted-by-litellm"},
    {"type": "function_call", "name": "g"},
    {"type": "custom_tool_call", "name": "c", "input": "redacted-by-litellm"},
    {"type": "custom_tool_call", "name": "d"},
    {"type": "message", "text": "redacted-by-litellm", "content": [{"type": "refusal", "refusal": "redacted-by-litellm", "text": "redacted-by-litellm"}]}
  ],
  "vertex_ai_grounding_metadata": [], "vertex_ai_url_context_metadata": [],
  "vertex_ai_safety_ratings": [], "vertex_ai_safety_results": [],
  "vertex_ai_citation_metadata": [],
  "_hidden_params": {"keep": 1}}"#;
const CHOICES: &str = r#"{"id": "r", "choices": [
    {"message": {"content": "a", "reasoning_content": "b", "thinking_blocks": [{}], "audio": {},
      "function_call": {"name": "f", "arguments": "{}"}}},
    {"delta": {"content": "d"}},
    {"text": "t"},
    {"message": {"content": null, "function_call": {"name": "g"}}},
    7
  ],
  "vertex_ai_safety_ratings": [1],
  "_hidden_params": {"vertex_ai_safety_ratings": [1], "keep": 1}}"#;
const CHOICES_REDACTED: &str = r#"{"id": "r", "choices": [
    {"message": {"content": "redacted-by-litellm", "reasoning_content": "redacted-by-litellm",
      "thinking_blocks": null, "audio": null,
      "function_call": {"name": "f", "arguments": "redacted-by-litellm"}}},
    {"delta": {"content": "redacted-by-litellm"}},
    {"text": "redacted-by-litellm"},
    {"message": {"content": null, "function_call": {"name": "g"}}},
    7
  ],
  "vertex_ai_safety_ratings": [],
  "_hidden_params": {"keep": 1}}"#;
const TEXT_ONLY: &str = r#"{"text": "redacted-by-litellm"}"#;

#[rstest]
#[case::output_items(OUTPUT, OUTPUT_REDACTED)]
#[case::choices(CHOICES, CHOICES_REDACTED)]
#[case::text("\"secret\"", "\"redacted-by-litellm\"")]
#[case::list("[\"secret\"]", TEXT_ONLY)]
#[case::number("3", TEXT_ONLY)]
#[case::bool("true", TEXT_ONLY)]
#[case::other_object(r#"{"other": "secret"}"#, TEXT_ONLY)]
#[case::null("null", "null")]
#[case::output_not_a_list(r#"{"output": "secret"}"#, r#"{"output": "secret"}"#)]
#[test]
fn logging_success_behavior_tests_behavior(#[case] response: &str, #[case] expected: &str) {
    let mut redacted = payload();
    redacted.response = Some(parse(response));
    redact_payload(&mut redacted);
    assert_eq!(redacted.response, Some(parse(expected)));
}
