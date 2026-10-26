#![cfg(test)]

use rstest::rstest;

use super::{
    AgenticLoopCeilingError, AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS,
    validated_max_agentic_loops,
};

#[rstest]
#[case::integer(AgenticLoopValue::Integer(5), 5)]
#[case::minimum(AgenticLoopValue::Integer(1), 1)]
#[case::maximum(AgenticLoopValue::Integer(i64::from(u32::MAX)), u32::MAX)]
#[case::float(AgenticLoopValue::Float(5.0), 5)]
#[case::float_minimum(AgenticLoopValue::Float(1.0), 1)]
#[case::text(AgenticLoopValue::Text(" 5 ".to_owned()), 5)]
#[case::signed_text(AgenticLoopValue::Text("+005".to_owned()), 5)]
#[case::separators(AgenticLoopValue::Text("1_000".to_owned()), 1000)]
fn agentic_loop_ceiling_accepts(#[case] value: AgenticLoopValue, #[case] expected: u32) {
    assert_eq!(
        validated_max_agentic_loops(&value, "field"),
        Ok(Some(expected))
    );
}

#[rstest]
#[case::true_value(AgenticLoopValue::Boolean(true))]
#[case::false_value(AgenticLoopValue::Boolean(false))]
#[case::fraction(AgenticLoopValue::Float(1.5))]
#[case::nan(AgenticLoopValue::Float(f64::NAN))]
#[case::infinity(AgenticLoopValue::Float(f64::INFINITY))]
#[case::negative_infinity(AgenticLoopValue::Float(f64::NEG_INFINITY))]
#[case::float_overflow(AgenticLoopValue::Float(f64::MAX))]
#[case::integer_overflow(AgenticLoopValue::Integer(i64::MAX))]
#[case::above_maximum(AgenticLoopValue::Integer(4_294_967_296))]
#[case::text_overflow(AgenticLoopValue::Text("9223372036854775808".to_owned()))]
#[case::non_numeric(AgenticLoopValue::Text("five".to_owned()))]
#[case::unicode_digits(AgenticLoopValue::Text("\u{661}".to_owned()))]
#[case::decimal_text(AgenticLoopValue::Text("5.0".to_owned()))]
#[case::empty(AgenticLoopValue::Text(String::new()))]
#[case::leading_separator(AgenticLoopValue::Text("_1".to_owned()))]
#[case::trailing_separator(AgenticLoopValue::Text("1_".to_owned()))]
#[case::repeated_separator(AgenticLoopValue::Text("1__0".to_owned()))]
#[case::sign_separator(AgenticLoopValue::Text("+_1".to_owned()))]
fn agentic_loop_ceiling_rejects(#[case] value: AgenticLoopValue) {
    let field = "litellm_settings.max_agentic_loops";
    let error = validated_max_agentic_loops(&value, field).unwrap_err();
    assert_eq!(
        error,
        AgenticLoopCeilingError::NotWholeNumber {
            field: field.to_owned()
        }
    );
    assert!(error.to_string().contains(field));
}

#[rstest]
#[case::zero(AgenticLoopValue::Integer(0), 0)]
#[case::negative(AgenticLoopValue::Integer(-1), -1)]
#[case::float_zero(AgenticLoopValue::Float(-0.0), 0)]
#[case::negative_text(AgenticLoopValue::Text("-2".to_owned()), -2)]
fn agentic_loop_ceiling_below_minimum(#[case] value: AgenticLoopValue, #[case] expected: i64) {
    let error = validated_max_agentic_loops(&value, "model.max_agentic_loops").unwrap_err();
    assert_eq!(
        error,
        AgenticLoopCeilingError::BelowMinimum {
            field: "model.max_agentic_loops".to_owned(),
            value: expected,
        }
    );
    assert!(error.to_string().contains("model.max_agentic_loops"));
}

#[test]
fn agentic_loop_ceiling_null() {
    assert_eq!(
        validated_max_agentic_loops(&AgenticLoopValue::Null, "field"),
        Ok(None)
    );
    assert_eq!(DEFAULT_MAX_AGENTIC_LOOPS, 3);
}
