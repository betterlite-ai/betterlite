use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::oracle_comparison_contracts::{
    ExpectedDifference, NumericRule, compare_documents, validate,
};
use crate::router_probes::comparison::{Documents, compare};
use crate::router_probes::single_candidate::Output;
use crate::router_probes::{ProbeOutput, Reply};
use crate::routing_component_oracle::{ProbeCompare, compare_probe_replies};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn numeric(pointer: &str, tolerance: Decimal, why: &str) -> NumericRule {
    NumericRule {
        pointer: pointer.to_owned(),
        absolute_tolerance: tolerance,
        why: why.to_owned(),
    }
}

fn expected(pointer: &str, reference: Value, rust: Value) -> ExpectedDifference {
    ExpectedDifference {
        pointer: pointer.to_owned(),
        reference,
        rust,
        why: "the declared difference pins both sides".to_owned(),
        link: "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_strategy/complexity_router/llm_v2.py#L200-L224".to_owned(),
    }
}

#[test]
fn a_numeric_rule_excludes_a_float_diagnostic_within_tolerance() {
    let reference = value(r#"{"output": {"score": 0.5}}"#);
    let rust = value(r#"{"output": {"score": 0.500000000001}}"#);
    let rules = [numeric(
        "/output/score",
        Decimal::new(1, 12),
        "scores are float diagnostics",
    )];
    let excluded = compare_documents(&reference, &rust, &rules, &[], false).unwrap();
    assert_eq!(excluded, vec!["/output/score".to_owned()]);
}

#[test]
fn an_expected_difference_asserts_both_sides_and_excludes_the_pointer() {
    let reference = value(r#"{"detail": {"model": "reference"}}"#);
    let rust = value(r#"{"detail": {"model": "betterlite"}}"#);
    let rules = [expected(
        "/detail",
        value(r#"{"model": "reference"}"#),
        value(r#"{"model": "betterlite"}"#),
    )];
    let excluded = compare_documents(&reference, &rust, &[], &rules, false).unwrap();
    assert_eq!(excluded, vec!["/detail".to_owned()]);
}

#[test]
fn self_check_asserts_the_litellm_value_on_both_sides() {
    let reference = value(r#"{"selected": "diagnostic"}"#);
    let rust = value(r#"{"selected": "diagnostic"}"#);
    let rules = [expected(
        "/selected",
        value(r#""diagnostic""#),
        value(r#""a value only betterlite may return""#),
    )];
    let excluded = compare_documents(&reference, &rust, &[], &rules, true).unwrap();
    assert_eq!(excluded, vec!["/selected".to_owned()]);
}

#[test]
fn a_declared_number_matches_the_same_value_in_another_representation() {
    let reference = value(r#"{"count": 1.0}"#);
    let rust = value(r#"{"count": 1}"#);
    let rules = [expected("/count", value("1"), value("1.0"))];
    compare_documents(&reference, &rust, &[], &rules, false).unwrap();
}

#[test]
fn a_zero_tolerance_still_accepts_equal_values() {
    let reference = value(r#"{"latency": 12.5}"#);
    let rust = value(r#"{"latency": 12.5}"#);
    let rules = [numeric("/latency", Decimal::ZERO, "exact diagnostics")];
    let excluded = compare_documents(&reference, &rust, &rules, &[], false).unwrap();
    assert_eq!(excluded, vec!["/latency".to_owned()]);
}

#[test]
fn rules_exclude_every_declared_pointer_in_order() {
    let reference = value(r#"{"score": 1.0, "detail": "reference"}"#);
    let rust = value(r#"{"score": 1.0, "detail": "betterlite"}"#);
    let numeric_rules = [numeric(
        "/score",
        Decimal::new(1, 12),
        "scores are float diagnostics",
    )];
    let differences = [expected(
        "/detail",
        value(r#""reference""#),
        value(r#""betterlite""#),
    )];
    let excluded =
        compare_documents(&reference, &rust, &numeric_rules, &differences, false).unwrap();
    assert_eq!(excluded, vec!["/detail".to_owned(), "/score".to_owned()]);
}

#[test]
fn no_rules_exclude_nothing() {
    let reference = value(r#"{"a": 1}"#);
    let rust = value(r#"{"a": 1}"#);
    assert_eq!(
        compare_documents(&reference, &rust, &[], &[], false).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn an_https_link_and_an_empty_link_are_both_accepted() {
    let mut with_empty_link = expected("/detail", value("1"), value("2"));
    with_empty_link.link = String::new();
    validate(&[], &[with_empty_link]).unwrap();
    validate(&[], &[expected("/detail", value("1"), value("2"))]).unwrap();
}

#[rstest]
#[case::declared_difference(
    r#"{"detail": "reference", "other": 1}"#,
    r#"{"detail": "betterlite", "other": 1}"#,
    true,
    false,
    None
)]
#[case::score_within_tolerance(
    r#"{"score": 0.5, "other": 1}"#,
    r#"{"score": 0.5000001, "other": 1}"#,
    false,
    true,
    None
)]
#[case::third_field_after_a_declared_difference(
    r#"{"detail": "reference", "other": 1}"#,
    r#"{"detail": "betterlite", "other": 2}"#,
    true,
    false,
    Some("/other")
)]
#[case::third_field_after_a_tolerated_score(
    r#"{"score": 0.5, "other": 1}"#,
    r#"{"score": 0.5000001, "other": 2}"#,
    false,
    true,
    Some("/other")
)]
fn comparison_drops_each_asserted_pointer_before_the_structural_diff(
    #[case] reference_text: &str,
    #[case] rust_text: &str,
    #[case] with_difference: bool,
    #[case] with_numeric: bool,
    #[case] pointer: Option<&str>,
) {
    let (reference, rust) = (value(reference_text), value(rust_text));
    let differences = [expected(
        "/detail",
        value(r#""reference""#),
        value(r#""betterlite""#),
    )];
    let scores = [numeric("/score", Decimal::new(1, 6), "float diagnostic")];
    let outcome = compare(&Documents {
        reference: &reference,
        rust: &rust,
        numeric: if with_numeric { &scores } else { &[] },
        expected_difference: if with_difference { &differences } else { &[] },
        ignore: &[],
        self_check: false,
        message: "documents differ",
    });
    assert_eq!(
        outcome.as_ref().map(|found| found.pointer.as_str()),
        pointer
    );
    if let Some(found) = outcome {
        assert_eq!(found.message, "documents differ");
    }
}

fn selected(id: &str) -> Reply {
    Reply::Ok {
        output: ProbeOutput::SingleCandidate(Output {
            selected: id.to_owned(),
        }),
        warnings: Vec::new(),
    }
}

#[rstest]
#[case::native_first_python_second("native", "python", false, true)]
#[case::sides_swapped("python", "native", false, false)]
#[case::self_check_compares_litellm_twice("python", "python", true, true)]
#[case::self_check_off_expects_the_betterlite_value("python", "python", false, false)]
fn a_probe_declares_the_litellm_value_second_and_the_betterlite_value_first(
    #[case] first: &str,
    #[case] second: &str,
    #[case] self_check: bool,
    #[case] passes: bool,
) {
    let compare: ProbeCompare = serde_json::from_str(
        r#"{"expected_difference": [{"pointer": "/output/single_candidate/selected",
            "reference": "python", "rust": "native", "why": "the picks differ by design", "link": ""}]}"#,
    )
    .unwrap();
    let outcome =
        compare_probe_replies(&selected(first), &selected(second), &compare, self_check).unwrap();
    assert_eq!(outcome.is_none(), passes);
}
