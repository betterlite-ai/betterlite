use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::support::{
    DISCOVERY_URL, JWKS_URL, ScriptedHttp, json, key, keys, new_resolver_fixture, settings,
    stale_key, timestamp_key,
};
use crate::jwt_keys::errors::{KeyError, exception_for};
use crate::jwt_keys::{
    DecodeError, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_DEPTH, decode_discovery_document,
    userinfo_document,
};

fn nested(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

#[rstest]
#[case::empty("")]
#[case::open_brace("{")]
#[case::not_json("not json")]
#[case::trailing("{\"a\":1} trailing")]
fn decoder_boundaries(#[case] body: &str) {
    let exact = format!("\"{}\"", "a".repeat(MAX_DOCUMENT_BYTES - 2));
    assert_eq!(exact.len(), MAX_DOCUMENT_BYTES);
    decode_discovery_document(exact.as_bytes()).unwrap();
    let over = "a".repeat(MAX_DOCUMENT_BYTES + 1);
    let size_error = decode_discovery_document(over.as_bytes()).unwrap_err();
    assert!(
        matches!(size_error, DecodeError::TooLarge {bytes, limit: MAX_DOCUMENT_BYTES} if bytes == MAX_DOCUMENT_BYTES + 1)
    );
    decode_discovery_document(nested(MAX_DOCUMENT_DEPTH).as_bytes()).unwrap();
    let depth_error =
        decode_discovery_document(nested(MAX_DOCUMENT_DEPTH + 1).as_bytes()).unwrap_err();
    assert!(matches!(
        depth_error,
        DecodeError::TooDeep {
            limit: MAX_DOCUMENT_DEPTH
        }
    ));
    let object_depth = format!(
        "{}1{}",
        r#"{"a":"#.repeat(MAX_DOCUMENT_DEPTH),
        "}".repeat(MAX_DOCUMENT_DEPTH)
    );
    decode_discovery_document(object_depth.as_bytes()).unwrap();
    let syntax_error = decode_discovery_document(body.as_bytes()).unwrap_err();
    assert!(
        matches!(syntax_error, DecodeError::Syntax(_)),
        "body {body:?}"
    );
}

#[rstest]
#[case::array("[]")]
#[case::string("\"x\"")]
#[case::number("7")]
#[case::null("null")]
#[case::boolean("true")]
fn userinfo_document_requires_json_object(#[case] body: &str) {
    let fields = userinfo_document(br#"{"sub":"a","email":"one@example.com"}"#).unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields.get("sub"), Some(&json(r#""a""#)));
    let error = userinfo_document(body.as_bytes()).unwrap_err();
    assert!(
        matches!(
            error,
            DecodeError::NotObject {
                document: "UserInfo"
            }
        ),
        "body {body}"
    );
}

#[rstest]
#[case::jwks_bytes(JWKS_URL, "a".repeat(MAX_DOCUMENT_BYTES + 1))]
#[case::jwks_depth(JWKS_URL, nested(MAX_DOCUMENT_DEPTH + 1))]
#[case::discovery_bytes(DISCOVERY_URL, "a".repeat(MAX_DOCUMENT_BYTES + 1))]
#[case::discovery_depth(DISCOVERY_URL, nested(MAX_DOCUMENT_DEPTH + 1))]
#[tokio::test]
async fn discovery_and_jwks_byte_depth_limits_return_auth_error(
    #[case] url: &str,
    #[case] body: String,
) {
    let fixture = new_resolver_fixture(
        settings(Some(url), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &body)],
    );
    fixture.cache.seed(&stale_key(), keys(&[key("a")]));
    fixture.cache.seed(&timestamp_key(), json("1000"));
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        KeyError::JwksParse { .. } | KeyError::DiscoveryParse { .. }
    ));
    let exception = exception_for(&error);
    assert_eq!(exception.code, "401");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert!(exception.message.starts_with("Authentication Error, "));
    assert_eq!(fixture.http.requests(), vec![url.to_owned()]);
    assert!(fixture.clock.sleeps().is_empty());
    assert!(fixture.cache.writes().is_empty());
}

#[tokio::test]
async fn exact_byte_bound_reaches_key_selection() {
    let mut body = String::from(r#"{"keys": null}"#);
    body.push_str(&" ".repeat(MAX_DOCUMENT_BYTES - body.len()));
    let fixture = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &body)],
    );
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::NoMatchingKey { .. }));
}

#[rstest]
fn replays_the_jwt_keys_corpus(#[files("fuzz/corpus/jwt_keys/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_discovery_document(&bytes));
    drop(userinfo_document(&bytes));
}
