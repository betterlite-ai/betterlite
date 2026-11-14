use rstest::rstest;

use super::support::{assert_json, data, headers};

#[rstest]
#[case::no_metadata_emits_no_headers("{}", "[]")]
#[case::litellm_metadata_overlays_metadata_by_key(r#"{"metadata": {"applied_guardrails": ["a"]}, "litellm_metadata": {"applied_guardrails": ["b"]}}"#,r#"[["x-litellm-applied-guardrails", "b"]]"#)]
#[case::empty_applied_lists_emit_empty_headers(
    r#"{"metadata": {"applied_guardrails": [], "applied_policies": []}}"#,
    r#"[["x-litellm-applied-guardrails", ""], ["x-litellm-applied-policies", ""]]"#
)]
#[case::applied_names_join_with_commas(r#"{"metadata": {"applied_guardrails": ["pii", "moderation"], "applied_policies": ["a", "b"]}}"#,r#"[["x-litellm-applied-guardrails", "pii,moderation"], ["x-litellm-applied-policies", "a,b"]]"#)]
#[case::a_null_list_emits_no_header(r#"{"metadata": {"applied_guardrails": null}}"#, "[]")]
#[case::scan_ids_join_in_order(
    r#"{"metadata": {"guardrail_scan_ids": ["scan-2", "scan-1"]}}"#,
    r#"[["x-litellm-guardrail-scan-id", "scan-2,scan-1"]]"#
)]
#[case::an_empty_scan_id_list_emits_no_header(r#"{"metadata": {"guardrail_scan_ids": []}}"#, "[]")]
#[case::a_non_object_bucket_is_ignored(
    r#"{"metadata": "text", "litellm_metadata": {"applied_guardrails": ["kept"]}}"#,
    r#"[["x-litellm-applied-guardrails", "kept"]]"#
)]
#[case::policy_sources_join_with_semicolons(
    r#"{"metadata": {"policy_sources": {"p1": "reason, with comma", "p2": "other"}}}"#,
    r#"[["x-litellm-policy-sources", "p1=reason, with comma; p2=other"]]"#
)]
#[case::an_empty_policy_sources_map_emits_no_header(
    r#"{"metadata": {"policy_sources": {}}}"#,
    "[]"
)]
// docs/deviations/B8994.yaml: the bucket is a BTreeMap, so LiteLLM's insertion order is key order here
#[case::policy_sources_come_out_in_key_order(
    r#"{"metadata": {"policy_sources": {"zeta-policy": "second", "alpha-policy": "first"}}}"#,
    r#"[["x-litellm-policy-sources", "alpha-policy=first; zeta-policy=second"]]"#
)]
#[case::semantic_similarity_renders_as_a_number(
    r#"{"metadata": {"semantic-similarity": 0.85}}"#,
    r#"[["x-litellm-semantic-similarity", "0.85"]]"#
)]
#[case::a_whole_number_similarity_keeps_its_decimal(
    r#"{"metadata": {"semantic-similarity": 0.0}}"#,
    r#"[["x-litellm-semantic-similarity", "0.0"]]"#
)]
#[case::an_integer_similarity_renders_without_a_decimal(
    r#"{"metadata": {"semantic-similarity": 1}}"#,
    r#"[["x-litellm-semantic-similarity", "1"]]"#
)]
#[case::a_null_similarity_emits_no_header(r#"{"metadata": {"semantic-similarity": null}}"#, "[]")]
#[case::headers_keep_litellms_order(r#"{"metadata": {"applied_guardrails": ["a"], "guardrail_scan_ids": ["s"], "applied_policies": ["p"], "policy_sources": {"p": "r"}, "semantic-similarity": 0.5}}"#,r#"[["x-litellm-applied-guardrails", "a"], ["x-litellm-guardrail-scan-id", "s"], ["x-litellm-applied-policies", "p"], ["x-litellm-policy-sources", "p=r"], ["x-litellm-semantic-similarity", "0.5"]]"#)]
fn guardrail_metadata_cases(#[case] request: &str, #[case] expected: &str) {
    assert_json(&headers(&data(request)), expected);
}
