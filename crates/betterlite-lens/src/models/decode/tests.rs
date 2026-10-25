#![cfg(test)]

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::models::decode::{DecodeError, decode};
use crate::models::execution::{ExecutionContent, Sample};
use crate::models::finding::{Evidence, FindingDraft};
use crate::models::job::{Job, Lens, LensList, Worker};
use crate::models::messages::{
    Claim, FindingUpdate, JobResult, ModelRequest, ModelResult, Progress, RunRequest,
};
use crate::models::settings::{LensSettings, Source};
use crate::models::validate::{RecordError, Validate};

fn rule<T>(json: &str) -> RecordError
where
    T: DeserializeOwned + Validate + Debug,
{
    match decode::<T>(json.as_bytes()).unwrap_err() {
        DecodeError::Invalid(rule) => rule,
        DecodeError::Json(error) => panic!("expected a rule failure, got {error}"),
    }
}

#[test]
fn settings_fill_the_pinned_defaults() {
    let settings =
        decode::<LensSettings>(br#"{"name": "lens", "model": "gpt-4o", "context": "Cite."}"#)
            .unwrap();
    assert_eq!(
        (
            settings.source,
            settings.lookback_hours,
            settings.interval_minutes
        ),
        (Source::Traces, 24, 15)
    );
    assert_eq!(
        (settings.sample_size, settings.concurrency, settings.enabled),
        (None, 8, true)
    );
    assert_eq!(
        (settings.sample_percent, settings.monthly_budget),
        (Decimal::ONE_HUNDRED, Decimal::ONE_HUNDRED)
    );
    let written = serde_json::to_string(&settings).unwrap();
    assert!(written.contains(r#""monthly_budget":100.0"#));
}

#[test]
fn money_keeps_its_decimal_digits() {
    let settings = decode::<LensSettings>(
        br#"{"name": "l", "model": "m", "context": "c", "monthly_budget": 0.1}"#,
    );
    assert_eq!(settings.unwrap().monthly_budget, Decimal::new(1, 1));
}

#[rstest]
#[case::name(r#"{"name": "", "model": "m", "context": "c"}"#, "name")]
#[case::context(&format!(r#"{{"name": "n", "model": "m", "context": "{}"}}"#, "\u{e9}".repeat(6001)), "context")]
#[case::model(&format!(r#"{{"name": "n", "model": "{}", "context": "c"}}"#, "m".repeat(201)), "model")]
#[case::filter_key(
    r#"{"name": "n", "model": "m", "context": "c", "filters": [{"key": "", "value": "v"}]}"#,
    "key"
)]
#[case::check_instruction(
    r#"{"name": "n", "model": "m", "checks": [{"id": "a", "instruction": "ok"}]}"#,
    "instruction"
)]
fn settings_reject_text_out_of_bounds(#[case] json: &str, #[case] field: &str) {
    let RecordError::Length { field: found, .. } = rule::<LensSettings>(json) else {
        panic!("expected a length failure");
    };
    assert_eq!(found, field);
}

#[rstest]
#[case::no_lookback(r#""lookback_hours": 0"#, "lookback_hours")]
#[case::lookback_past_a_year(r#""lookback_hours": 8761"#, "lookback_hours")]
#[case::interval_past_a_week(r#""interval_minutes": 10081"#, "interval_minutes")]
#[case::empty_sample(r#""sample_size": 0"#, "sample_size")]
#[case::no_concurrency(r#""concurrency": 0"#, "concurrency")]
#[case::zero_percent(r#""sample_percent": 0"#, "sample_percent")]
#[case::percent_past_all(r#""sample_percent": 100.5"#, "sample_percent")]
#[case::free_budget(r#""monthly_budget": 0"#, "monthly_budget")]
#[case::budget_past_cap(r#""monthly_budget": 100000.01"#, "monthly_budget")]
fn settings_reject_numbers_out_of_range(#[case] field_json: &str, #[case] field: &'static str) {
    let json = format!(r#"{{"name": "n", "model": "m", "context": "c", {field_json}}}"#);
    assert_eq!(rule::<LensSettings>(&json), RecordError::Range { field });
}

const EVIDENCE: &str = r#"{"execution_id": "e", "span_id": "s", "quote": "q"}"#;
const DRAFT: &str =
    r#"{"title": "Tit", "description": "ten chars!", "check_id": "c", "evidence": "#;

#[rstest]
#[case::nine_filters(
    rule::<LensSettings>,
    &format!(r#"{{"name": "n", "model": "m", "context": "c", "filters": [{}]}}"#, [r#"{"key": "k", "value": "v"}"#; 9].join(",")),
    "filters must hold 0 to 8 items, got 9"
)]
#[case::no_evidence(rule::<FindingDraft>, &format!("{DRAFT}[]}}"), "evidence must hold 1 to 20 items, got 0")]
#[case::evidence_past_cap(
    rule::<FindingDraft>,
    &format!("{DRAFT}[{}]}}", [EVIDENCE; 21].join(",")),
    "evidence must hold 1 to 20 items, got 21"
)]
#[case::empty_quote(rule::<Evidence>, r#"{"execution_id": "e", "span_id": "s", "quote": ""}"#, "quote must be 1 to 1000 characters long, got 0")]
#[case::analysis_key(
    rule::<Worker>,
    r#"{"id": "w", "name": "n", "scope": {}, "last_seen": "2026-01-01T00:00:00Z", "analysis_key_id": "ABC"}"#,
    "analysis_key_id does not match its pattern"
)]
#[case::run_lookback(rule::<RunRequest>, r#"{"lookback_hours": 0}"#, "lookback_hours is out of range")]
#[case::empty_prompt(rule::<ModelRequest>, r#"{"prompt": "", "purpose": "extract"}"#, "prompt must be 1 to 100000 characters long, got 0")]
fn records_keep_their_limits(
    #[case] check: fn(&str) -> RecordError,
    #[case] json: &str,
    #[case] message: &str,
) {
    assert_eq!(check(json).to_string(), message);
}

#[rstest]
#[case::extra_field(br#"{"name": "n", "model": "m", "context": "c", "temperature": 1}"#)]
#[case::extra_nested_field(
    br#"{"name": "n", "model": "m", "checks": [{"id": "a", "instruction": "abc", "on": 1}]}"#
)]
#[case::unknown_source(br#"{"name": "n", "model": "m", "context": "c", "source": "logs"}"#)]
#[case::repeated_field(br#"{"name": "n", "name": "o", "model": "m", "context": "c"}"#)]
#[case::not_utf8(b"{\"name\": \"\xff\", \"model\": \"m\", \"context\": \"c\"}")]
fn settings_refuse_unknown_fields_and_values(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode::<LensSettings>(bytes),
        Err(DecodeError::Json(_))
    ));
}

fn replay<T>(bytes: &[u8])
where
    T: DeserializeOwned + Validate + Serialize,
{
    if let Ok(record) = decode::<T>(bytes) {
        serde_json::to_vec(&record).unwrap();
    }
}

#[rstest]
fn replays_the_lens_model_corpus(#[files("fuzz/corpus/lens_model/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    replay::<LensSettings>(&bytes);
    replay::<Lens>(&bytes);
    replay::<LensList>(&bytes);
    replay::<Job>(&bytes);
    replay::<Worker>(&bytes);
    replay::<Claim>(&bytes);
    replay::<JobResult>(&bytes);
    replay::<Progress>(&bytes);
    replay::<RunRequest>(&bytes);
    replay::<FindingUpdate>(&bytes);
    replay::<ModelRequest>(&bytes);
    replay::<ModelResult>(&bytes);
    replay::<ExecutionContent>(&bytes);
    replay::<Sample>(&bytes);
    replay::<Evidence>(&bytes);
    replay::<FindingDraft>(&bytes);
}
