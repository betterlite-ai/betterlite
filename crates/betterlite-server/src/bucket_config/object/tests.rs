#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_config::sections::settings::settings_rules::JsonValue as Value;
use rstest::rstest;

use super::parse_bucket_object;

fn value(json: &str) -> Value {
    serde_json::from_str(json).unwrap()
}

#[test]
fn parses_a_mapping_document() {
    let parsed = parse_bucket_object("config.yaml", b"model_list:\n  - model_name: gpt-4\n");
    assert_eq!(
        parsed,
        Some(value(r#"{"model_list": [{"model_name": "gpt-4"}]}"#))
    );
}

#[test]
fn parses_a_nested_mapping_with_includes() {
    let parsed = parse_bucket_object(
        "team/config.yaml",
        b"include:\n  - shared.yaml\nlitellm_settings:\n  drop_params: true\n",
    );
    assert_eq!(
        parsed,
        Some(value(
            r#"{"include": ["shared.yaml"], "litellm_settings": {"drop_params": true}}"#
        ))
    );
}

#[test]
fn an_empty_document_reads_as_null() {
    assert_eq!(parse_bucket_object("config.yaml", b""), Some(Value::Null));
}

#[test]
fn an_explicit_null_document_reads_as_null() {
    assert_eq!(
        parse_bucket_object("config.yaml", b"null\n"),
        Some(Value::Null)
    );
}

#[test]
fn invalid_yaml_reads_as_missing() {
    assert_eq!(parse_bucket_object("config.yaml", b"{unclosed: ["), None);
}

#[test]
fn a_multi_document_stream_reads_as_missing() {
    assert_eq!(
        parse_bucket_object("config.yaml", b"a: 1\n---\nb: 2\n"),
        None
    );
}

#[test]
fn bytes_that_are_not_utf8_read_as_missing() {
    assert_eq!(
        parse_bucket_object("config.yaml", &[0xff, 0xfe, 0x00]),
        None
    );
}

#[test]
fn a_sequence_document_is_kept_for_the_caller_to_reject() {
    assert_eq!(
        parse_bucket_object("config.yaml", b"- one\n- two\n"),
        Some(value(r#"["one", "two"]"#))
    );
}

#[test]
fn a_scalar_document_is_kept_for_the_caller_to_reject() {
    assert_eq!(
        parse_bucket_object("config.yaml", b"just a string\n"),
        Some(value(r#""just a string""#))
    );
}

#[rstest]
#[case::mapping("config_mapping.yaml")]
#[case::includes("config_with_includes.yaml")]
#[case::empty("empty_document.yaml")]
#[case::explicit_null("null_document.yaml")]
#[case::invalid("invalid_flow.yaml")]
#[case::scalar("scalar_document.yaml")]
#[case::sequence("sequence_document.yaml")]
#[case::comments("comments_only.yaml")]
#[case::invalid_utf8("invalid_utf8.bin")]
fn replays_the_bucket_config_yaml_corpus(#[case] corpus_name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bucket_config_yaml");
    let bytes = fs::read(root.join(corpus_name)).unwrap();
    drop(parse_bucket_object("corpus/config.yaml", &bytes));
}
