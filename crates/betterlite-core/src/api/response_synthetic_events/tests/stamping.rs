#![cfg(test)]

use rstest::rstest;

use super::*;

#[test]
fn stamping_without_calculator_changes_nothing() {
    let mut transformed = response_with(
        "[]",
        r#","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#,
    );
    stamp_usage_cost(&mut transformed, None::<&FixedCost>);
    assert!(transformed.usage.as_ref().unwrap().cost.is_none());
    let mut object =
        json(r#"{"id":"r","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#);
    if let Json::Object(fields) = &mut object {
        stamp_object_usage_cost(fields, None::<&FixedCost>);
    }
    assert_eq!(
        field(&object, "usage"),
        &json(r#"{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#)
    );
}

#[test]
fn stamping_skips_missing_usage() {
    let mut transformed = response("[]");
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    stamp_usage_cost(&mut transformed, Some(&calculator));
    assert!(transformed.usage.is_none());
    assert!(calculator.calls().is_empty());
}

#[rstest]
#[case::zero("0", "0")]
#[case::zero_decimal("0.0", "0.0")]
#[case::decimal("2.5", "2.5")]
fn stamping_keeps_existing_cost(#[case] cost: &str, #[case] expected: &str) {
    let extra = format!(
        r#","usage":{{"input_tokens":1,"output_tokens":1,"total_tokens":2,"cost":{cost}}}"#
    );
    let mut transformed = response_with("[]", &extra);
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    stamp_usage_cost(&mut transformed, Some(&calculator));
    assert_eq!(
        transformed
            .usage
            .as_ref()
            .unwrap()
            .cost
            .unwrap()
            .to_string(),
        expected
    );
    assert!(calculator.calls().is_empty());
}

#[test]
fn stamping_keeps_existing_cost_when_the_calculator_fails() {
    let mut transformed = response_with(
        "[]",
        r#","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2,"cost":0.5}"#,
    );
    let calculator = FixedCost::failing();
    stamp_usage_cost(&mut transformed, Some(&calculator));
    assert!(calculator.calls().is_empty());
}

#[test]
fn stamping_attaches_positive_costs() {
    let mut transformed = response_with(
        "[]",
        r#","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#,
    );
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    stamp_usage_cost(&mut transformed, Some(&calculator));
    let usage = transformed.usage.as_ref().unwrap();
    assert_eq!(usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(calculator.calls(), ["resp_a"]);
    let usage_json = to_json(usage);
    assert_eq!(number_text(field(&usage_json, "cost")), "0.25");
    let mut whole_transformed = response_with(
        "[]",
        r#","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#,
    );
    let whole_calculator = FixedCost::returning(Decimal::new(3, 0));
    stamp_usage_cost(&mut whole_transformed, Some(&whole_calculator));
    let whole_usage = whole_transformed.usage.as_ref().unwrap();
    assert_eq!(number_text(field(&to_json(whole_usage), "cost")), "3.0");
}

#[rstest]
#[case::zero(FixedCost::returning(Decimal::ZERO))]
#[case::negative(FixedCost::returning(Decimal::new(-1, 0)))]
#[case::silent(FixedCost::silent())]
#[case::failing(FixedCost::failing())]
fn stamping_ignores_zero_negative_null_and_errors(#[case] calculator: FixedCost) {
    let mut transformed = response_with(
        "[]",
        r#","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#,
    );
    stamp_usage_cost(&mut transformed, Some(&calculator));
    assert!(transformed.usage.as_ref().unwrap().cost.is_none());
    assert_eq!(calculator.calls(), ["resp_a"]);
}

#[test]
fn stamping_happens_before_lifecycle_copies() {
    let output = format!("[{}]", message_with_text("msg_a", "hi"));
    let mut transformed = response_with(
        &output,
        r#","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#,
    );
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    let items = build(&mut transformed, Some(&calculator), 2);
    assert_eq!(
        transformed.usage.as_ref().unwrap().cost,
        Some(Decimal::new(25, 2))
    );
    assert_eq!(
        transformed.usage.as_ref().unwrap().input_tokens,
        1,
        "the builder does not estimate usage"
    );
    let created = event_of(&items, "response.created");
    assert_eq!(
        number_text(at(created, &["response", "usage", "cost"])),
        "0.25"
    );
    let in_progress = event_of(&items, "response.in_progress");
    assert_eq!(
        number_text(at(in_progress, &["response", "usage", "cost"])),
        "0.25"
    );
    let completed = items.last().unwrap();
    assert_eq!(
        number_text(at(completed, &["response", "usage", "cost"])),
        "0.25"
    );
}

#[test]
fn object_stamping_rebinds_dict_usage_to_model() {
    let mut object = json(
        r#"{"id":"r3","usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"extra_note":"x"}}"#,
    );
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    if let Json::Object(fields) = &mut object {
        stamp_object_usage_cost(fields, Some(&calculator));
    }
    assert_eq!(
        field(&object, "usage"),
        &json(
            r#"{"input_tokens":1,"input_tokens_details":null,"output_tokens":2,"output_tokens_details":null,"total_tokens":3,"cost":0.25,"extra_note":"x"}"#
        )
    );
    assert_eq!(calculator.calls(), ["r3"]);
}

#[rstest]
#[case::dict_cost(r#"{"total_cost":0.4}"#, "0.4")]
#[case::zero("0", "0")]
#[case::decimal("2.5", "2.5")]
fn object_stamping_prevents_recalculation_for_existing_costs(
    #[case] cost: &str,
    #[case] expected: &str,
) {
    let raw = format!(
        r#"{{"id":"r3","usage":{{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":{cost}}}}}"#
    );
    let mut object = json(&raw);
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    if let Json::Object(fields) = &mut object {
        stamp_object_usage_cost(fields, Some(&calculator));
    }
    assert_eq!(
        number_text(at(&object, &["usage", "cost"])),
        expected,
        "usage {cost}"
    );
    assert!(calculator.calls().is_empty());
}

#[rstest]
#[case::missing_usage(r#"{"id":"r3"}"#)]
#[case::null_usage(r#"{"id":"r3","usage":null}"#)]
#[case::wrong_token_type(r#"{"id":"r3","usage":{"input_tokens":"many"}}"#)]
#[case::bool_cost(
    r#"{"id":"r3","usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":true}}"#
)]
#[case::bool_usage(r#"{"id":"r3","usage":true}"#)]
#[case::list_usage(r#"{"id":"r3","usage":[1]}"#)]
fn object_stamping_ignores_unusable_usage(#[case] raw: &str) {
    let mut object = json(raw);
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    let before = object.clone();
    if let Json::Object(fields) = &mut object {
        stamp_object_usage_cost(fields, Some(&calculator));
    }
    assert_eq!(object, before, "usage untouched for {raw}");
    assert!(calculator.calls().is_empty());
}

#[rstest]
#[case::zero(FixedCost::returning(Decimal::ZERO))]
#[case::negative(FixedCost::returning(Decimal::new(-1, 0)))]
#[case::silent(FixedCost::silent())]
#[case::failing(FixedCost::failing())]
fn object_stamping_ignores_nonpositive_costs(#[case] calculator: FixedCost) {
    let mut object = json(
        r#"{"id":"r3","usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":null}}"#,
    );
    if let Json::Object(fields) = &mut object {
        stamp_object_usage_cost(fields, Some(&calculator));
    }
    assert_eq!(
        at(&object, &["usage", "cost"]),
        &Json::Null,
        "costs that are not positive stay absent"
    );
    assert_eq!(
        at(&object, &["usage", "input_tokens_details"]),
        &Json::Null,
        "usage is rebound to the model even when no cost is attached"
    );
    assert_eq!(calculator.calls(), ["r3"]);
}

#[test]
fn object_stamping_attaches_positive_costs() {
    let mut object = json(
        r#"{"id":"r3","usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":null}}"#,
    );
    let calculator = FixedCost::returning(Decimal::new(25, 2));
    if let Json::Object(fields) = &mut object {
        stamp_object_usage_cost(fields, Some(&calculator));
    }
    assert_eq!(number_text(at(&object, &["usage", "cost"])), "0.25");
    assert_eq!(calculator.calls(), ["r3"]);
}
