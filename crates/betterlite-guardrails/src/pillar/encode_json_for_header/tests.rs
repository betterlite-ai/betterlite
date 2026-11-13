#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::json_header::encode_json_for_header;

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

#[rstest]
#[case::sorted_object(
    object(&[("a", GuardrailValue::Text("x".to_owned())), ("b", GuardrailValue::SignedInteger(1))]),
    "%7B%22a%22%3A%22x%22%2C%22b%22%3A1%7D"
)]
#[case::empty_object(object(&[]), "%7B%7D")]
#[case::list_with_scalars(
    GuardrailValue::Array(vec![
        GuardrailValue::UnsignedInteger(1),
        GuardrailValue::Boolean(true),
        GuardrailValue::Null,
        GuardrailValue::Text("x".to_owned()),
    ]),
    "%5B1%2Ctrue%2Cnull%2C%22x%22%5D"
)]
#[case::slash_and_spaces_are_quoted(
    object(&[("v", GuardrailValue::Text("a/b c:d,e".to_owned()))]),
    "%7B%22v%22%3A%22a%2Fb%20c%3Ad%2Ce%22%7D"
)]
#[case::escape_sequences_stay_escaped(
    object(&[(
        "v",
        GuardrailValue::Text("a\"f\\g\nh\t\u{1}".to_owned()),
    )]),
    "%7B%22v%22%3A%22a%5C%22f%5C%5Cg%5Cnh%5Ct%5Cu0001%22%7D"
)]
fn encodes_like_json_dumps_without_ascii_escapes(
    #[case] value: GuardrailValue,
    #[case] expected: &str,
) {
    assert_eq!(encode_json_for_header(&value), expected);
}

#[test]
fn non_ascii_text_is_utf8_percent_encoded_rather_than_escaped() {
    let value = object(&[("text", GuardrailValue::Text("héllo wörld".to_owned()))]);
    assert_eq!(
        encode_json_for_header(&value),
        "%7B%22text%22%3A%22h%C3%A9llo%20w%C3%B6rld%22%7D"
    );
}

#[test]
fn tilde_and_unreserved_punctuation_are_kept() {
    let value = GuardrailValue::Text("a-b_c.d~e".to_owned());
    assert_eq!(encode_json_for_header(&value), "%22a-b_c.d~e%22");
}

#[test]
fn percent_signs_are_encoded_again() {
    let value = GuardrailValue::Text("100%".to_owned());
    assert_eq!(encode_json_for_header(&value), "%22100%25%22");
}

#[test]
fn decimal_numbers_keep_their_shortest_form() {
    let value = object(&[("score", GuardrailValue::Decimal(1.5_f64))]);
    assert_eq!(encode_json_for_header(&value), "%7B%22score%22%3A1.5%7D");
}
