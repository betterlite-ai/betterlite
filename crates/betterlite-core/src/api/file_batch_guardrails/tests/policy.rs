#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{FakeFailure, Raised, named, object};
use crate::api::file_batch_guardrails::custom_id::{custom_id_of, describe};
use crate::api::file_batch_guardrails::fingerprint::{build_scan_metadata, fingerprint};
use crate::api::file_batch_guardrails::hook::{is_content_block, naming_guardrail};
use crate::api::file_batch_guardrails::record_json::RecordJson;
use crate::api::file_batch_guardrails::record_json::dumps::{Rendering, dumps};
use crate::api::file_batch_guardrails::record_json::text::RecordText;

#[rstest]
#[case::text(r#"{"custom_id":"abc"}"#, Some("abc"))]
#[case::integer(r#"{"custom_id":7}"#, Some("7"))]
#[case::negative_zero(r#"{"custom_id":-0}"#, Some("0"))]
#[case::float(r#"{"custom_id":1.5}"#, Some("1.5"))]
#[case::float_exponent(r#"{"custom_id":1e2}"#, Some("100.0"))]
#[case::large_float(r#"{"custom_id":1e16}"#, Some("1e+16"))]
#[case::overflowing_float(r#"{"custom_id":1e400}"#, Some("inf"))]
#[case::negative_zero_float(r#"{"custom_id":-0.0}"#, Some("-0.0"))]
#[case::big_integer(
    r#"{"custom_id":123456789012345678901234567890}"#,
    Some("123456789012345678901234567890")
)]
#[case::nan(r#"{"custom_id":NaN}"#, Some("nan"))]
#[case::infinity(r#"{"custom_id":Infinity}"#, Some("inf"))]
#[case::negative_infinity(r#"{"custom_id":-Infinity}"#, Some("-inf"))]
#[case::flag(r#"{"custom_id":true}"#, None)]
#[case::null(r#"{"custom_id":null}"#, None)]
#[case::object(r#"{"custom_id":{"x":1}}"#, None)]
#[case::list(r#"{"custom_id":["a"]}"#, None)]
#[case::absent("{}", None)]
#[case::lone_surrogate(r#"{"custom_id":"\ud800"}"#, Some("?"))]
#[case::surrogate_pair("{\"custom_id\":\"\\ud83d\\ude00\"}", Some("\u{1f600}"))]
fn renders_the_custom_id_as_text(#[case] payload: &str, #[case] expected: Option<&str>) {
    assert_eq!(custom_id_of(&object(payload)).as_deref(), expected);
}

#[rstest]
#[case::missing(None, "")]
#[case::empty(Some(""), "")]
#[case::plain(Some("abc"), " (custom_id abc)")]
#[case::controls(Some("a\nb\u{0}c\u{7f}"), " (custom_id a b c )")]
#[case::unit_separator(Some("a\u{1f}b"), " (custom_id a b)")]
#[case::space_and_beyond_ascii(Some("a b\u{80}"), " (custom_id a b\u{80})")]
fn describes_a_custom_id_for_logs(#[case] custom_id: Option<&str>, #[case] expected: &str) {
    assert_eq!(describe(custom_id), expected);
}

#[rstest]
#[case::at_the_cap(128, 128)]
#[case::one_past(129, 128)]
#[case::short(5, 5)]
fn caps_the_logged_custom_id(#[case] length: usize, #[case] shown: usize) {
    let custom_id = "x".repeat(length);
    assert_eq!(
        describe(Some(&custom_id)),
        format!(" (custom_id {})", "x".repeat(shown))
    );
}

fn same_fingerprint(left: &str, right: &str, keys: &[&str]) -> bool {
    let names: Vec<RecordText> = keys.iter().map(|key| RecordText::from(*key)).collect();
    let compared: BTreeSet<&RecordText> = names.iter().collect();
    fingerprint(&object(left), &compared) == fingerprint(&object(right), &compared)
}

#[rstest]
#[case::reordered(r#"{"x":1,"y":{"a":1,"b":2}}"#, r#"{"y":{"b":2,"a":1},"x":1}"#, &["x", "y"], true)]
#[case::absent_against_null(r#"{"y":1}"#, r#"{"x":null,"y":1}"#, &["x", "y"], false)]
#[case::null_added(r#"{"x":1}"#, r#"{"x":1,"z":null}"#, &["x", "z"], false)]
#[case::value_changed(r#"{"x":1}"#, r#"{"x":2}"#, &["x"], false)]
#[case::list_order_matters(r#"{"x":[1,2]}"#, r#"{"x":[2,1]}"#, &["x"], false)]
#[case::outside_the_keys(r#"{"x":1,"w":1}"#, r#"{"x":1,"w":2}"#, &["x"], true)]
#[case::injected_keys_still_count(r#"{"metadata":{}}"#, r#"{"metadata":{"a":1}}"#, &["metadata"], false)]
#[case::text_against_number(r#"{"x":"1"}"#, r#"{"x":1}"#, &["x"], false)]
#[case::same_float_written_twice(r#"{"x":1e2}"#, r#"{"x":100.0}"#, &["x"], true)]
#[case::trailing_zeros(r#"{"x":[1.50]}"#, r#"{"x":[1.5]}"#, &["x"], true)]
#[case::negative_zero_integer(r#"{"x":{"y":-0}}"#, r#"{"x":{"y":0}}"#, &["x"], true)]
#[case::integer_against_float(r#"{"x":1}"#, r#"{"x":1.0}"#, &["x"], false)]
fn fingerprints_ignore_only_key_order(
    #[case] left: &str,
    #[case] right: &str,
    #[case] keys: &[&str],
    #[case] same: bool,
) {
    assert_eq!(same_fingerprint(left, right, keys), same);
}

#[test]
fn scan_metadata_keeps_only_what_dispatch_reads() {
    let request: IndexMap<String, Json> = serde_json::from_str(
        r#"{"user":"u","guardrails":["g"],"_guardrail_pipelines":[1],"_pipeline_managed_guardrails":["p"],"user_api_key_metadata":{"a":1},"user_api_key_team_metadata":{"b":2},"tags":["t"],"headers":{"x-h":"v"},"litellm_parent_otel_span":"span","metadata":{"m":1}}"#,
    )
    .unwrap();
    let narrowed = RecordJson::Object(build_scan_metadata(&request));
    assert_eq!(
        dumps(&narrowed, Rendering::AsRead),
        r#"{"guardrails":["g"],"_guardrail_pipelines":[1],"_pipeline_managed_guardrails":["p"],"user_api_key_metadata":{"a":1},"user_api_key_team_metadata":{"b":2},"tags":["t"],"headers":{"x-h":"v"}}"#
    );
    assert!(build_scan_metadata(&IndexMap::new()).is_empty());
}

#[rstest]
#[case::raised_blocked(Raised::Verdict { blocked: true, caused: false }, true)]
#[case::raised_blocked_with_cause(Raised::Verdict { blocked: true, caused: true }, true)]
#[case::raised_unjudged(Raised::Verdict { blocked: false, caused: false }, false)]
#[case::intervention(Raised::Status { intervention: true, caused: false }, true)]
#[case::intervention_with_cause(Raised::Status { intervention: true, caused: true }, false)]
#[case::technical(Raised::Status { intervention: false, caused: false }, false)]
#[case::technical_with_cause(Raised::Status { intervention: false, caused: true }, false)]
fn a_block_is_a_judged_record(#[case] raised: Raised, #[case] blocked: bool) {
    assert_eq!(is_content_block(&named(raised)), blocked);
}

#[rstest]
#[case::attribute(Some("g"), Some("d"), Some("g"))]
#[case::empty_attribute(Some(""), Some("d"), Some(""))]
#[case::detail(None, Some("d"), Some("d"))]
#[case::nothing(None, None, None)]
fn names_the_guardrail_that_raised(
    #[case] name: Option<&str>,
    #[case] detail_name: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let failure = FakeFailure {
        raised: Raised::Status {
            intervention: true,
            caused: false,
        },
        name: name.map(str::to_owned),
        detail_name: detail_name.map(str::to_owned),
    };
    assert_eq!(naming_guardrail(&failure).as_deref(), expected);
}
