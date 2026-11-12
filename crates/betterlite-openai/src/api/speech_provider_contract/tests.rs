#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use super::{SpeechVoice, TextToSpeechRequestData};
use crate::openai_base_config::Object;

fn record(text: &str) -> TextToSpeechRequestData {
    serde_json::from_str(text).unwrap()
}

#[test]
fn empty_record_has_no_keys_and_no_default_body() {
    let parsed = record("{}");
    assert_eq!(parsed, TextToSpeechRequestData::default());
    assert_eq!(parsed.dict_body, None);
    assert_eq!(parsed.ssml_body, None);
    assert_eq!(parsed.headers, None);
    assert_eq!(serde_json::to_string(&parsed).unwrap(), "{}");
}

#[test]
fn empty_dict_body_stays_distinct_from_an_absent_one() {
    let parsed = record(r#"{"dict_body":{}}"#);
    assert_eq!(parsed.dict_body, Some(Object::new()));
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        r#"{"dict_body":{}}"#
    );
}

#[test]
fn ssml_body_is_kept_as_supplied() {
    let parsed = record(r#"{"ssml_body":"<speak>hello</speak>"}"#);
    assert_eq!(parsed.ssml_body.as_deref(), Some("<speak>hello</speak>"));
    assert_eq!(parsed.dict_body, None);
    assert_eq!(parsed.headers, None);
}

#[test]
fn headers_are_kept_in_supplied_order() {
    let parsed = record(r#"{"headers":{"z":"1","a":"2"}}"#);
    let expected = IndexMap::from([
        ("z".to_owned(), "1".to_owned()),
        ("a".to_owned(), "2".to_owned()),
    ]);
    assert_eq!(parsed.headers, Some(expected));
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        r#"{"headers":{"z":"1","a":"2"}}"#
    );
}

#[test]
fn dict_and_ssml_bodies_may_be_combined() {
    let text = r#"{"dict_body":{"input":"hi"},"ssml_body":"<speak/>","headers":{"x":"y"}}"#;
    let parsed = record(text);
    let spoken = parsed.dict_body.as_ref().and_then(|body| body.get("input"));
    assert_eq!(spoken.and_then(|value| value.as_str()), Some("hi"));
    assert_eq!(parsed.ssml_body.as_deref(), Some("<speak/>"));
    assert_eq!(serde_json::to_string(&parsed).unwrap(), text);
}

#[rstest]
#[case::name(r#""alloy""#, SpeechVoice::Name("alloy".to_owned()))]
#[case::empty_name(r#""""#, SpeechVoice::Name(String::new()))]
#[case::options(
    r#"{"id":"v"}"#,
    SpeechVoice::Options(serde_json::from_str(r#"{"id":"v"}"#).unwrap())
)]
fn voice_reads_a_string_or_an_object(#[case] text: &str, #[case] expected: SpeechVoice) {
    let parsed: SpeechVoice = serde_json::from_str(text).unwrap();
    assert_eq!(parsed, expected);
    assert_eq!(serde_json::to_string(&parsed).unwrap(), text);
}
