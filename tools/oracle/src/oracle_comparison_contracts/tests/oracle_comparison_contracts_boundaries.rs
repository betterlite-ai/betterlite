use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::slice;

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::case::{Compare, LoadedCase, load_sdk_http_cases, parse_case};
use crate::compare::compare as compare_responses;
use crate::http::Response;
use crate::oracle_comparison_contracts::{
    ContractError, ExpectedDifference, NumericRule, compare_documents, validate,
};
use crate::routing_scenarios::compare::flatten;
use crate::runner::comparison_cases;

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn numeric(pointer: &str, tolerance: Decimal) -> NumericRule {
    NumericRule {
        pointer: pointer.to_owned(),
        absolute_tolerance: tolerance,
        why: "the pointer carries a float diagnostic".to_owned(),
    }
}

fn expected(pointer: &str, reference: Value, rust: Value) -> ExpectedDifference {
    ExpectedDifference {
        pointer: pointer.to_owned(),
        reference,
        rust,
        why: "the declared difference pins both sides".to_owned(),
        link: String::new(),
    }
}

#[test]
fn two_numeric_rules_cannot_name_the_same_pointer() {
    let rules = [
        numeric("/score", Decimal::ONE),
        numeric("/score", Decimal::ONE),
    ];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::Duplicate { pointer, .. }) if pointer == "/score"
    ));
}

#[test]
fn a_numeric_rule_and_an_expected_difference_cannot_share_a_pointer() {
    let numeric_rules = [numeric("/detail", Decimal::ONE)];
    let differences = [expected("/detail", value("1"), value("2"))];
    assert!(matches!(
        validate(&numeric_rules, &differences),
        Err(ContractError::Duplicate { pointer, .. }) if pointer == "/detail"
    ));
}

#[rstest]
#[case::numeric_names_the_parent("/score", "/score/detail")]
#[case::expected_names_the_parent("/score/detail", "/score")]
fn overlapping_pointers_are_rejected(
    #[case] numeric_pointer: &str,
    #[case] expected_pointer: &str,
) {
    let numeric_rules = [numeric(numeric_pointer, Decimal::ONE)];
    let differences = [expected(expected_pointer, value("1"), value("2"))];
    assert!(matches!(
        validate(&numeric_rules, &differences),
        Err(ContractError::Overlap { .. })
    ));
}

#[rstest]
#[case::no_leading_slash("score")]
#[case::bad_escape("/score~2")]
#[case::trailing_tilde("/score~")]
fn a_pointer_that_is_not_a_json_pointer_is_rejected(#[case] pointer: &str) {
    let rules = [numeric(pointer, Decimal::ONE)];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::Pointer { .. })
    ));
}

#[test]
fn a_negative_tolerance_is_rejected() {
    let rules = [numeric("/score", Decimal::NEGATIVE_ONE)];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::NegativeTolerance { pointer }) if pointer == "/score"
    ));
}

#[rstest]
#[case::numeric(true)]
#[case::expected_difference(false)]
fn a_rule_needs_a_reason(#[case] numeric_rule: bool) {
    let numeric_rules = [numeric("/score", Decimal::ONE)];
    let differences = [expected("/detail", value("1"), value("2"))];
    let outcome = if numeric_rule {
        let mut blank = numeric_rules[0].clone();
        blank.why = "  ".to_owned();
        validate(&[blank], &[])
    } else {
        let mut blank = differences[0].clone();
        blank.why = String::new();
        validate(&[], &[blank])
    };
    assert!(matches!(outcome, Err(ContractError::MissingText { .. })));
}

#[test]
fn a_link_that_is_not_https_is_rejected() {
    let mut rule = expected("/detail", value("1"), value("2"));
    rule.link = "http://example.invalid/litellm".to_owned();
    assert!(matches!(
        validate(&[], &[rule]),
        Err(ContractError::Link { .. })
    ));
}

#[rstest]
#[case::selected("/output/selected")]
#[case::tier("/output/tier")]
#[case::status_code("/billing/status_code")]
#[case::count("/values/retry_count")]
#[case::tokens("/usage/total_tokens")]
#[case::spend("/billing/spend")]
#[case::provider_requests("/upstream/provider_requests")]
#[case::permission("/auth/allowed")]
fn a_tolerance_on_a_decision_pointer_is_rejected(#[case] pointer: &str) {
    let rules = [numeric(pointer, Decimal::new(1, 12))];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::ExactOnly { .. })
    ));
}

#[test]
fn a_score_pointer_may_carry_a_tolerance() {
    let rules = [numeric("/output/score", Decimal::new(1, 12))];
    validate(&rules, &[]).unwrap();
}

#[test]
fn an_expected_difference_naming_a_missing_litellm_path_is_a_mismatch() {
    let reference = value(r#"{"other": 1}"#);
    let rust = value(r#"{"detail": 2}"#);
    let differences = [expected("/detail", value("2"), value("2"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
    assert!(error.message.contains("missing"));
}

#[test]
fn a_missing_betterlite_path_is_a_mismatch() {
    let reference = value(r#"{"detail": 2}"#);
    let rust = value(r#"{"other": 2}"#);
    let differences = [expected("/detail", value("2"), value("2"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
}

#[test]
fn a_numeric_rule_on_a_non_number_is_a_mismatch() {
    let reference = value(r#"{"score": "high"}"#);
    let rust = value(r#"{"score": "high"}"#);
    let rules = [numeric("/score", Decimal::ONE)];
    let error = compare_documents(&reference, &rust, &rules, &[], false).unwrap_err();
    assert_eq!(error.pointer, "/score");
}

#[test]
fn a_declared_type_that_differs_from_the_value_is_a_mismatch() {
    let reference = value(r#"{"detail": "text"}"#);
    let rust = value(r#"{"detail": "text"}"#);
    let differences = [expected("/detail", value("1"), value("1"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
    assert!(error.message.contains("type"));
}

#[rstest]
#[case::at_the_boundary("1.000000000001", true)]
#[case::past_the_boundary("1.000000000002", false)]
fn the_absolute_tolerance_includes_its_boundary(#[case] rust_score: &str, #[case] passes: bool) {
    let reference = value(r#"{"score": 1.0}"#);
    let rust = value(&format!(r#"{{"score": {rust_score}}}"#));
    let rules = [numeric("/score", Decimal::new(1, 12))];
    let outcome = compare_documents(&reference, &rust, &rules, &[], false);
    assert_eq!(outcome.is_ok(), passes);
}

#[test]
fn a_betterlite_value_that_differs_from_the_declared_rust_value_is_a_mismatch() {
    let reference = value(r#"{"detail": 2}"#);
    let rust = value(r#"{"detail": 3}"#);
    let differences = [expected("/detail", value("2"), value("2"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
    assert!(error.message.contains("betterlite"));
}

const HTTP_CASE: &str = "
name: declared
config: |
  model_list: []
env: {}
upstream: []
request:
  method: POST
  path: /v1/chat/completions
  headers: {content-type: application/json}
  body: {model: m}
compare:
  headers: []
  headers_present: []
  upstream_headers: []
  ignore: []
  expected_difference:
    - {pointer: /detail, reference: from-litellm, rust: from-betterlite, why: declared, link: ''}
";

const SCENARIO_CASE: &str = "
name: declared
config: |
  model_list: []
env: {}
upstream: []
scenario:
  steps:
    - request:
        method: POST
        path: /v1/chat/completions
        headers: {content-type: application/json}
        body: {model: m}
      compare:
        headers: []
        headers_present: []
        upstream_headers: []
        ignore: []
        expected_difference:
          - {pointer: /detail, reference: from-litellm, rust: from-betterlite, why: declared, link: ''}
    - concurrent:
        requests:
          - request:
              method: POST
              path: /v1/chat/completions
              headers: {content-type: application/json}
              body: {model: m}
            compare:
              headers: []
              headers_present: []
              upstream_headers: []
              ignore: []
              expected_difference:
                - {pointer: /detail, reference: from-litellm, rust: from-betterlite, why: declared, link: ''}
          - request:
              method: POST
              path: /v1/chat/completions
              headers: {content-type: application/json}
              body: {model: m}
            compare:
              headers: []
              headers_present: []
              upstream_headers: []
              ignore: []
              expected_difference:
                - {pointer: /detail, reference: from-litellm, rust: from-betterlite, why: declared, link: ''}
    - join: {}
";

fn response(detail: &str) -> Response {
    Response {
        status: 200,
        headers: BTreeMap::new(),
        body: value(&format!(r#"{{"detail": "{detail}"}}"#)),
    }
}

fn assert_self_check_is_asserted(compare: &Compare, self_check: bool) {
    let contracts = compare.contracts.as_deref().unwrap();
    assert_eq!(contracts.self_check, self_check);
    let litellm = response("from-litellm");
    let outcome = compare_responses(&litellm, &litellm, &[], &[], compare);
    assert_eq!(outcome.is_ok(), self_check);
}

#[test]
fn self_check_reaches_the_compare_of_an_http_case() {
    let LoadedCase::Http(case) = parse_case(Path::new("declared.yaml"), HTTP_CASE).unwrap() else {
        panic!("an http case was expected");
    };
    assert_self_check_is_asserted(&case.with_self_check(true).compare, true);
    assert_self_check_is_asserted(&case.with_self_check(false).compare, false);
    let group = comparison_cases(slice::from_ref(&case), true);
    assert_self_check_is_asserted(&group[0].compare, true);
}

#[rstest]
#[case::http_step(0)]
#[case::first_concurrent_request(1)]
#[case::second_concurrent_request(2)]
fn self_check_reaches_every_step_compare_of_a_scenario_case(#[case] index: usize) {
    let LoadedCase::Scenario(case) = parse_case(Path::new("declared.yaml"), SCENARIO_CASE).unwrap()
    else {
        panic!("a scenario case was expected");
    };
    let marked = case.with_self_check(true);
    let steps = flatten(&marked.scenario);
    assert_eq!(steps.len(), 3);
    assert_self_check_is_asserted(&steps[index].compare, true);
}

#[rstest]
#[case("numeric:\n    - {pointer: /score, absolute_tolerance: 0.1, why: float noise}")]
#[case(
    "expected_difference:\n    - {pointer: /detail, reference: 2, rust: 3, why: declared, link: \"\"}"
)]
fn an_sdk_http_case_cannot_declare_a_comparison_rule(#[case] rule: &str) {
    let text = format!(
        "kind: sdk_http\nname: rule\ncall:\n  openai_chat:\n    model: gpt-4o-mini\n    messages: [{{role: user, content: hello}}]\n    api_key: sk-test\n    timeout_seconds: 10\nupstream:\n  - status: 200\n    headers: {{content-type: application/json}}\n    body: {{}}\ncompare:\n  headers: []\n  headers_present: []\n  upstream_headers: []\n  ignore: []\n  {rule}\n"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("case.yaml");
    fs::write(&path, text).unwrap();
    let error = load_sdk_http_cases(&[path]).unwrap_err();
    assert!(error.to_string().contains("compare.numeric"), "{error}");
}
