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
