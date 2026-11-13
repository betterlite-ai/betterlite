use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::gdc_auth::token::{MAX_REPLY_BYTES, decode_gdc_token_reply};

#[test]
fn an_integer_expiry_is_read_as_seconds() {
    let reply =
        decode_gdc_token_reply(b"{\"access_token\":\"token\",\"expires_in\":3600}").unwrap();
    assert_eq!(reply.token.expose_secret(), "token");
    assert_eq!(reply.expires_in, Some(3600));
}

#[test]
fn a_string_expiry_is_read_as_seconds() {
    let reply =
        decode_gdc_token_reply(b"{\"access_token\":\"token\",\"expires_in\":\"3600\"}").unwrap();
    assert_eq!(reply.expires_in, Some(3600));
    let padded =
        decode_gdc_token_reply(b"{\"access_token\":\"token\",\"expires_in\":\" 3600 \"}").unwrap();
    assert_eq!(padded.expires_in, Some(3600));
    let negative =
        decode_gdc_token_reply(b"{\"access_token\":\"token\",\"expires_in\":\"-5\"}").unwrap();
    assert_eq!(negative.expires_in, Some(-5));
}

#[test]
fn an_absent_or_null_expiry_means_no_expiry() {
    let absent = decode_gdc_token_reply(b"{\"access_token\":\"token\"}").unwrap();
    assert_eq!(absent.expires_in, None);
    let null = decode_gdc_token_reply(b"{\"access_token\":\"token\",\"expires_in\":null}").unwrap();
    assert_eq!(null.expires_in, None);
}

#[test]
fn a_zero_expiry_is_kept_as_expired() {
    let reply = decode_gdc_token_reply(b"{\"access_token\":\"token\",\"expires_in\":0}").unwrap();
    assert_eq!(reply.expires_in, Some(0));
}

#[test]
fn an_empty_access_token_is_kept() {
    let reply = decode_gdc_token_reply(b"{\"access_token\":\"\"}").unwrap();
    assert_eq!(reply.token.expose_secret(), "");
}

#[rstest]
#[case::missing_token(b"{\"expires_in\":60}".as_slice())]
#[case::null_token(b"{\"access_token\":null}".as_slice())]
#[case::number_token(b"{\"access_token\":7}".as_slice())]
#[case::json_array(b"[{\"access_token\":\"token\"}]".as_slice())]
#[case::plain_text(b"not json".as_slice())]
#[case::truncated_json(b"{\"access_token\":\"token\"".as_slice())]
#[case::float_expiry(b"{\"access_token\":\"token\",\"expires_in\":1.5}".as_slice())]
#[case::unreadable_expiry(b"{\"access_token\":\"token\",\"expires_in\":\"soon\"}".as_slice())]
#[case::empty_expiry(b"{\"access_token\":\"token\",\"expires_in\":\"\"}".as_slice())]
#[case::boolean_expiry(b"{\"access_token\":\"token\",\"expires_in\":true}".as_slice())]
fn unusable_replies_are_refused(#[case] body: &[u8]) {
    let _error = decode_gdc_token_reply(body).expect_err("a refused reply");
}

#[test]
fn surrounding_whitespace_does_not_hide_the_object() {
    let reply = decode_gdc_token_reply(b"  \n {\"access_token\":\"token\"} \t").unwrap();
    assert_eq!(reply.token.expose_secret(), "token");
}

#[test]
fn the_size_limit_keeps_the_boundary_and_refuses_one_byte_more() {
    let body = b"{\"access_token\":\"token\"}";
    let padding = MAX_REPLY_BYTES.saturating_sub(body.len());
    let at_limit = [body, " ".repeat(padding).as_bytes()].concat();
    let _reply = decode_gdc_token_reply(&at_limit).expect("a reply at the limit");
    let over = [body, " ".repeat(padding.saturating_add(1)).as_bytes()].concat();
    let _error = decode_gdc_token_reply(&over).expect_err("a reply one byte over the limit");
}
