use betterlite_openai::openai_base_config::Headers;
use rstest::rstest;

use super::super::decode::{SonioxReply, decode_soniox_reply};
use super::super::raise::{provider_message, raise_for_response};
use crate::api::api_boundaries::ApiLimits;

fn reply(status: i64, body: &str) -> SonioxReply {
    decode_soniox_reply(
        status,
        Headers::new(),
        body.as_bytes(),
        &ApiLimits::default(),
    )
    .expect("the reply decodes")
}

#[rstest]
#[case::ok(200)]
#[case::redirect(302)]
#[case::below_error(399)]
fn passes_a_successful_status(#[case] status: i64) {
    assert!(raise_for_response(&reply(status, "{}"), "poll transcription").is_ok());
}

#[rstest]
#[case::bad_request(400)]
#[case::server_error(500)]
fn raises_with_the_action_and_status(#[case] status: i64) {
    let error = raise_for_response(
        &reply(status, r#"{"error_message": "quota"}"#),
        "create transcription",
    )
    .expect_err("the status raises");

    assert_eq!(error.status_code, status);
    assert!(
        error.message.starts_with(&format!(
            "Soniox create transcription failed (HTTP {status}): "
        )),
        "{}",
        error.message
    );
    assert!(error.message.ends_with("quota"), "{}", error.message);
}

#[test]
fn keeps_the_reply_headers() {
    let headers = Headers::from([(String::from("X-Request-Id"), String::from("abc"))]);
    let reply = decode_soniox_reply(500, headers, br#"{"error": "boom"}"#, &ApiLimits::default())
        .expect("the reply decodes");

    let error = raise_for_response(&reply, "fetch transcript").expect_err("the status raises");

    assert_eq!(
        error.headers,
        Some(Headers::from([(
            String::from("X-Request-Id"),
            String::from("abc")
        )]))
    );
}

#[rstest]
#[case::error_message_first(r#"{"error_message": "first", "error": "second"}"#, "first")]
#[case::empty_error_message(r#"{"error_message": "", "error": "second"}"#, "second")]
#[case::null_error_message(r#"{"error_message": null, "error": "second"}"#, "second")]
#[case::zero_error_message(r#"{"error_message": 0, "error": "second"}"#, "second")]
#[case::false_error_message(r#"{"error_message": false, "error": "second"}"#, "second")]
#[case::empty_list(r#"{"error_message": [], "error": "second"}"#, "second")]
#[case::empty_object(r#"{"error_message": {}, "error": "second"}"#, "second")]
#[case::only_error(r#"{"error": "second"}"#, "second")]
#[case::number_error(r#"{"error_message": "", "error": 7}"#, "7")]
fn chooses_the_provider_message(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(provider_message(&reply(400, body)), expected);
}

#[rstest]
#[case::no_fields(r#"{"other": "x"}"#)]
#[case::both_falsy(r#"{"error_message": "", "error": 0}"#)]
#[case::both_null(r#"{"error_message": null, "error": null}"#)]
fn falls_back_to_the_raw_text(#[case] body: &str) {
    assert_eq!(provider_message(&reply(500, body)), body);
}

#[rstest]
#[case::json_string(r#""boom""#)]
#[case::list("[1, 2]")]
#[case::scalar("418")]
#[case::malformed("not json")]
fn falls_back_to_raw_text_for_a_non_object_body(#[case] body: &str) {
    assert_eq!(provider_message(&reply(500, body)), body);
}

#[rstest]
#[case::number(r#"{"error_message": 7}"#, "7")]
#[case::true_flag(r#"{"error_message": true}"#, "true")]
#[case::list(r#"{"error_message": ["a"]}"#, r#"["a"]"#)]
#[case::object(r#"{"error_message": {"detail": "x"}}"#, r#"{"detail":"x"}"#)]
fn renders_a_non_string_candidate_as_json(#[case] body: &str, #[case] expected: &str) {
    assert!(provider_message(&reply(500, body)).ends_with(expected));
}
