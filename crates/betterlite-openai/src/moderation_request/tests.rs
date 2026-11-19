#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{
    ImageInputType, ImagePart, ImageUrl, ModerationInput, ModerationInputPart, ModerationRequest,
    ModerationRequestError, TextInputType, TextPart,
};

const MODEL: &str = "omni-moderation-latest";

#[rstest]
#[case::string(r#"{"input":"hello"}"#, ModerationInput::Text(String::from("hello")))]
#[case::array(
    r#"{"input":["hello","world"]}"#,
    ModerationInput::Texts(vec![String::from("hello"), String::from("world")])
)]
#[case::multimodal(
    r#"{"input":[{"image_url":{"url":"https://example.test/a.png"},"type":"image_url"},{"text":"hello","type":"text"}]}"#,
    ModerationInput::MultiModal(vec![
        ModerationInputPart::Image(ImagePart {
            image_url: ImageUrl {
                url: String::from("https://example.test/a.png"),
            },
            kind: ImageInputType::ImageUrl,
        }),
        ModerationInputPart::Text(TextPart {
            text: String::from("hello"),
            kind: TextInputType::Text,
        }),
    ])
)]
fn moderations_moderation_tests_behavior(#[case] body: &str, #[case] expected: ModerationInput) {
    let request = ModerationRequest::parse(body.as_bytes()).expect("moderation request");
    assert_eq!(request.input, expected);
    assert_eq!(request.model, None);
    assert!(request.passthrough.is_empty());
}

#[rstest]
#[case::string(
    r#"{"input":"hello"}"#,
    r#"{"input":"hello","model":"omni-moderation-latest"}"#
)]
#[case::array(
    r#"{"input":["hello","world"]}"#,
    r#"{"input":["hello","world"],"model":"omni-moderation-latest"}"#
)]
#[case::multimodal(
    r#"{"input":[{"image_url":{"url":"https://example.test/a.png"},"type":"image_url"},{"text":"hello","type":"text"}]}"#,
    r#"{"input":[{"image_url":{"url":"https://example.test/a.png"},"type":"image_url"},{"text":"hello","type":"text"}],"model":"omni-moderation-latest"}"#
)]
fn the_provider_body_carries_the_input_then_the_resolved_model(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let request = ModerationRequest::parse(body.as_bytes()).expect("moderation request");
    assert_eq!(
        request.upstream_body(MODEL).expect("upstream body"),
        expected.as_bytes()
    );
}

#[test]
fn the_body_model_is_read_but_never_sent_as_the_caller_wrote_it() {
    let request =
        ModerationRequest::parse(br#"{"model":"mod-alias","input":"hello"}"#).expect("request");
    assert_eq!(request.model.as_deref(), Some("mod-alias"));
    assert_eq!(
        request.upstream_body(MODEL).expect("upstream body"),
        br#"{"input":"hello","model":"omni-moderation-latest"}"#
    );
}

#[rstest]
#[case::api_base(
    r#"{"input":"hello","api_base":"https://attacker.test/v1"}"#,
    "api_base"
)]
#[case::api_key(r#"{"input":"hello","api_key":"sk-attacker"}"#, "api_key")]
#[case::user(r#"{"input":"hello","user":"reader"}"#, "user")]
fn refuses_every_request_field_litellm_would_drop_or_honour(
    #[case] body: &str,
    #[case] name: &str,
) {
    let error = ModerationRequest::parse(body.as_bytes()).expect_err("unsupported field");
    assert!(
        matches!(error, ModerationRequestError::Unsupported { name: refused } if refused == name)
    );
}

#[rstest]
#[case::malformed("{")]
#[case::missing_input(r#"{"model":"omni-moderation-latest"}"#)]
#[case::null_input(r#"{"input":null}"#)]
#[case::mixed_array(r#"{"input":["hello",{"text":"hello","type":"text"}]}"#)]
#[case::text_part_without_type(r#"{"input":[{"text":"hello"}]}"#)]
#[case::image_part_without_type(
    r#"{"input":[{"image_url":{"url":"https://example.test/a.png"}}]}"#
)]
#[case::wrong_text_type(r#"{"input":[{"text":"hello","type":"image_url"}]}"#)]
#[case::unknown_part_field(r#"{"input":[{"text":"hello","type":"text","detail":"high"}]}"#)]
#[case::non_text_model(r#"{"model":7,"input":"hello"}"#)]
fn rejects_invalid_moderation_input(#[case] body: &str) {
    assert!(matches!(
        ModerationRequest::parse(body.as_bytes()),
        Err(ModerationRequestError::Body { .. })
    ));
}

#[rstest]
fn replays_the_moderation_request_corpus(
    #[files("fuzz/corpus/moderation_request/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    if let Ok(request) = ModerationRequest::parse(&bytes) {
        let _body = request.upstream_body(MODEL);
    }
}
