use indexmap::IndexMap;
use rstest::rstest;

use super::super::inplacevar::{InplaceError, InplaceOp, inplacevar};
use super::super::value::Value;

#[rstest]
#[case::add("+=", InplaceOp::Add)]
#[case::subtract("-=", InplaceOp::Subtract)]
#[case::multiply("*=", InplaceOp::Multiply)]
#[case::divide("/=", InplaceOp::TrueDivide)]
#[case::floor_divide("//=", InplaceOp::FloorDivide)]
#[case::modulo("%=", InplaceOp::Modulo)]
#[case::power("**=", InplaceOp::Power)]
#[case::matrix("@=", InplaceOp::MatrixMultiply)]
#[case::and("&=", InplaceOp::BitAnd)]
#[case::or("|=", InplaceOp::BitOr)]
#[case::xor("^=", InplaceOp::BitXor)]
#[case::shift_left("<<=", InplaceOp::ShiftLeft)]
#[case::shift_right(">>=", InplaceOp::ShiftRight)]
fn reads_every_augmented_assignment_symbol(#[case] symbol: &str, #[case] expected: InplaceOp) {
    assert_eq!(InplaceOp::from_symbol(symbol), Ok(expected));
    assert_eq!(expected.symbol(), symbol);
}

#[test]
fn an_unknown_operator_is_a_syntax_error() {
    let error = InplaceOp::from_symbol("<>=").unwrap_err();
    assert!(matches!(
        &error,
        InplaceError::UnsupportedOperator { symbol } if symbol == "<>="
    ));
    assert_eq!(
        error.to_string(),
        "augmented assignment \"<>=\" is not supported"
    );
}

#[test]
fn adds_integers_with_overflow_checking() {
    assert_eq!(
        inplacevar(InplaceOp::Add, &Value::Int(2), &Value::Int(3)),
        Ok(Value::Int(5))
    );
    assert_eq!(
        inplacevar(InplaceOp::Add, &Value::Int(i64::MAX), &Value::Int(1)),
        Err(InplaceError::Overflow { symbol: "+=" })
    );
}

#[test]
fn concatenates_the_sequence_types() {
    assert_eq!(
        inplacevar(
            InplaceOp::Add,
            &Value::Str("ab".to_owned()),
            &Value::Str("cd".to_owned())
        ),
        Ok(Value::Str("abcd".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Add,
            &Value::List(vec![Value::Int(1)]),
            &Value::List(vec![Value::Int(2)])
        ),
        Ok(Value::List(vec![Value::Int(1), Value::Int(2)]))
    );
    assert!(matches!(
        inplacevar(
            InplaceOp::Add,
            &Value::Dict(IndexMap::new()),
            &Value::Dict(IndexMap::new())
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[test]
fn subtracts_and_multiplies_integers() {
    assert_eq!(
        inplacevar(InplaceOp::Subtract, &Value::Int(10), &Value::Int(4)),
        Ok(Value::Int(6))
    );
    assert_eq!(
        inplacevar(InplaceOp::Multiply, &Value::Int(6), &Value::Int(7)),
        Ok(Value::Int(42))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Str("ab".to_owned()),
            &Value::Int(3)
        ),
        Ok(Value::Str("ababab".to_owned()))
    );
}

#[test]
fn divides_like_python() {
    assert_eq!(
        inplacevar(InplaceOp::TrueDivide, &Value::Int(5), &Value::Int(2)),
        Ok(Value::Float(2.5))
    );
    assert_eq!(
        inplacevar(InplaceOp::TrueDivide, &Value::Int(1), &Value::Int(3)),
        Ok(Value::Float(0.333_333_333_333_333_3))
    );
    assert_eq!(
        inplacevar(InplaceOp::TrueDivide, &Value::Int(2), &Value::Int(3)),
        Ok(Value::Float(0.666_666_666_666_666_6))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::TrueDivide,
            &Value::Int(1),
            &Value::Int(30_000_000_000_000)
        ),
        Ok(Value::Float(3.333_333_333_333_333_4e-14))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::TrueDivide,
            &Value::Int(7),
            &Value::Int(0x0020_0000_0000_0001)
        ),
        Ok(Value::Float(7.771_561_172_376_095e-16))
    );
    assert_eq!(
        inplacevar(InplaceOp::FloorDivide, &Value::Int(-7), &Value::Int(2)),
        Ok(Value::Int(-4))
    );
    assert_eq!(
        inplacevar(InplaceOp::Modulo, &Value::Int(-7), &Value::Int(2)),
        Ok(Value::Int(1))
    );
    assert_eq!(
        inplacevar(InplaceOp::TrueDivide, &Value::Int(1), &Value::Int(0)),
        Err(InplaceError::DivideByZero)
    );
}

#[test]
fn shifts_and_bitwise_operators_stay_on_integers() {
    assert_eq!(
        inplacevar(InplaceOp::ShiftLeft, &Value::Int(1), &Value::Int(4)),
        Ok(Value::Int(16))
    );
    assert_eq!(
        inplacevar(InplaceOp::BitOr, &Value::Int(0b1010), &Value::Int(0b0101)),
        Ok(Value::Int(0b1111))
    );
    assert!(matches!(
        inplacevar(
            InplaceOp::BitAnd,
            &Value::Str("x".to_owned()),
            &Value::Int(1)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[test]
fn an_unsupported_operand_names_both_types() {
    let error = inplacevar(
        InplaceOp::Subtract,
        &Value::Str("x".to_owned()),
        &Value::Int(1),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported operand types for -=: str and int"
    );
}

#[test]
fn repeats_bytes_and_tuples_like_the_operator_module() {
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Bytes(b"ab".to_vec()),
            &Value::Int(2)
        ),
        Ok(Value::Bytes(b"abab".to_vec()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Int(2),
            &Value::Bytes(b"ab".to_vec())
        ),
        Ok(Value::Bytes(b"abab".to_vec()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Tuple(vec![Value::Int(1)]),
            &Value::Int(3)
        ),
        Ok(Value::Tuple(vec![
            Value::Int(1),
            Value::Int(1),
            Value::Int(1)
        ]))
    );
}

#[test]
fn a_zero_or_negative_count_gives_an_empty_value() {
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Str("x".to_owned()),
            &Value::Int(-1)
        ),
        Ok(Value::Str(String::new()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::List(vec![Value::Int(1)]),
            &Value::Int(0)
        ),
        Ok(Value::List(Vec::new()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Str("x".to_owned()),
            &Value::Int(0x0010_0001)
        ),
        Err(InplaceError::RepetitionTooLong)
    );
}

#[test]
fn shifts_check_the_result_and_saturate_like_python() {
    assert_eq!(
        inplacevar(InplaceOp::ShiftLeft, &Value::Int(3), &Value::Int(62)),
        Err(InplaceError::Overflow { symbol: "<<=" })
    );
    assert_eq!(
        inplacevar(InplaceOp::ShiftRight, &Value::Int(1), &Value::Int(64)),
        Ok(Value::Int(0))
    );
    assert_eq!(
        inplacevar(InplaceOp::ShiftRight, &Value::Int(-1), &Value::Int(64)),
        Ok(Value::Int(-1))
    );
    assert_eq!(
        inplacevar(InplaceOp::ShiftLeft, &Value::Int(1), &Value::Int(10)),
        Ok(Value::Int(1024))
    );
}

#[test]
fn booleans_act_as_integers_and_keep_boolean_bitwise_results() {
    assert_eq!(
        inplacevar(InplaceOp::Add, &Value::Bool(true), &Value::Int(1)),
        Ok(Value::Int(2))
    );
    assert_eq!(
        inplacevar(InplaceOp::Add, &Value::Int(1), &Value::Bool(true)),
        Ok(Value::Int(2))
    );
    assert_eq!(
        inplacevar(InplaceOp::BitOr, &Value::Bool(false), &Value::Bool(true)),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        inplacevar(InplaceOp::BitAnd, &Value::Bool(true), &Value::Bool(false)),
        Ok(Value::Bool(false))
    );
}

#[test]
fn a_list_extends_from_a_tuple_and_a_dict_updates_from_another() {
    assert_eq!(
        inplacevar(
            InplaceOp::Add,
            &Value::List(vec![Value::Int(1)]),
            &Value::Tuple(vec![Value::Int(2)])
        ),
        Ok(Value::List(vec![Value::Int(1), Value::Int(2)]))
    );
    let mut first = IndexMap::new();
    first.insert("a".to_owned(), Value::Int(1));
    let mut second = IndexMap::new();
    second.insert("b".to_owned(), Value::Int(2));
    let mut merged = first.clone();
    merged.insert("b".to_owned(), Value::Int(2));
    assert_eq!(
        inplacevar(InplaceOp::BitOr, &Value::Dict(first), &Value::Dict(second)),
        Ok(Value::Dict(merged))
    );
}

#[test]
fn a_string_percent_formats_its_value() {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s".to_owned()),
            &Value::Int(1)
        ),
        Ok(Value::Str("1".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%d".to_owned()),
            &Value::Int(5)
        ),
        Ok(Value::Str("5".to_owned()))
    );
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("plain".to_owned()),
            &Value::Int(1)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s %s".to_owned()),
            &Value::Int(1)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[test]
fn a_string_percent_formats_tuple_and_mapping_arguments() {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s %s".to_owned()),
            &Value::Tuple(vec![Value::Str("a".to_owned()), Value::Str("b".to_owned())])
        ),
        Ok(Value::Str("a b".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s".to_owned()),
            &Value::Tuple(vec![Value::Str("a".to_owned())])
        ),
        Ok(Value::Str("a".to_owned()))
    );
    let mut mapping = IndexMap::new();
    mapping.insert("a".to_owned(), Value::Int(1));
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%(a)s".to_owned()),
            &Value::Dict(mapping)
        ),
        Ok(Value::Str("1".to_owned()))
    );
}

#[test]
fn a_string_percent_formats_conversions_and_width() {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%x".to_owned()),
            &Value::Int(255)
        ),
        Ok(Value::Str("ff".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%5d".to_owned()),
            &Value::Int(1)
        ),
        Ok(Value::Str("    1".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%r".to_owned()),
            &Value::Str("a".to_owned())
        ),
        Ok(Value::Str("'a'".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s".to_owned()),
            &Value::None
        ),
        Ok(Value::Str("None".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s".to_owned()),
            &Value::List(vec![Value::Int(1)])
        ),
        Ok(Value::Str("[1]".to_owned()))
    );
}

#[test]
fn a_percent_format_rejects_leftover_and_float_arguments() {
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%%".to_owned()),
            &Value::Int(1)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s".to_owned()),
            &Value::Float(1.0)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%s".to_owned()),
            &Value::Float(1e22)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[test]
fn bytes_percent_formats_an_integer() {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Bytes(b"%d".to_vec()),
            &Value::Int(1)
        ),
        Ok(Value::Bytes(b"1".to_vec()))
    );
}

#[test]
fn a_negative_exponent_gives_a_float() {
    assert_eq!(
        inplacevar(InplaceOp::Power, &Value::Int(2), &Value::Int(-1)),
        Ok(Value::Float(0.5))
    );
    assert_eq!(
        inplacevar(InplaceOp::Power, &Value::Int(2), &Value::Int(-50)),
        Ok(Value::Float(8.881_784_197_001_252e-16))
    );
}

#[test]
fn a_power_beyond_u32_answers_the_small_bases() {
    assert_eq!(
        inplacevar(InplaceOp::Power, &Value::Int(1), &Value::Int(5_000_000_000)),
        Ok(Value::Int(1))
    );
    assert_eq!(
        inplacevar(InplaceOp::Power, &Value::Int(0), &Value::Int(5_000_000_000)),
        Ok(Value::Int(0))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Power,
            &Value::Int(-1),
            &Value::Int(5_000_000_000)
        ),
        Ok(Value::Int(1))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Power,
            &Value::Int(-1),
            &Value::Int(5_000_000_001)
        ),
        Ok(Value::Int(-1))
    );
}

#[test]
fn a_list_extends_from_text_bytes_and_mapping_keys() {
    assert_eq!(
        inplacevar(
            InplaceOp::Add,
            &Value::List(vec![Value::Int(1)]),
            &Value::Str("ab".to_owned())
        ),
        Ok(Value::List(vec![
            Value::Int(1),
            Value::Str("a".to_owned()),
            Value::Str("b".to_owned())
        ]))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Add,
            &Value::List(vec![Value::Int(1)]),
            &Value::Bytes(b"ab".to_vec())
        ),
        Ok(Value::List(vec![
            Value::Int(1),
            Value::Int(97),
            Value::Int(98)
        ]))
    );
    let mut mapping = IndexMap::new();
    mapping.insert("a".to_owned(), Value::Int(2));
    assert_eq!(
        inplacevar(
            InplaceOp::Add,
            &Value::List(vec![Value::Int(1)]),
            &Value::Dict(mapping)
        ),
        Ok(Value::List(vec![Value::Int(1), Value::Str("a".to_owned())]))
    );
}

#[test]
fn a_boolean_count_repeats_by_one_or_zero() {
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Str("ab".to_owned()),
            &Value::Bool(true)
        ),
        Ok(Value::Str("ab".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::List(vec![Value::Int(1)]),
            &Value::Bool(true)
        ),
        Ok(Value::List(vec![Value::Int(1)]))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Bool(true),
            &Value::Str("ab".to_owned())
        ),
        Ok(Value::Str("ab".to_owned()))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Multiply,
            &Value::Tuple(vec![Value::Int(1)]),
            &Value::Bool(false)
        ),
        Ok(Value::Tuple(Vec::new()))
    );
}

#[test]
fn a_dict_updates_from_pair_iterables() {
    let mut first = IndexMap::new();
    first.insert("b".to_owned(), Value::Int(1));
    let pairs = Value::List(vec![Value::Tuple(vec![
        Value::Str("a".to_owned()),
        Value::Int(2),
    ])]);
    let mut merged = first.clone();
    merged.insert("a".to_owned(), Value::Int(2));
    assert_eq!(
        inplacevar(InplaceOp::BitOr, &Value::Dict(first), &pairs),
        Ok(Value::Dict(merged))
    );
}

#[rstest]
#[case::huge("%9000000000000000000s")]
#[case::usize_max("%18446744073709551615s")]
#[case::gigabytes("%3000000000s")]
fn a_format_width_past_the_cap_is_refused_before_allocating(#[case] template: &str) {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str(template.to_owned()),
            &Value::Str("a".to_owned())
        ),
        Err(InplaceError::RepetitionTooLong)
    );
}

#[test]
fn a_format_width_too_big_for_usize_overflows() {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%99999999999999999999999d".to_owned()),
            &Value::Int(1)
        ),
        Err(InplaceError::Overflow { symbol: "%=" })
    );
}

#[rstest]
#[case::double_long("%lld", Value::Int(1))]
#[case::double_short("%hhd", Value::Int(1))]
#[case::mixed_modifiers("%lhd", Value::Int(1))]
#[case::zero_flag("%05d", Value::Int(1))]
#[case::zero_flag_hex("%03x", Value::Int(1))]
#[case::star_width("%*d", Value::Tuple(vec![Value::Int(1), Value::Int(2)]))]
fn percent_flags_the_formatter_does_not_build_are_unsupported(
    #[case] template: &str,
    #[case] argument: Value,
) {
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str(template.to_owned()),
            &argument
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[rstest]
#[case::percent_s(b"%s")]
#[case::non_utf8(b"\xff%d")]
#[case::double_long(b"%lld")]
fn bytes_percent_s_and_non_utf8_templates_are_unsupported(#[case] template: &[u8]) {
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Bytes(template.to_vec()),
            &Value::Int(1)
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[rstest]
#[case::long("%ld")]
#[case::short("%hd")]
#[case::long_double("%Ld")]
fn length_modifiers_are_skipped(#[case] template: &str) {
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str(template.to_owned()),
            &Value::Int(1)
        ),
        Ok(Value::Str("1".to_owned()))
    );
}

#[rstest]
#[case::list_with_text(Value::Str("plain".to_owned()), Value::List(vec![Value::Int(1)]))]
#[case::bytes_with_text(Value::Str("plain".to_owned()), Value::Bytes(b"x".to_vec()))]
#[case::list_with_bytes(Value::Bytes(b"plain".to_vec()), Value::List(vec![Value::Int(1)]))]
fn a_list_or_bytes_operand_may_go_unused(#[case] template: Value, #[case] operand: Value) {
    assert_eq!(
        inplacevar(InplaceOp::Modulo, &template, &operand),
        Ok(template)
    );
}

#[test]
fn a_bytes_operand_may_not_go_unused_by_a_bytes_template() {
    assert!(matches!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Bytes(b"plain".to_vec()),
            &Value::Bytes(b"x".to_vec())
        ),
        Err(InplaceError::UnsupportedOperand { .. })
    ));
}

#[test]
fn a_formatted_result_past_the_cap_is_refused_when_each_piece_is_under_it() {
    let mapping = Value::Dict(IndexMap::from([(
        "a".to_owned(),
        Value::Str("x".repeat(600_000)),
    )]));
    assert_eq!(
        inplacevar(InplaceOp::Modulo, &Value::Str("%(a)s".repeat(2)), &mapping),
        Err(InplaceError::RepetitionTooLong)
    );
    assert_eq!(
        inplacevar(InplaceOp::Modulo, &Value::Str("%(a)s".to_owned()), &mapping),
        Ok(Value::Str("x".repeat(600_000)))
    );
}

#[test]
fn padding_is_capped_across_all_conversions_of_a_template() {
    let mapping = Value::Dict(IndexMap::from([(
        "a".to_owned(),
        Value::Str("x".to_owned()),
    )]));
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%(a)600000s".repeat(2)),
            &mapping
        ),
        Err(InplaceError::RepetitionTooLong)
    );
    assert_eq!(
        inplacevar(
            InplaceOp::Modulo,
            &Value::Str("%(a)3s%(a)3s".to_owned()),
            &mapping
        ),
        Ok(Value::Str("  x  x".to_owned()))
    );
}

#[test]
fn a_true_division_rounds_to_nearest_even_and_keeps_the_sign_of_zero() {
    assert_eq!(
        inplacevar(
            InplaceOp::TrueDivide,
            &Value::Int(3_291_455_435_069_936_578),
            &Value::Int(2_097_532_002_050_067_875)
        ),
        Ok(Value::Float(f64::from_bits(0x3ff9_1b75_9132_d8fb)))
    );
    assert_eq!(
        inplacevar(
            InplaceOp::TrueDivide,
            &Value::Int(0x0020_0000_0000_0001),
            &Value::Int(0x0020_0000_0000_0000)
        ),
        Ok(Value::Float(1.0))
    );
    let Ok(Value::Float(zero)) = inplacevar(InplaceOp::TrueDivide, &Value::Int(0), &Value::Int(-1))
    else {
        panic!("0 / -1 is a float");
    };
    assert!(zero == 0.0_f64 && zero.is_sign_negative());
}

#[test]
fn a_negative_power_rounds_a_large_base_to_a_double_first() {
    assert_eq!(
        inplacevar(
            InplaceOp::Power,
            &Value::Int(0x0020_0000_0000_0001),
            &Value::Int(-1)
        ),
        Ok(Value::Float(1.110_223_024_625_156_5e-16))
    );
}

#[rstest]
#[case::max(i64::MAX, 0x3c00_0000_0000_0000)]
#[case::min(i64::MIN, 0xbc00_0000_0000_0000)]
#[case::just_under_max(9_223_372_036_854_775_296, 0x3c00_0000_0000_0000)]
fn a_negative_power_of_a_base_that_rounds_to_two_to_the_63_still_divides(
    #[case] base: i64,
    #[case] bits: u64,
) {
    assert_eq!(
        inplacevar(InplaceOp::Power, &Value::Int(base), &Value::Int(-1)),
        Ok(Value::Float(f64::from_bits(bits)))
    );
}

#[rstest]
#[case::even_exponent(-2, -2, 0.25_f64)]
#[case::odd_exponent(-2, -3, -0.125_f64)]
fn a_negative_base_takes_its_sign_from_the_exponent_parity(
    #[case] base: i64,
    #[case] exponent: i64,
    #[case] expected: f64,
) {
    assert_eq!(
        inplacevar(InplaceOp::Power, &Value::Int(base), &Value::Int(exponent)),
        Ok(Value::Float(expected))
    );
}
