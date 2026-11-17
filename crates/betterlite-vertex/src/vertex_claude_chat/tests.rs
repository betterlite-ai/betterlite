#![cfg(test)]

mod fixture;

use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use rstest::rstest;

use crate::vertex_claude_chat::provider::is_supported_model;

use fixture::{
    InliningFetcher, RefusingFetcher, Scenario, expect_absent, expect_field, json, map,
    map_with_packaged,
};

const NO_DROP: DropParams = DropParams {
    global: false,
    call: false,
};
const FORMAT: &str = r#"{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}"#;
const BILLING: &str = r#"[{"role":"system","content":"x-anthropic-billing-header: abc"},{"role":"system","content":"kept"},{"role":"user","content":"hi"}]"#;
const REMOTE_IMAGES: &str = r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}},{"type":"image_url","image_url":{"url":"http://example.test/b.png"}}]}]"#;
const INLINE_IMAGE: &str = r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:image/png;base64,QUJD"}]}]"#;
const INLINED_IMAGES: &str = r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}]"#;
const INLINED_IMAGE: &str = r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}]"#;
const FILE_MESSAGE: &str =
    r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-abc"}}]}]"#;

#[rstest]
#[case::haiku_keeps_unrelated_keys(
    "claude-haiku-4-5",
    r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
    Some(r#"{"format":{"type":"json_schema"}}"#)
)]
#[case::haiku_drops_an_empty_object(
    "claude-haiku-4-5",
    r#"{"output_config":{"effort":"high"}}"#,
    None
)]
#[case::opus_keeps_effort(
    "claude-opus-4-5",
    r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
    Some(r#"{"effort":"high","format":{"type":"json_schema"}}"#)
)]
#[case::explicit_null_survives("claude-haiku-4-5", r#"{"output_config":null}"#, Some("null"))]
#[case::non_object_is_removed("claude-haiku-4-5", r#"{"output_config":"high"}"#, None)]
fn overrides_output_config_effort_policy(
    #[case] model: &str,
    #[case] optional: &str,
    #[case] expected: Option<&str>,
) {
    let outcome = Scenario::new(model, fixture::SIMPLE)
        .optional(optional)
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(
        outcome.get("output_config").cloned(),
        expected.map(json),
        "{model}"
    );
    expect_absent(&outcome, "model");
}

#[test]
fn overrides_response_format_falls_back_to_tools() {
    let mapped = map(
        "claude-sonnet-4-5",
        &format!(r#"{{"response_format":{FORMAT}}}"#),
        "{}",
        NO_DROP,
    )
    .expect("the request maps");
    assert!(!mapped.contains_key("output_format"));
    assert!(mapped.contains_key("tools"));
    // claude-3-sonnet-20240229 rejects forced tool use, so the fallback model loses the choice
    assert!(!mapped.contains_key("tool_choice"));
    assert_eq!(mapped.get("json_mode"), Some(&json("true")));
}

#[test]
fn overrides_response_format_native_keeps_model() {
    let mapped = map(
        "claude-sonnet-4-6",
        &format!(r#"{{"response_format":{FORMAT}}}"#),
        "{}",
        NO_DROP,
    )
    .expect("the request maps");
    assert!(mapped.contains_key("output_format"));
    assert!(!mapped.contains_key("tools"));
    assert_eq!(mapped.get("json_mode"), Some(&json("true")));
}

#[test]
fn overrides_reads_native_output_format_from_the_packaged_catalog() {
    let mapped = map_with_packaged(
        "claude-sonnet-4-5",
        &format!(r#"{{"response_format":{FORMAT}}}"#),
        "{}",
        NO_DROP,
        r#"{"claude-sonnet-4-5":{"supports_native_structured_output":true}}"#,
    )
    .expect("the request maps");
    assert!(mapped.contains_key("output_format"));
    assert!(!mapped.contains_key("tools"));
}

#[test]
fn overrides_adaptive_thinking_uses_original_model() {
    let mapped = map(
        "claude-sonnet-4-5",
        &format!(
            r#"{{"response_format":{FORMAT},"thinking":{{"type":"enabled","budget_tokens":4096}}}}"#
        ),
        "{}",
        NO_DROP,
    )
    .expect("the request maps");
    assert_eq!(
        mapped.get("thinking"),
        Some(&json(r#"{"type":"adaptive"}"#))
    );
    assert_eq!(
        mapped.get("output_config"),
        Some(&json(r#"{"effort":"high"}"#))
    );
}

#[rstest]
#[case::effort_beta_survives_the_dropped_effort(
    "claude-haiku-4-5",
    r#"{"output_config":{"effort":"high"}}"#,
    "effort-2025-11-24"
)]
#[case::effort_beta_for_supported_model(
    "claude-opus-4-5",
    r#"{"output_config":{"effort":"high"}}"#,
    "effort-2025-11-24"
)]
#[case::vertex_tool_search_beta(
    "claude-haiku-4-5",
    r#"{"tools":[{"type":"tool_search_tool_regex_20251119","name":"find","input_schema":{"type":"object"}}]}"#,
    "tool-search-tool-2025-10-19"
)]
#[case::compact_edit_selects_compact(
    "claude-haiku-4-5",
    r#"{"context_management":{"edits":[{"type":"compact_20260112"}]}}"#,
    "compact-2026-01-12"
)]
#[case::any_other_edit_selects_context_management(
    "claude-haiku-4-5",
    r#"{"context_management":{"edits":[{"type":"compaction"}]}}"#,
    "context-management-2025-06-27"
)]
#[case::both_edit_kinds_keep_both_betas(
    "claude-haiku-4-5",
    r#"{"context_management":{"edits":[{"type":"compact_20260112"},{"type":"clear_tool_uses_20250919"}]}}"#,
    "compact-2026-01-12,context-management-2025-06-27"
)]
#[case::comma_string_splits_and_trims(
    "claude-haiku-4-5",
    r#"{"extra_headers":{"anthropic-beta":"foo, bar, ,baz"}}"#,
    "bar,baz,foo"
)]
#[case::list_merges_in_order(
    "claude-haiku-4-5",
    r#"{"extra_headers":{"anthropic-beta":["x","y"]}}"#,
    "x,y"
)]
#[case::union_with_automatic_betas(
    "claude-haiku-4-5",
    r#"{"tools":[{"type":"tool_search_tool_regex_20251119","name":"find","input_schema":{"type":"object"}}],"extra_headers":{"anthropic-beta":"zzz"}}"#,
    "tool-search-tool-2025-10-19,zzz"
)]
fn overrides_beta_membership(#[case] model: &str, #[case] optional: &str, #[case] expected: &str) {
    let outcome = Scenario::new(model, fixture::SIMPLE)
        .optional(optional)
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(outcome.beta(), Some(expected), "{optional}");
    let listed: Vec<&str> = expected.split(',').collect();
    let expected_body = serde_json::to_string(&listed).expect("the beta list encodes");
    expect_field(&outcome, "anthropic_beta", &expected_body);
    expect_absent(&outcome, "extra_headers");
}

#[test]
fn overrides_writes_no_beta_when_none_is_computed() {
    let outcome = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .header("anthropic-beta", "keep")
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(outcome.beta(), Some("keep"));
    expect_absent(&outcome, "anthropic_beta");
}

#[test]
fn overrides_removes_model_and_strips_billing() {
    let outcome = Scenario::new("claude-haiku-4-5", BILLING)
        .run_with(&RefusingFetcher::default())
        .ok();
    expect_absent(&outcome, "model");
    expect_field(&outcome, "system", r#"[{"type":"text","text":"kept"}]"#);
}

#[rstest]
#[case::vertex_ai("claude-3-5-sonnet-20241022", "vertex_ai", true)]
#[case::vertex_ai_beta_lowercases("CLAUDE-opus-4-5", "vertex_ai_beta", true)]
#[case::other_provider("claude-3-5-sonnet-20241022", "anthropic", false)]
#[case::catalog_member("vertex-legacy-model", "vertex_ai", true)]
#[case::unknown_model("gemini-2.5-pro", "vertex_ai", false)]
fn overrides_accepts_vertex_providers_and_claude_models(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] expected: bool,
) {
    let mut sets = ProviderModelSets::default();
    if let Some(models) = sets.sets.get_mut(&ProviderModelSet::VertexAnthropic) {
        models.insert("vertex-legacy-model".to_owned());
    }
    assert_eq!(
        is_supported_model(model, provider, &sets),
        expected,
        "{model}"
    );
}

#[test]
fn remote_image_fetch() {
    let inlining = InliningFetcher::default();
    let outcome = Scenario::new("claude-haiku-4-5", REMOTE_IMAGES)
        .run_with(&inlining)
        .ok();
    assert_eq!(
        inlining.urls(),
        ["https://example.test/a.png", "http://example.test/b.png"]
    );
    expect_field(&outcome, "messages", INLINED_IMAGES);

    let inline = InliningFetcher::default();
    let outcome = Scenario::new("claude-haiku-4-5", INLINE_IMAGE)
        .run_with(&inline)
        .ok();
    assert!(inline.urls().is_empty());
    expect_field(&outcome, "messages", INLINED_IMAGE);

    let refusing = RefusingFetcher::default();
    let (status, message) = Scenario::new("claude-haiku-4-5", REMOTE_IMAGES)
        .run_with(&refusing)
        .failed();
    assert_eq!(status, Some(400));
    assert_eq!(refusing.calls(), 1);
    assert!(message.contains("https://example.test/a.png"), "{message}");
}

#[rstest]
#[case::context_management_not_an_object(
    r#"{"context_management":"edits"}"#,
    "context_management must be an object"
)]
#[case::edits_not_a_list(
    r#"{"context_management":{"edits":"compact_20260112"}}"#,
    "context_management.edits must be a list"
)]
#[case::edit_not_an_object(
    r#"{"context_management":{"edits":[1]}}"#,
    "context_management edit 0 is not an object"
)]
#[case::extra_headers_not_an_object(
    r#"{"extra_headers":"anthropic-beta: foo"}"#,
    "extra_headers must be an object"
)]
#[case::beta_entry_not_text(
    r#"{"extra_headers":{"anthropic-beta":[1]}}"#,
    "extra_headers.anthropic-beta entries must be strings"
)]
fn overrides_rejects_malformed_beta_inputs(#[case] optional: &str, #[case] message: &str) {
    let (status, failure) = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .optional(optional)
        .run_with(&RefusingFetcher::default())
        .failed();
    assert_eq!(status, Some(400), "{optional}");
    assert!(failure.contains(message), "{failure}");
}

#[rstest]
#[case::other_key_only(r#"{"extra_headers":{"x-other":"1"}}"#)]
#[case::null_beta_value(r#"{"extra_headers":{"anthropic-beta":null}}"#)]
#[case::bool_beta_value(r#"{"extra_headers":{"anthropic-beta":true}}"#)]
#[case::number_beta_value(r#"{"extra_headers":{"anthropic-beta":1}}"#)]
#[case::object_beta_value(r#"{"extra_headers":{"anthropic-beta":{}}}"#)]
#[case::null_extra_headers(r#"{"extra_headers":null}"#)]
#[case::null_context_management(r#"{"context_management":null}"#)]
#[case::null_edits(r#"{"context_management":{"edits":null}}"#)]
#[case::no_edits(r#"{"context_management":{"edits":[]}}"#)]
fn overrides_ignores_beta_inputs_it_cannot_read(#[case] optional: &str) {
    let outcome = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .optional(optional)
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(outcome.beta(), None, "{optional}");
    expect_absent(&outcome, "anthropic_beta");
    expect_absent(&outcome, "extra_headers");
}

#[test]
fn overrides_skips_the_parent_beta_headers() {
    let outcome = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .optional(r#"{"output_format":{"type":"json_schema","schema":{"type":"object"}}}"#)
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(outcome.beta(), None);
    expect_absent(&outcome, "anthropic_beta");
    expect_field(
        &outcome,
        "output_format",
        r#"{"type":"json_schema","schema":{"type":"object"}}"#,
    );
}

#[test]
fn overrides_reports_the_vertex_provider_to_the_parent() {
    let (status, failure) = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .optional(r#"{"speed":"fast"}"#)
        .run_with(&RefusingFetcher::default())
        .failed();
    assert_eq!(status, Some(400));
    assert!(failure.contains("does not support speed"), "{failure}");
}

#[test]
fn overrides_adds_the_file_betas_for_a_file_id_message() {
    let outcome = Scenario::new("claude-haiku-4-5", FILE_MESSAGE)
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(
        outcome.beta(),
        Some("code-execution-2025-05-22,files-api-2025-04-14")
    );
}

#[test]
fn overrides_adds_the_mcp_beta_for_mcp_servers() {
    let outcome = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .optional(r#"{"mcp_servers":[{"type":"url","url":"https://mcp.test/sse","name":"tools"}]}"#)
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(outcome.beta(), Some("mcp-client-2025-04-04"));
}

#[test]
fn overrides_adds_the_computer_use_beta() {
    let outcome = Scenario::new("claude-haiku-4-5", fixture::SIMPLE)
        .optional(
            r#"{"tools":[{"type":"computer_20250124","name":"computer","display_width_px":1024,"display_height_px":768}]}"#,
        )
        .run_with(&RefusingFetcher::default())
        .ok();
    assert_eq!(outcome.beta(), Some("computer-use-2025-01-24"));
}

#[test]
fn overrides_restores_the_requested_model_in_the_response() {
    let envelope = fixture::transform_response_body("claude-sonnet-4-5", fixture::RESPONSE);
    assert_eq!(envelope.model.as_deref(), Some("claude-sonnet-4-5"));
}

#[test]
fn overrides_maps_without_the_fallback_model_when_response_format_is_absent() {
    let mapped = map(
        "claude-haiku-4-5",
        r#"{"tool_choice":"required","tools":[{"type":"function","function":{"name":"look","parameters":{"type":"object"}}}]}"#,
        "{}",
        NO_DROP,
    )
    .expect("the request maps");
    assert_eq!(mapped.get("tool_choice"), Some(&json(r#"{"type":"any"}"#)));
}
