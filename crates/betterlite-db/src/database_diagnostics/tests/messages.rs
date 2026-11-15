#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::database_diagnostics::message::{LogArguments, LogValue, MessageError, format_message};

fn text(value: &str) -> LogValue {
    LogValue::Text(value.to_owned())
}

#[rstest]
#[case::none("%s", LogValue::Null, "None")]
#[case::true_value("%s", LogValue::Bool(true), "True")]
#[case::false_value("%s", LogValue::Bool(false), "False")]
#[case::negative_integer("%s", LogValue::Integer(-5), "-5")]
#[case::whole_float("%s", LogValue::Float(1.0), "1.0")]
#[case::negative_whole_float("%s", LogValue::Float(-2.0), "-2.0")]
#[case::fractional_float("%s", LogValue::Float(1.5), "1.5")]
#[case::list("%s", LogValue::List(vec![text("a"), LogValue::Integer(1)]), "['a', 1]")]
#[case::object(
    "%s",
    LogValue::Object(BTreeMap::from([
        ("k".to_owned(), text("v")),
        ("n".to_owned(), LogValue::Null),
    ])),
    "{'k': 'v', 'n': None}"
)]
#[case::repr_backslash("%r", text("a\\b"), "'a\\\\b'")]
#[case::integer_from_float("%d", LogValue::Float(3.9), "3")]
#[case::negative_integer_from_float("%d", LogValue::Float(-3.9), "-3")]
#[case::integer_alias("%i", LogValue::Integer(7), "7")]
#[case::float_from_integer("%f", LogValue::Integer(2), "2.000000")]
#[case::float_six_places("%f", LogValue::Float(2.5), "2.500000")]
#[case::float_rounds("%f", LogValue::Float(0.123_456_789), "0.123457")]
#[case::float_tie_goes_even("%f", LogValue::Float(0.007_812_5), "0.007812")]
fn message_conversions_render_python_values(
    #[case] template: &str,
    #[case] value: LogValue,
    #[case] expected: &str,
) {
    let rendered = format_message(template, &LogArguments::Positional(vec![value])).unwrap();
    assert_eq!(rendered, expected);
}

#[rstest]
#[case::unused_argument("one", LogArguments::Positional(vec![LogValue::Integer(1)]), MessageError::UnusedArguments)]
#[case::missing_argument("%s %s", LogArguments::Positional(vec![LogValue::Integer(1)]), MessageError::MissingArgument)]
#[case::missing_name("%(other)s", LogArguments::Named(BTreeMap::from([("name".to_owned(), text("a"))])), MessageError::MissingArgument)]
#[case::incomplete_specifier("100%", LogArguments::Positional(vec![LogValue::Integer(1)]), MessageError::IncompleteSpecifier)]
#[case::incomplete_name("%(name", LogArguments::Named(BTreeMap::from([("name".to_owned(), text("a"))])), MessageError::IncompleteSpecifier)]
#[case::unsupported_specifier("%x", LogArguments::Positional(vec![LogValue::Integer(1)]), MessageError::UnsupportedSpecifier)]
#[case::text_as_integer("%d", LogArguments::Positional(vec![text("a")]), MessageError::UnsupportedValue)]
#[case::text_as_float("%f", LogArguments::Positional(vec![text("a")]), MessageError::UnsupportedValue)]
fn message_argument_errors(
    #[case] template: &str,
    #[case] arguments: LogArguments,
    #[case] expected: MessageError,
) {
    assert_eq!(format_message(template, &arguments), Err(expected));
}

#[rstest]
#[case::no_arguments(LogArguments::None)]
#[case::empty_positional(LogArguments::Positional(Vec::new()))]
#[case::empty_named(LogArguments::Named(BTreeMap::new()))]
fn empty_arguments_skip_formatting(#[case] arguments: LogArguments) {
    assert_eq!(format_message("100% %s", &arguments).unwrap(), "100% %s");
}
