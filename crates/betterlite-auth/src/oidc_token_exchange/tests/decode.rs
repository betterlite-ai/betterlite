use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::oidc_token_exchange::decode::{
    MAX_OIDC_RESPONSE_BYTES, OidcResponseError, OidcResponseKind, UserinfoDocument,
    decode_oidc_token_response, decode_oidc_userinfo,
};

use super::support::text;

#[test]
fn reads_the_access_token_and_every_claim_of_a_token_response() {
    let response = decode_oidc_token_response(
        br#"{"access_token":"access-1","token_type":"Bearer","expires_in":3600,"nested":{"list":[1,2]}}"#,
    )
    .unwrap();
    assert_eq!(response.access_token.expose_secret(), "access-1");
    assert_eq!(text(&response.claims, "token_type"), Some("Bearer"));
    assert_eq!(
        serde_json::to_string(&response.claims).unwrap(),
        r#"{"access_token":"access-1","expires_in":3600,"nested":{"list":[1,2]},"token_type":"Bearer"}"#
    );
}

#[test]
fn keeps_the_last_duplicate_access_token() {
    let response =
        decode_oidc_token_response(br#"{"access_token":"", "access_token":"last-wins"}"#).unwrap();
    assert_eq!(response.access_token.expose_secret(), "last-wins");
    assert_eq!(text(&response.claims, "access_token"), Some("last-wins"));
}

#[rstest]
#[case::not_json(br#"{"access_token":"#, "not-json")]
#[case::trailing_bytes(br#"{"access_token":"a"} trailing"#, "not-json")]
#[case::invalid_utf8(b"{\"access_token\":\"a\xff\"}", "not-json")]
#[case::empty_body(b"", "not-json")]
#[case::json_null(b"null", "not-an-object")]
#[case::json_bool(b"true", "not-an-object")]
#[case::json_list(br#"[{"access_token":"a"}]"#, "not-an-object")]
#[case::json_string(br#""access-token""#, "not-an-object")]
#[case::missing_access_token(br#"{"token_type":"Bearer"}"#, "missing-access-token")]
#[case::null_access_token(br#"{"access_token":null}"#, "missing-access-token")]
#[case::empty_access_token(br#"{"access_token":""}"#, "missing-access-token")]
#[case::numeric_access_token(br#"{"access_token":42}"#, "missing-access-token")]
#[case::object_access_token(br#"{"access_token":{"token":"a"}}"#, "missing-access-token")]
#[case::list_access_token(br#"{"access_token":["a"]}"#, "missing-access-token")]
#[case::nan_literal(br#"{"access_token":"a","score":NaN}"#, "not-json")]
#[case::infinity_literal(br#"{"access_token":"a","score":Infinity}"#, "not-json")]
fn rejects_token_responses_the_provider_should_not_send(#[case] body: &[u8], #[case] reason: &str) {
    let error = decode_oidc_token_response(body).unwrap_err();
    assert_eq!(reason_of(&error), reason);
}

#[rstest]
#[case::claims(br#"{"sub":"user-1","email":"u@example.com"}"#, "claims")]
#[case::nested_claims(br#"{"sub":"user-1","nested":{"roles":["admin"]}}"#, "claims")]
#[case::empty_object(b"{}", "empty")]
#[case::json_null(b"null", "empty")]
#[case::json_false(b"false", "empty")]
#[case::json_zero(b"0", "empty")]
#[case::json_float_zero(b"0.0", "empty")]
#[case::json_empty_string(b"\"\"", "empty")]
#[case::empty_list(b"[]", "empty")]
#[case::json_true(b"true", "not-an-object")]
#[case::nonzero_number(b"5", "not-an-object")]
#[case::non_empty_list(br#"[{"sub":"user-1"}]"#, "not-an-object")]
#[case::json_string(br#""user-1""#, "not-an-object")]
#[case::not_json(br#"{"sub":"#, "not-json")]
#[case::invalid_utf8(b"{\"sub\":\"\xff\"}", "not-json")]
fn classifies_userinfo_bodies(#[case] body: &[u8], #[case] shape: &str) {
    let result = decode_oidc_userinfo(body);
    assert_eq!(shape_or_reason(result), shape);
}

#[test]
fn keeps_an_integer_past_u64_exactly() {
    let response =
        decode_oidc_token_response(br#"{"access_token":"a","user_id":18446744073709551616}"#)
            .unwrap();
    assert_eq!(
        serde_json::to_string(&response.claims["user_id"].json).unwrap(),
        "18446744073709551616"
    );
}

#[test]
fn keeps_a_number_past_f64_range_as_the_provider_wrote_it() {
    let response =
        decode_oidc_token_response(br#"{"access_token":"a","expires_in":1e400}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&response.claims["expires_in"].json).unwrap(),
        "1e+400"
    );
}

#[test]
fn keeps_userinfo_claims_as_the_provider_sent_them() {
    let document =
        decode_oidc_userinfo(br#"{"sub":"user-1","roles":["a","b"],"count":2}"#).unwrap();
    let UserinfoDocument::Claims(claims) = document else {
        panic!("a JSON object carries claims");
    };
    assert_eq!(
        serde_json::to_string(&claims).unwrap(),
        r#"{"count":2,"roles":["a","b"],"sub":"user-1"}"#
    );
}

#[test]
fn accepts_a_body_at_the_size_limit_and_refuses_one_past_it() {
    let head = "{\"access_token\":\"x\",\"pad\":\"";
    let tail = "\"}";
    let padding = MAX_OIDC_RESPONSE_BYTES - head.len() - tail.len();
    let mut body = String::with_capacity(MAX_OIDC_RESPONSE_BYTES);
    body.push_str(head);
    body.push_str(&"a".repeat(padding));
    body.push_str(tail);
    assert_eq!(body.len(), MAX_OIDC_RESPONSE_BYTES);
    decode_oidc_token_response(body.as_bytes()).unwrap();
    body.push('a');
    let error = decode_oidc_token_response(body.as_bytes()).unwrap_err();
    assert_eq!(reason_of(&error), "too-large");
}

#[rstest]
fn replays_the_oidc_token_response_corpus(
    #[files("fuzz/corpus/oidc_token_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    if let Ok(kind) = OidcResponseKind::try_from(tag) {
        drop(kind.decode(payload));
    }
}

#[test]
fn the_oidc_token_response_corpus_reaches_both_boundaries() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/oidc_token_response");
    let kinds = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter_map(|bytes| bytes.first().copied())
        .filter_map(|tag| OidcResponseKind::try_from(tag).ok())
        .collect::<Vec<OidcResponseKind>>();
    assert!(kinds.contains(&OidcResponseKind::Token));
    assert!(kinds.contains(&OidcResponseKind::Userinfo));
}

#[test]
fn refuses_a_tag_no_response_kind_names() {
    let error = OidcResponseKind::try_from(7_u8).unwrap_err();
    assert_eq!(reason_of(&error), "unknown-kind");
}

fn shape_or_reason(result: Result<UserinfoDocument, OidcResponseError>) -> &'static str {
    match result {
        Ok(UserinfoDocument::Claims(_)) => "claims",
        Ok(UserinfoDocument::Empty) => "empty",
        Ok(UserinfoDocument::NotAnObject) => "not-an-object",
        Err(error) => reason_of(&error),
    }
}

fn reason_of(error: &OidcResponseError) -> &'static str {
    match error {
        OidcResponseError::TooLarge { .. } => "too-large",
        OidcResponseError::Json(_) => "not-json",
        OidcResponseError::NotAnObject => "not-an-object",
        OidcResponseError::MissingAccessToken => "missing-access-token",
        OidcResponseError::Claims(_) => "claims",
        OidcResponseError::UnknownKind { .. } => "unknown-kind",
    }
}
