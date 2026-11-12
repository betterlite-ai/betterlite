use rstest::rstest;

use super::support::json;
use crate::gemini_embedding_records::usage_metadata::Modality;
use crate::gemini_embedding_records::usage_metadata::ModalityCount;
use crate::gemini_embedding_records::usage_metadata::UsageMetadata;
use crate::gemini_embedding_records::usage_metadata::parse_usage_metadata;

fn parsed(source: &str) -> Option<UsageMetadata> {
    parse_usage_metadata(Some(&json(source)))
}

#[test]
fn an_empty_record_is_valid() {
    assert_eq!(
        parsed("{}"),
        Some(UsageMetadata {
            prompt_token_count: None,
            total_token_count: None,
            prompt_tokens_details: None,
        })
    );
}

#[rstest]
#[case::prompt("promptTokenCount")]
#[case::total("totalTokenCount")]
#[case::candidates("candidatesTokenCount")]
#[case::response("responseTokenCount")]
#[case::cached_content("cachedContentTokenCount")]
#[case::tool_use("toolUsePromptTokenCount")]
#[case::thoughts("thoughtsTokenCount")]
fn every_integer_field_is_validated(#[case] field: &str) {
    assert!(parsed(&format!("{{\"{field}\": 4}}")).is_some());
    assert!(parsed(&format!("{{\"{field}\": \"4\"}}")).is_none());
    assert!(parsed(&format!("{{\"{field}\": true}}")).is_none());
    assert!(parsed(&format!("{{\"{field}\": 4.5}}")).is_none());
}

#[rstest]
#[case::prompt("promptTokensDetails")]
#[case::tool_use("toolUsePromptTokensDetails")]
#[case::cache("cacheTokensDetails")]
#[case::response("responseTokensDetails")]
#[case::candidates("candidatesTokensDetails")]
fn every_detail_field_is_validated(#[case] field: &str) {
    assert!(parsed(&format!("{{\"{field}\": []}}")).is_some());
    assert!(parsed(&format!("{{\"{field}\": \"nope\"}}")).is_none());
    assert!(
        parsed(&format!(
            "{{\"{field}\": [{{\"modality\": \"TEXT\", \"tokenCount\": 1}}]}}"
        ))
        .is_some()
    );
    assert!(parsed(&format!("{{\"{field}\": [{{\"modality\": \"TEXT\"}}]}}")).is_none());
}

#[test]
fn null_takes_the_absent_value() {
    assert_eq!(
        parsed(r#"{"promptTokenCount": null, "promptTokensDetails": null}"#),
        Some(UsageMetadata {
            prompt_token_count: None,
            total_token_count: None,
            prompt_tokens_details: None,
        })
    );
}

#[rstest]
#[case::unknown_integer(r#"{"futureTokenCount": 3}"#)]
#[case::unknown_detail(r#"{"futureTokensDetails": "nope"}"#)]
fn unknown_fields_are_ignored(#[case] source: &str) {
    assert!(parsed(source).is_some());
}

#[test]
fn counts_and_details_are_read_from_the_record() {
    assert_eq!(
        parsed(
            r#"{"promptTokenCount": 7, "totalTokenCount": 9, "promptTokensDetails": [
                {"modality": "TEXT", "tokenCount": 2},
                {"modality": "AUDIO", "tokenCount": 3},
                {"modality": "IMAGE", "tokenCount": 4},
                {"modality": "VIDEO", "tokenCount": 5}
            ]}"#
        ),
        Some(UsageMetadata {
            prompt_token_count: Some(7),
            total_token_count: Some(9),
            prompt_tokens_details: Some(vec![
                ModalityCount {
                    modality: Modality::Text,
                    token_count: 2,
                },
                ModalityCount {
                    modality: Modality::Audio,
                    token_count: 3,
                },
                ModalityCount {
                    modality: Modality::Image,
                    token_count: 4,
                },
                ModalityCount {
                    modality: Modality::Video,
                    token_count: 5,
                },
            ]),
        })
    );
}

#[rstest]
#[case::lowercase(r#"{"promptTokensDetails": [{"modality": "text", "tokenCount": 1}]}"#)]
#[case::unknown(r#"{"promptTokensDetails": [{"modality": "DOCUMENT", "tokenCount": 1}]}"#)]
#[case::missing_count(r#"{"promptTokensDetails": [{"modality": "TEXT"}]}"#)]
#[case::null_count(r#"{"promptTokensDetails": [{"modality": "TEXT", "tokenCount": null}]}"#)]
#[case::count_string(r#"{"promptTokensDetails": [{"modality": "TEXT", "tokenCount": "1"}]}"#)]
fn malformed_details_invalidate_the_record(#[case] source: &str) {
    assert!(parsed(source).is_none());
}

#[test]
fn a_detail_entry_ignores_unknown_keys() {
    assert_eq!(
        parsed(r#"{"promptTokensDetails": [{"modality": "TEXT", "tokenCount": 1, "extra": 2}]}"#)
            .and_then(|metadata| metadata.prompt_tokens_details)
            .and_then(|details| details.first().copied()),
        Some(ModalityCount {
            modality: Modality::Text,
            token_count: 1,
        })
    );
}

#[rstest]
#[case::not_an_object("[]")]
#[case::text(r#""usage""#)]
#[case::number("5")]
#[case::boolean("true")]
fn a_value_outside_the_record_is_invalid(#[case] source: &str) {
    assert!(parsed(source).is_none());
}

#[test]
fn an_absent_record_is_invalid() {
    assert!(parse_usage_metadata(None).is_none());
}

#[rstest]
#[case::negative("-3", true, Some(-3))]
#[case::past_i64("9223372036854775808", false, None)]
fn integers_keep_the_record_range(
    #[case] value: &str,
    #[case] valid: bool,
    #[case] expected: Option<i64>,
) {
    let metadata = parsed(&format!("{{\"promptTokenCount\": {value}}}"));
    assert_eq!(metadata.is_some(), valid);
    if let Some(metadata) = metadata {
        assert_eq!(metadata.prompt_token_count, expected);
    }
}
