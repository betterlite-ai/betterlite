use betterlite_openai::openai_base_config::Headers;
use rstest::rstest;
use serde_json::json;

use super::super::decode::{SonioxReply, decode_soniox_reply};
use super::super::error::ReplyError;
use super::super::response::{transcript, transcription_meta};
use crate::api::api_boundaries::ApiLimits;

fn reply(body: &str) -> SonioxReply {
    decode_soniox_reply(200, Headers::new(), body.as_bytes(), &ApiLimits::default())
        .expect("the reply decodes")
}

fn limited(items: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 1024,
        max_items: items,
    }
}

#[test]
fn projects_the_status_fields() {
    let body = r#"{"status": "completed", "error_message": "none", "error_type": "provider", "audio_duration_ms": 1500, "extra": true}"#;

    let meta = transcription_meta(&reply(body)).expect("the meta projects");

    assert_eq!(meta.status.as_deref(), Some("completed"));
    assert_eq!(meta.error_message.as_deref(), Some("none"));
    assert_eq!(meta.error_type.as_deref(), Some("provider"));
    assert_eq!(meta.audio_duration_ms, Some(json!(1_500_i32)));
    assert_eq!(meta.raw.get("extra"), Some(&json!(true)));
}

#[rstest]
#[case::absent("{}")]
#[case::null(
    r#"{"status": null, "error_message": null, "error_type": null, "audio_duration_ms": null}"#
)]
fn leaves_absent_and_null_status_fields_empty(#[case] body: &str) {
    let meta = transcription_meta(&reply(body)).expect("the meta projects");

    assert_eq!(meta.status, None);
    assert_eq!(meta.error_message, None);
    assert_eq!(meta.error_type, None);
    assert_eq!(meta.audio_duration_ms, None);
}

#[rstest]
#[case::status(r#"{"status": 1}"#)]
#[case::error_message(r#"{"error_message": {"text": "x"}}"#)]
#[case::error_type(r#"{"error_type": ["x"]}"#)]
fn refuses_a_status_field_that_is_not_text(#[case] body: &str) {
    assert!(matches!(
        transcription_meta(&reply(body)),
        Err(ReplyError::Shape { .. })
    ));
}

#[test]
fn projects_a_transcript() {
    let body =
        r#"{"text": "hi", "tokens": [{"text": "hi", "speaker": 1, "language": "en"}], "extra": 2}"#;

    let decoded = transcript(&reply(body), &ApiLimits::default()).expect("the transcript projects");

    assert_eq!(decoded.text.as_deref(), Some("hi"));
    assert_eq!(decoded.tokens.len(), 1);
    assert_eq!(decoded.tokens[0].text_string(), "hi");
    assert_eq!(decoded.raw.get("extra"), Some(&json!(2_i32)));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"tokens": null}"#)]
#[case::empty(r#"{"tokens": []}"#)]
fn leaves_missing_tokens_empty(#[case] body: &str) {
    let decoded = transcript(&reply(body), &ApiLimits::default()).expect("the transcript projects");

    assert!(decoded.tokens.is_empty());
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"text": null}"#)]
fn leaves_missing_text_empty(#[case] body: &str) {
    let decoded = transcript(&reply(body), &ApiLimits::default()).expect("the transcript projects");

    assert_eq!(decoded.text, None);
}

#[rstest]
#[case::object(r#"{"tokens": {}}"#)]
#[case::text(r#"{"tokens": "none"}"#)]
#[case::number(r#"{"tokens": 1}"#)]
fn refuses_tokens_that_are_not_a_list(#[case] body: &str) {
    assert!(matches!(
        transcript(&reply(body), &ApiLimits::default()),
        Err(ReplyError::Shape { .. })
    ));
}

#[rstest]
#[case::bad_speaker(r#"{"tokens": [{"speaker": {"who": 1}}]}"#)]
#[case::scalar_entry(r#"{"tokens": [3]}"#)]
fn refuses_malformed_tokens(#[case] body: &str) {
    assert!(matches!(
        transcript(&reply(body), &ApiLimits::default()),
        Err(ReplyError::Field { .. })
    ));
}

#[test]
fn bounds_the_token_count_before_decoding() {
    let malformed = r#"{"tokens": [{"speaker": {"who": 1}}, {"speaker": {"who": 2}}, {"speaker": {"who": 3}}]}"#;
    let at_limit = r#"{"tokens": [{}, {}]}"#;

    let decoded = transcript(&reply(at_limit), &limited(2))
        .expect("the transcript at the token limit decodes");
    assert_eq!(decoded.tokens.len(), 2);
    assert!(matches!(
        transcript(&reply(malformed), &limited(2)),
        Err(ReplyError::TooManyTokens {
            observed: 3,
            limit: 2
        })
    ));
}

#[rstest]
#[case::text(r#"{"text": 1}"#)]
#[case::list(r#"{"text": ["x"]}"#)]
fn refuses_text_that_is_not_text(#[case] body: &str) {
    assert!(matches!(
        transcript(&reply(body), &ApiLimits::default()),
        Err(ReplyError::Shape { .. })
    ));
}

#[rstest]
#[case::list("[]")]
#[case::scalar("7")]
fn refuses_a_non_object_reply(#[case] body: &str) {
    let decoded = reply(body);

    assert!(matches!(
        transcription_meta(&decoded),
        Err(ReplyError::NotObject)
    ));
    assert!(matches!(
        transcript(&decoded, &ApiLimits::default()),
        Err(ReplyError::NotObject)
    ));
}
