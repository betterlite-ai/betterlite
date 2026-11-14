use rstest::rstest;

use super::support::{assert_json, data, headers};

#[rstest]
#[case::a_trusted_map_keeps_its_spelling_and_prefix(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": {"X-Pillar-Foo": "bar", "other": "dropped"}}}"#,r#"[["X-Pillar-Foo", "bar"]]"#)]
#[case::an_empty_trusted_map_suppresses_the_flag_fallback(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": {}, "pillar_flagged": true}}"#,"[]")]
#[case::a_trusted_map_omits_non_pillar_keys(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": {"x-pillar-a": "1", "x-other": "2"}}}"#,r#"[["x-pillar-a", "1"]]"#)]
#[case::a_trusted_null_map_falls_back_to_the_flag(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": null, "pillar_flagged": true}}"#,r#"[["x-pillar-flagged", "true"]]"#)]
#[case::a_trusted_flag_supplies_the_lowercase_header(
    r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_flagged": true}}"#,
    r#"[["x-pillar-flagged", "true"]]"#
)]
#[case::a_false_flag_is_lowercased(
    r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_flagged": false}}"#,
    r#"[["x-pillar-flagged", "false"]]"#
)]
#[case::an_untrusted_text_marker_is_ignored(r#"{"metadata": {"_pillar_response_headers_trusted": "true", "pillar_response_headers": {"x-pillar-a": "1"}, "pillar_flagged": true}}"#,"[]")]
#[case::an_untrusted_number_marker_is_ignored(
    r#"{"metadata": {"_pillar_response_headers_trusted": 1, "pillar_flagged": true}}"#,
    "[]"
)]
#[case::a_false_marker_is_ignored(
    r#"{"metadata": {"_pillar_response_headers_trusted": false, "pillar_flagged": true}}"#,
    "[]"
)]
#[case::a_null_map_value_is_skipped(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": {"x-pillar-a": null, "x-pillar-b": "kept"}}}"#,r#"[["x-pillar-b", "kept"]]"#)]
#[case::the_overlay_can_withdraw_trust(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": {"x-pillar-a": "1"}}, "litellm_metadata": {"_pillar_response_headers_trusted": false}}"#,"[]")]
fn guardrail_metadata_cases(#[case] request: &str, #[case] expected: &str) {
    assert_json(&headers(&data(request)), expected);
}
