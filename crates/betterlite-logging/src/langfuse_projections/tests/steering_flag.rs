#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::steering::{as_steering_flag, as_steering_key_sequence, trace_public_flag};
use super::support::value;
use super::text::optional_str;

#[rstest]
#[case::true_word("true", true)]
#[case::false_word(" FALSE ", false)]
#[case::unrecognized_yes("yes", true)]
#[case::unrecognized_zero("0", true)]
#[case::empty_string("", false)]
fn steering_flag_reads_a_string(#[case] raw: &str, #[case] expected: bool) {
    assert_eq!(as_steering_flag(&Json::Text(raw.to_owned())), expected);
}

#[rstest]
#[case::true_flag("true", true)]
#[case::false_flag("false", false)]
#[case::zero_number("0", false)]
#[case::non_zero_number("2", true)]
#[case::empty_list("[]", false)]
#[case::non_empty_list("[null]", true)]
#[case::empty_object("{}", false)]
#[case::null("null", false)]
fn steering_flag_reads_a_non_string(#[case] raw: &str, #[case] expected: bool) {
    assert_eq!(as_steering_flag(&value(raw)), expected);
}

#[test]
fn steering_key_splits_a_comma_separated_string() {
    assert_eq!(
        as_steering_key_sequence(Some(&value(r#""name,tags, release""#))),
        vec!["name".to_owned(), "tags".to_owned(), "release".to_owned()]
    );
}

#[test]
fn steering_key_drops_blank_entries() {
    assert_eq!(
        as_steering_key_sequence(Some(&value(r#"" a, , b ,""#))),
        vec!["a".to_owned(), "b".to_owned()]
    );
    assert_eq!(
        as_steering_key_sequence(Some(&Json::Text(String::new()))),
        Vec::<String>::new()
    );
    assert_eq!(as_steering_key_sequence(None), Vec::<String>::new());
}

#[test]
fn steering_key_accepts_a_list() {
    assert_eq!(
        as_steering_key_sequence(Some(&value(r#"["a", "b"]"#))),
        vec!["a".to_owned(), "b".to_owned()]
    );
    assert_eq!(
        as_steering_key_sequence(Some(&value(r#"["a", 3]"#))),
        vec!["a".to_owned(), "3".to_owned()]
    );
}

#[test]
fn steering_flag_trace_public_flag_keeps_absence_apart_from_false() {
    assert_eq!(trace_public_flag(None), None);
    assert_eq!(trace_public_flag(Some(&Json::Null)), None);
    assert_eq!(trace_public_flag(Some(&Json::Bool(false))), Some(false));
    assert_eq!(
        trace_public_flag(Some(&Json::Text("true".to_owned()))),
        Some(true)
    );
    assert_eq!(
        trace_public_flag(Some(&Json::Text("no".to_owned()))),
        Some(true)
    );
}

#[test]
fn steering_flag_optional_str_keeps_text_and_numbers() {
    assert_eq!(
        optional_str(Some(&Json::Text("v1".to_owned()))),
        Some("v1".to_owned())
    );
    assert_eq!(optional_str(Some(&value("2"))), Some("2".to_owned()));
    assert_eq!(optional_str(Some(&Json::Null)), None);
    assert_eq!(optional_str(None), None);
}
