use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde_json::Value;

use super::{ExpectedDifference, Mismatch, NumericRule, validate};

pub(crate) fn compare_documents(
    reference: &Value,
    rust: &Value,
    numeric: &[NumericRule],
    expected_difference: &[ExpectedDifference],
    self_check: bool,
) -> Result<Vec<String>, Mismatch> {
    validate(numeric, expected_difference).map_err(|error| Mismatch {
        pointer: error.pointer().to_owned(),
        message: error.to_string(),
    })?;
    let mut excluded = Vec::with_capacity(numeric.len().saturating_add(expected_difference.len()));
    for rule in expected_difference {
        assert_expected(reference, rust, rule, self_check)?;
        excluded.push(rule.pointer.clone());
    }
    for rule in numeric {
        assert_numeric(reference, rust, rule)?;
        excluded.push(rule.pointer.clone());
    }
    Ok(excluded)
}

fn assert_expected(
    reference: &Value,
    rust: &Value,
    rule: &ExpectedDifference,
    self_check: bool,
) -> Result<(), Mismatch> {
    let failure = |message: String| Mismatch {
        pointer: rule.pointer.clone(),
        message,
    };
    let reference_value = reference.pointer(&rule.pointer).ok_or_else(|| {
        failure(
            "compare.expected_difference names a path missing from the LiteLLM document".to_owned(),
        )
    })?;
    assert_value(reference_value, &rule.reference, "the LiteLLM").map_err(failure)?;
    let missing = if self_check {
        "compare.expected_difference names a path missing from the second LiteLLM run"
    } else {
        "compare.expected_difference names a path missing from the betterlite document"
    };
    let rust_value = rust
        .pointer(&rule.pointer)
        .ok_or_else(|| failure(missing.to_owned()))?;
    if self_check {
        assert_value(rust_value, &rule.reference, "the self-check LiteLLM").map_err(failure)?;
    } else {
        assert_value(rust_value, &rule.rust, "the betterlite").map_err(failure)?;
    }
    Ok(())
}

fn assert_value(actual: &Value, expected: &Value, side: &str) -> Result<(), String> {
    let actual_kind = kind_name(actual);
    let expected_kind = kind_name(expected);
    if actual_kind != expected_kind {
        return Err(format!(
            "compare.expected_difference: {side} value has type {actual_kind} but the rule declares {expected_kind}"
        ));
    }
    if !json_equal(actual, expected) {
        return Err(format!(
            "compare.expected_difference: {side} value differs from the declared value"
        ));
    }
    Ok(())
}

fn assert_numeric(reference: &Value, rust: &Value, rule: &NumericRule) -> Result<(), Mismatch> {
    let failure = |message: String| Mismatch {
        pointer: rule.pointer.clone(),
        message,
    };
    let reference_value =
        numeric_value(reference, &rule.pointer, "the LiteLLM document").map_err(failure)?;
    let rust_value =
        numeric_value(rust, &rule.pointer, "the betterlite document").map_err(failure)?;
    let difference = reference_value
        .checked_sub(rust_value)
        .ok_or_else(|| failure("compare.numeric values are too large to compare".to_owned()))?
        .abs();
    if difference > rule.absolute_tolerance {
        return Err(failure(format!(
            "compare.numeric: the values differ by {difference}, past the absolute tolerance {}",
            rule.absolute_tolerance
        )));
    }
    Ok(())
}

fn numeric_value(document: &Value, pointer: &str, side: &str) -> Result<Decimal, String> {
    let Some(value) = document.pointer(pointer) else {
        return Err(format!("compare.numeric names a path missing from {side}"));
    };
    let Value::Number(number) = value else {
        return Err(format!("compare.numeric: {side} value is not a number"));
    };
    Decimal::from_str(&number.to_string()).map_err(|source| {
        format!("compare.numeric: {side} value is not a decimal number: {source}")
    })
}

const fn kind_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_equal(reference: &Value, rust: &Value) -> bool {
    match (reference, rust) {
        (Value::Number(reference_number), Value::Number(rust_number)) => {
            let reference_decimal = Decimal::from_str(&reference_number.to_string());
            let rust_decimal = Decimal::from_str(&rust_number.to_string());
            match (reference_decimal, rust_decimal) {
                (Ok(reference_value), Ok(rust_value)) => reference_value == rust_value,
                (Err(_), _) | (_, Err(_)) => reference_number == rust_number,
            }
        }
        (Value::Array(reference_items), Value::Array(rust_items)) => {
            reference_items.len() == rust_items.len()
                && reference_items
                    .iter()
                    .zip(rust_items)
                    .all(|(reference_item, rust_item)| json_equal(reference_item, rust_item))
        }
        (Value::Object(reference_fields), Value::Object(rust_fields)) => {
            reference_fields.len() == rust_fields.len()
                && reference_fields.iter().all(|(name, reference_value)| {
                    rust_fields
                        .get(name)
                        .is_some_and(|rust_value| json_equal(reference_value, rust_value))
                })
        }
        (Value::Null, Value::Null) | (Value::Bool(_), Value::Bool(_)) => reference == rust,
        (Value::String(reference_text), Value::String(rust_text)) => reference_text == rust_text,
        (
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_)
            | Value::Object(_),
            _,
        ) => false,
    }
}
