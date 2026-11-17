use rstest::rstest;

use crate::sso_mappings::literal::hashable::has_unhashable_member;
use crate::sso_mappings::literal::{Literal, LiteralError, MAX_LITERAL_DEPTH, parse_literal};

fn text(value: &str) -> Literal {
    Literal::Str(value.to_owned())
}

#[rstest]
#[case::text("'text'", text("text"))]
#[case::double_quoted("\"text\"", text("text"))]
#[case::prefixed("u'text'", text("text"))]
#[case::raw_keeps_the_backslash("r'a\\nb'", text("a\\nb"))]
#[case::raw_escape_hides_the_quote("r'\\''", text("\\'"))]
#[case::triple_quoted("'''a'''", text("a"))]
#[case::implicit_concatenation("'a' 'b'", text("ab"))]
#[case::bytes("b'bytes'", Literal::Bytes)]
#[case::raw_bytes("rb'\\x'", Literal::Bytes)]
#[case::true_value("True", Literal::Bool(true))]
#[case::false_value("False", Literal::Bool(false))]
#[case::none("None", Literal::None)]
#[case::ellipsis("...", Literal::Ellipsis)]
#[case::hex_escape("'\\x41'", text("A"))]
#[case::short_hex_escape("'\\u00e9'", text("\u{e9}"))]
#[case::long_hex_escape("'\\U0001f600'", text("\u{1f600}"))]
#[case::octal_escape("'\\101'", text("A"))]
#[case::octal_escape_takes_three_digits("'\\1011'", text("A1"))]
#[case::null_escape("'\\0'", text("\0"))]
#[case::escaped_newline("'a\\nb'", text("a\nb"))]
#[case::raw_line_ending("r\"a\\\nb\"", text("a\\\nb"))]
#[case::raw_carriage_return("r\"a\\\r\nb\"", text("a\\\nb"))]
#[case::continued_crlf_line("'a\\\r\nb'", text("ab"))]
#[case::triple_line_endings("'''a\r\nb'''", text("a\nb"))]
#[case::non_ascii_text("'é'", text("é"))]
#[case::unknown_escape("'\\q'", text("\\q"))]
#[case::integer("42", Literal::Number)]
#[case::negative("-42", Literal::Number)]
#[case::leading_dot(".5", Literal::Number)]
#[case::imaginary("+1.5e-3j", Literal::Number)]
#[case::float_with_imaginary_suffix("5.j", Literal::Number)]
#[case::hexadecimal("0x_1f", Literal::Number)]
#[case::octal("0o17", Literal::Number)]
#[case::binary("0b1010", Literal::Number)]
#[case::leading_zeros("00", Literal::Number)]
#[case::underscores("1_000", Literal::Number)]
#[case::empty_list("[]", Literal::List(Vec::new()))]
#[case::list(
    "[1, 'a', None]",
    Literal::List(vec![Literal::Number, text("a"), Literal::None])
)]
#[case::empty_dict("{}", Literal::Dict(Vec::new()))]
#[case::dictionary("{'a': 1}", Literal::Dict(vec![(text("a"), Literal::Number)]))]
#[case::duplicate_key(
    "{'a': 'x', 'a': 'y'}",
    Literal::Dict(vec![(text("a"), text("y"))])
)]
#[case::set_of_numbers("{1}", Literal::Set(vec![Literal::Number]))]
#[case::set_with_several_elements(
    "{1, 2}",
    Literal::Set(vec![Literal::Number, Literal::Number])
)]
#[case::empty_set("set()", Literal::Set(Vec::new()))]
#[case::empty_tuple("()", Literal::Tuple(Vec::new()))]
#[case::tuple("(1, 2)", Literal::Tuple(vec![Literal::Number, Literal::Number]))]
#[case::parenthesized("(1)", Literal::Number)]
#[case::trailing_comment("{'a': 1} # note", Literal::Dict(vec![(text("a"), Literal::Number)]))]
#[case::continued_line(
    "{'a': \\\n 1}",
    Literal::Dict(vec![(text("a"), Literal::Number)])
)]
fn parses_an_accepted_literal(#[case] input: &str, #[case] expected: Literal) {
    assert_eq!(parse_literal(input.as_bytes()), Ok(expected));
}

#[rstest]
#[case::bare_name("x", LiteralError::NotLiteral)]
#[case::bare_set_name("set", LiteralError::NotLiteral)]
#[case::word_with_underscore("True_x", LiteralError::NotLiteral)]
#[case::f_string("f'x'", LiteralError::NotLiteral)]
#[case::star_unpacking("[*[]]", LiteralError::NotLiteral)]
#[case::dict_unpacking("{**{}}", LiteralError::NotLiteral)]
#[case::binary_operator("1 + 2", LiteralError::Syntax)]
#[case::missing_colon("{'a' 1}", LiteralError::Syntax)]
#[case::mixed_concatenation("b'a' 'b'", LiteralError::Syntax)]
#[case::unterminated_text("'unterminated", LiteralError::Syntax)]
#[case::newline_in_a_short_string("'a\nb'", LiteralError::Syntax)]
#[case::expression_on_the_next_line("'a'\n'b'", LiteralError::Syntax)]
#[case::unclosed_dict("{'a': 1", LiteralError::Syntax)]
#[case::leading_zero("01", LiteralError::Syntax)]
#[case::leading_zero_underscore("0_1", LiteralError::Syntax)]
#[case::double_underscore("1__0", LiteralError::Syntax)]
#[case::trailing_underscore("1_", LiteralError::Syntax)]
#[case::exponent_underscore("1e_5", LiteralError::Syntax)]
#[case::fraction_underscore("1._5", LiteralError::Syntax)]
#[case::empty_exponent("1e", LiteralError::Syntax)]
#[case::empty_radix("0x", LiteralError::Syntax)]
#[case::bad_octal_digit("0o8", LiteralError::Syntax)]
#[case::bad_binary_digit("0b2", LiteralError::Syntax)]
#[case::imaginary_after_radix("0x1j", LiteralError::Syntax)]
#[case::empty("", LiteralError::Syntax)]
#[case::blank("   ", LiteralError::Syntax)]
#[case::named_escape("'\\N{SNOWMAN}'", LiteralError::NamedEscape)]
#[case::lone_surrogate("'\\ud800'", LiteralError::Syntax)]
fn refuses_everything_else(#[case] input: &str, #[case] expected: LiteralError) {
    assert_eq!(parse_literal(input.as_bytes()), Err(expected));
}

#[test]
fn refuses_bytes_that_are_not_utf8() {
    assert_eq!(parse_literal(&[0xff, 0xfe]), Err(LiteralError::NotUtf8));
}

fn nested(depth: usize) -> Vec<u8> {
    let mut literal = "[".repeat(depth);
    literal.push_str(&"]".repeat(depth));
    literal.into_bytes()
}

#[test]
fn accepts_nesting_up_to_the_limit() {
    assert!(parse_literal(&nested(MAX_LITERAL_DEPTH)).is_ok());
}

#[test]
fn refuses_nesting_past_the_limit() {
    assert_eq!(
        parse_literal(&nested(MAX_LITERAL_DEPTH + 1)),
        Err(LiteralError::TooDeep {
            limit: MAX_LITERAL_DEPTH
        })
    );
}

#[rstest]
#[case::none(Literal::None, false)]
#[case::text(text("a"), false)]
#[case::numbers(Literal::Number, false)]
#[case::list_of_text(Literal::List(vec![text("a")]), false)]
#[case::tuple_of_numbers(Literal::Tuple(vec![Literal::Number]), false)]
#[case::set_of_text(Literal::Set(vec![text("a")]), false)]
#[case::dict_with_text_keys(Literal::Dict(vec![(text("a"), Literal::Number)]), false)]
#[case::list_key(Literal::Dict(vec![(Literal::List(Vec::new()), Literal::Number)]), true)]
#[case::tuple_key_with_a_list(
    Literal::Dict(vec![(
        Literal::Tuple(vec![Literal::List(Vec::new())]),
        Literal::Number
    )]),
    true
)]
#[case::list_element(Literal::Set(vec![Literal::List(Vec::new())]), true)]
#[case::unhashable_value(
    Literal::Dict(vec![(text("a"), Literal::Set(vec![Literal::List(Vec::new())]))]),
    true
)]
fn reports_unhashable_members(#[case] value: Literal, #[case] expected: bool) {
    assert_eq!(has_unhashable_member(&value), expected);
}
