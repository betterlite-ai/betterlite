use http::StatusCode;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::gigachat_tokens::error::GigachatAuthError;
use crate::gigachat_tokens::reply::{MAX_REPLY_BYTES, decode_token_reply};

#[test]
fn reply_decoding_reads_tok_and_exp() {
    let reply =
        decode_token_reply(br#"{"tok":"token-a","exp":1735689600000}"#).expect("the reply decodes");

    assert_eq!(reply.token.expose_secret(), "token-a");
    assert_eq!(reply.expires_at, Some(1_735_689_600_000));
}

#[test]
fn reply_decoding_reads_access_token_and_expires_at() {
    let reply = decode_token_reply(br#"{"access_token":"token-b","expires_at":42}"#)
        .expect("the reply decodes");

    assert_eq!(reply.token.expose_secret(), "token-b");
    assert_eq!(reply.expires_at, Some(42));
}

#[test]
fn reply_decoding_prefers_tok_over_access_token() {
    let reply =
        decode_token_reply(br#"{"tok":"token-a","access_token":"token-b","exp":7,"expires_at":9}"#)
            .expect("the reply decodes");

    assert_eq!(reply.token.expose_secret(), "token-a");
    assert_eq!(reply.expires_at, Some(7));
}

#[test]
fn reply_decoding_skips_an_empty_tok() {
    let reply =
        decode_token_reply(br#"{"tok":"","access_token":"token-b","exp":0,"expires_at":9}"#)
            .expect("the reply decodes");

    assert_eq!(reply.token.expose_secret(), "token-b");
    assert_eq!(reply.expires_at, Some(9));
}

#[test]
fn reply_decoding_skips_a_missing_exp() {
    let reply =
        decode_token_reply(br#"{"tok":"token-a","expires_at":42}"#).expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(42));
}

#[test]
fn reply_decoding_skips_a_null_exp() {
    let reply = decode_token_reply(br#"{"tok":"token-a","exp":null,"expires_at":42}"#)
        .expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(42));
}

#[test]
fn reply_decoding_skips_a_zero_exp() {
    let reply = decode_token_reply(br#"{"tok":"token-a","exp":0,"expires_at":42}"#)
        .expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(42));
}

#[test]
fn reply_decoding_skips_an_empty_exp_string() {
    let reply = decode_token_reply(br#"{"tok":"token-a","exp":"","expires_at":42}"#)
        .expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(42));
}

#[test]
fn reply_decoding_returns_no_expiry_when_both_keys_are_absent() {
    let reply = decode_token_reply(br#"{"tok":"token-a"}"#).expect("the reply decodes");

    assert_eq!(reply.expires_at, None);
}

#[test]
fn reply_decoding_returns_no_expiry_for_a_null_expires_at() {
    let reply =
        decode_token_reply(br#"{"tok":"token-a","expires_at":null}"#).expect("the reply decodes");

    assert_eq!(reply.expires_at, None);
}

#[test]
fn reply_decoding_reads_a_numeric_string_expiry() {
    let reply = decode_token_reply(br#"{"tok":"token-a","exp":" 1735689600000 "}"#)
        .expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(1_735_689_600_000));
}

#[test]
fn reply_decoding_reads_a_negative_expiry() {
    let reply = decode_token_reply(br#"{"tok":"token-a","exp":-5}"#).expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(-5));
}

#[test]
fn reply_decoding_reads_a_string_zero_expiry() {
    let reply = decode_token_reply(br#"{"tok":"token-a","exp":"0"}"#).expect("the reply decodes");

    assert_eq!(reply.expires_at, Some(0));
}

#[rstest]
#[case::not_json("not json")]
#[case::an_array("[]")]
#[case::a_four_element_array("[\"token-a\",null,null,null]")]
#[case::a_string("\"text\"")]
#[case::a_null("null")]
#[case::missing_token("{\"exp\":1}")]
#[case::a_numeric_tok("{\"tok\":1}")]
#[case::an_empty_tok("{\"tok\":\"\"}")]
#[case::an_empty_access_token("{\"access_token\":\"\"}")]
#[case::a_float_exp("{\"tok\":\"token-a\",\"exp\":1.5}")]
#[case::a_bool_exp("{\"tok\":\"token-a\",\"exp\":true}")]
#[case::an_array_exp("{\"tok\":\"token-a\",\"exp\":[1]}")]
#[case::an_object_exp("{\"tok\":\"token-a\",\"exp\":{}}")]
#[case::an_unparsable_exp("{\"tok\":\"token-a\",\"exp\":\"soon\"}")]
#[case::a_blank_exp_string("{\"tok\":\"token-a\",\"exp\":\"  \"}")]
#[case::an_exp_past_i64("{\"tok\":\"token-a\",\"exp\":18446744073709551615}")]
#[case::a_float_expires_at("{\"tok\":\"token-a\",\"expires_at\":1.5}")]
#[case::an_unparsable_expires_at("{\"tok\":\"token-a\",\"expires_at\":\"soon\"}")]
fn reply_decoding_rejects_an_unusable_reply(#[case] body: &str) {
    assert!(matches!(
        decode_token_reply(body.as_bytes()),
        Err(GigachatAuthError::InvalidReply)
    ));
}

#[test]
fn reply_decoding_accepts_leading_whitespace() {
    let reply =
        decode_token_reply(b" \n\t{\"tok\":\"token-a\"}").expect("leading whitespace is allowed");

    assert_eq!(reply.token.expose_secret(), "token-a");
}

#[test]
fn reply_decoding_accepts_a_body_of_two_kib() {
    let mut body = String::from("{\"tok\":\"token-a\"}");
    body.push_str(&" ".repeat(2048_usize.saturating_sub(body.len())));
    assert_eq!(body.len(), 2048);

    let reply = decode_token_reply(body.as_bytes()).expect("a two KiB body decodes");

    assert_eq!(reply.token.expose_secret(), "token-a");
}

#[test]
fn reply_decoding_accepts_a_body_of_exactly_64_kib() {
    let mut body = String::from("{\"tok\":\"token-a\"}");
    body.push_str(&" ".repeat(MAX_REPLY_BYTES.saturating_sub(body.len())));
    assert_eq!(body.len(), MAX_REPLY_BYTES);

    let reply = decode_token_reply(body.as_bytes()).expect("a body at the limit decodes");

    assert_eq!(reply.token.expose_secret(), "token-a");
}

#[test]
fn reply_decoding_rejects_a_body_over_64_kib() {
    let mut body = String::from("{\"tok\":\"token-a\"}");
    body.push_str(&" ".repeat(MAX_REPLY_BYTES.saturating_add(1).saturating_sub(body.len())));
    assert_eq!(body.len(), MAX_REPLY_BYTES.saturating_add(1));

    assert!(matches!(
        decode_token_reply(body.as_bytes()),
        Err(GigachatAuthError::InvalidReply)
    ));
}

#[test]
fn reply_decoding_errors_report_status_500() {
    let error = decode_token_reply(b"[]").expect_err("an array is not a token reply");

    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}
