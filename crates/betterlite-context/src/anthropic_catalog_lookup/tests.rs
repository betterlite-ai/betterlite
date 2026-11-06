#![cfg(test)]

use rstest::rstest;
use serde::Deserialize;

use super::candidates::CandidatePatterns;
use super::error::CatalogLookupError;
use super::{RawCatalog, exact_capability, model_capability, supports_fast_mode};

enum Call {
    Candidates,
    Capability,
    Exact,
    FastMode(&'static str),
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
enum Outcome {
    Candidates(Vec<String>),
    Flag(Option<bool>),
    Failure(String),
}

fn catalog(source: &str) -> RawCatalog {
    serde_json::from_str(source).unwrap()
}

fn failure(error: &CatalogLookupError) -> Outcome {
    Outcome::Failure(error.to_string())
}

fn assert_lookup(
    call: &Call,
    model: &str,
    runtime_json: &str,
    packaged_json: &str,
    expected: &str,
) {
    let patterns = CandidatePatterns::new().unwrap();
    let runtime = catalog(runtime_json);
    let actual = match call {
        Call::Candidates => Outcome::Candidates(patterns.candidates(model)),
        Call::Capability => Outcome::Flag(model_capability(
            &patterns,
            &runtime,
            &catalog(packaged_json),
            model,
            "flag",
        )),
        Call::Exact => exact_capability(&runtime, model, "flag")
            .map_or_else(|error| failure(&error), Outcome::Flag),
        Call::FastMode(provider) => supports_fast_mode(&runtime, model, provider).map_or_else(
            |error| failure(&error),
            |enabled| Outcome::Flag(Some(enabled)),
        ),
    };
    assert_eq!(actual, serde_json::from_str::<Outcome>(expected).unwrap());
}

#[rstest]
#[case::candidate_order(
    Call::Candidates,
    "bedrock/invoke/us.anthropic.Claude-Sonnet-4.6-20260219-v1:0@x",
    "{}",
    "{}",
    r#"["bedrock/invoke/us.anthropic.Claude-Sonnet-4.6-20260219-v1:0@x",
    "us.anthropic.Claude-Sonnet-4.6-20260219-v1:0@x",
    "invoke/us.anthropic.Claude-Sonnet-4.6-20260219-v1:0@x",
    "anthropic.Claude-Sonnet-4.6-20260219-v1:0@x",
    "bedrock/anthropic.Claude-Sonnet-4.6-20260219-v1:0@x",
    "bedrock/invoke/us.anthropic.Claude-Sonnet-4-6-20260219-v1:0@x",
    "bedrock/invoke/us.anthropic.Claude-Sonnet-4.6-20260219-v1:0",
    "us.anthropic.Claude-Sonnet-4-6-20260219-v1:0@x",
    "us.anthropic.Claude-Sonnet-4.6-20260219-v1:0",
    "invoke/us.anthropic.Claude-Sonnet-4-6-20260219-v1:0@x",
    "invoke/us.anthropic.Claude-Sonnet-4.6-20260219-v1:0",
    "anthropic.Claude-Sonnet-4-6-20260219-v1:0@x",
    "anthropic.Claude-Sonnet-4.6-20260219-v1:0",
    "bedrock/anthropic.Claude-Sonnet-4-6-20260219-v1:0@x",
    "bedrock/anthropic.Claude-Sonnet-4.6-20260219-v1:0"]"#
)]
#[case::catalog_precedence(
    Call::Capability,
    "claude-sonnet-4.6-20260219",
    r#"{"claude-sonnet-4-6": {"flag": false}}"#,
    r#"{"claude-sonnet-4.6-20260219": {"flag": true}}"#,
    "false"
)]
#[case::exact_capability(
    Call::Exact,
    "claude-sonnet-4-6",
    r#"{"claude-sonnet-4-6": null}"#,
    "{}",
    r#""catalog entry claude-sonnet-4-6 is not an object""#
)]
#[case::fast_host_only(
    Call::FastMode("anthropic"),
    "claude-opus-4-6",
    r#"{"claude-opus-4-6": {"supports_fast_mode": true}}"#,
    "{}",
    "true"
)]
#[case::bare_prefix_raises(Call::Candidates, "invoke/global", "{}", "{}", r#"["invoke/global"]"#)]
#[case::bare_prefix_after_bedrock(
    Call::Candidates,
    "bedrock/global",
    "{}",
    "{}",
    r#"["bedrock/global", "global"]"#
)]
#[case::leading_at_kept(
    Call::Candidates,
    "@x",
    "{}",
    "{}",
    r#"["@x", "bedrock/@x", "bedrock/"]"#
)]
#[case::empty_model(Call::Candidates, "", "{}", "{}", r#"[""]"#)]
#[case::trailing_newline(
    Call::Candidates,
    "claude-x-20250101\n",
    "{}",
    "{}",
    r#"["claude-x-20250101\n", "bedrock/claude-x-20250101\n", "claude-x\n", "bedrock/claude-x\n"]"#
)]
#[case::version_then_minor(
    Call::Candidates,
    "us-gov.anthropic.claude-v2:1",
    "{}",
    "{}",
    r#"["us-gov.anthropic.claude-v2:1", "anthropic.claude-v2:1", "bedrock/anthropic.claude-v2:1",
    "us-gov.anthropic.claude", "us-gov.anthropic.claude-v2", "anthropic.claude",
    "anthropic.claude-v2", "bedrock/anthropic.claude", "bedrock/anthropic.claude-v2"]"#
)]
#[case::composite_dotted_version_date(
    Call::Candidates,
    "us.anthropic.claude-sonnet-4.6-20260301-v1:0",
    "{}",
    "{}",
    r#"["us.anthropic.claude-sonnet-4.6-20260301-v1:0", "anthropic.claude-sonnet-4.6-20260301-v1:0",
    "bedrock/anthropic.claude-sonnet-4.6-20260301-v1:0", "us.anthropic.claude-sonnet-4.6-20260301",
    "us.anthropic.claude-sonnet-4.6-20260301-v1", "us.anthropic.claude-sonnet-4-6-20260301-v1:0",
    "us.anthropic.claude-sonnet-4-6", "anthropic.claude-sonnet-4.6-20260301",
    "anthropic.claude-sonnet-4.6-20260301-v1", "anthropic.claude-sonnet-4-6-20260301-v1:0",
    "anthropic.claude-sonnet-4-6", "bedrock/anthropic.claude-sonnet-4.6-20260301",
    "bedrock/anthropic.claude-sonnet-4.6-20260301-v1", "bedrock/anthropic.claude-sonnet-4-6-20260301-v1:0",
    "bedrock/anthropic.claude-sonnet-4-6"]"#
)]
#[case::string_skipped_for_packaged(
    Call::Capability,
    "m",
    r#"{"m": {"flag": "yes"}}"#,
    r#"{"m": {"flag": true}}"#,
    "true"
)]
#[case::number_and_null_skipped(
    Call::Capability,
    "m",
    r#"{"m": {"flag": 1}}"#,
    r#"{"m": {"flag": null}}"#,
    "null"
)]
#[case::runtime_malformed_aborts(
    Call::Capability,
    "m",
    r#"{"m": null}"#,
    r#"{"m": {"flag": true}}"#,
    "null"
)]
#[case::packaged_malformed_aborts(Call::Capability, "m", "{}", r#"{"m": [1]}"#, "null")]
#[case::hit_before_malformed(
    Call::Capability,
    "m-v1:0",
    r#"{"m-v1:0": {"flag": true}, "m": null}"#,
    "{}",
    "true"
)]
#[case::malformed_before_hit(
    Call::Capability,
    "m-v1:0",
    r#"{"m-v1:0": null, "m": {"flag": true}}"#,
    "{}",
    "null"
)]
#[case::runtime_alias_beats_packaged_exact(
    Call::Capability,
    "m-v1:0",
    r#"{"m": {"flag": false}}"#,
    r#"{"m-v1:0": {"flag": true}}"#,
    "false"
)]
#[case::exact_skips_normalization(
    Call::Exact,
    "claude-x-20250101",
    r#"{"claude-x": {"flag": true}}"#,
    "{}",
    "null"
)]
#[case::exact_bool(Call::Exact, "m", r#"{"m": {"flag": true}}"#, "{}", "true")]
#[case::exact_string_ignored(Call::Exact, "m", r#"{"m": {"flag": "true"}}"#, "{}", "null")]
#[case::exact_list_entry_fails(
    Call::Exact,
    "m",
    "{\"m\": []}",
    "{}",
    r#""catalog entry m is not an object""#
)]
#[case::fast_bedrock(
    Call::FastMode("bedrock"),
    "m",
    r#"{"m": {"supports_fast_mode": true}}"#,
    "{}",
    "false"
)]
#[case::fast_provider_case(
    Call::FastMode("Anthropic"),
    "m",
    r#"{"m": {"supports_fast_mode": true}}"#,
    "{}",
    "false"
)]
#[case::fast_string_flag(
    Call::FastMode("anthropic"),
    "m",
    r#"{"m": {"supports_fast_mode": "true"}}"#,
    "{}",
    "false"
)]
#[case::fast_alias(
    Call::FastMode("anthropic"),
    "anthropic/m",
    r#"{"m": {"supports_fast_mode": true}}"#,
    "{}",
    "false"
)]
#[case::fast_packaged_only(
    Call::FastMode("anthropic"),
    "m",
    "{}",
    r#"{"m": {"supports_fast_mode": true}}"#,
    "false"
)]
#[case::fast_other_host_skips_entry(
    Call::FastMode("bedrock"),
    "m",
    r#"{"m": null}"#,
    "{}",
    "false"
)]
#[case::fast_malformed_entry_fails(
    Call::FastMode("anthropic"),
    "m",
    r#"{"m": null}"#,
    "{}",
    r#""catalog entry m is not an object""#
)]
fn anthropic_catalog_lookup_cases(
    #[case] call: Call,
    #[case] model: &str,
    #[case] runtime: &str,
    #[case] packaged: &str,
    #[case] expected: &str,
) {
    assert_lookup(&call, model, runtime, packaged, expected);
}
