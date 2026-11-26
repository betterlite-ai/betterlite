use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::jwt_keys::errors::{KeyError, exception_for};
use crate::jwt_keys::{
    DecodeError, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_DEPTH, decode_discovery_document,
    userinfo_document,
};

use super::support::{DISCOVERY_URL, JWKS_URL, ScriptedHttp, fixture, json, settings};

fn nested(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

#[rstest]
#[case::empty("")]
#[case::open_brace("{")]
#[case::not_json("not json")]
#[case::trailing("{\"a\":1} trailing")]
fn decoder_boundaries(#[case] body: &str) {
    // exactly the byte bound parses, one byte over is refused before parsing
    let exact = format!("\"{}\"", "a".repeat(MAX_DOCUMENT_BYTES - 2));
    assert_eq!(exact.len(), MAX_DOCUMENT_BYTES);
    assert!(decode_discovery_document(exact.as_bytes()).is_ok());
    let over = "a".repeat(MAX_DOCUMENT_BYTES + 1);
    let error = decode_discovery_document(over.as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        DecodeError::TooLarge {
            bytes,
            limit: MAX_DOCUMENT_BYTES,
        } if bytes == MAX_DOCUMENT_BYTES + 1
    ));

    // exactly the depth bound parses, one level more is refused
    assert!(decode_discovery_document(nested(MAX_DOCUMENT_DEPTH).as_bytes()).is_ok());
    let error = decode_discovery_document(nested(MAX_DOCUMENT_DEPTH + 1).as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        DecodeError::TooDeep {
            limit: MAX_DOCUMENT_DEPTH
        }
    ));
    let object_depth = format!(
        "{}1{}",
        r#"{"a":"#.repeat(MAX_DOCUMENT_DEPTH),
        "}".repeat(MAX_DOCUMENT_DEPTH)
    );
    assert!(decode_discovery_document(object_depth.as_bytes()).is_ok());

    // anything that is not JSON is a syntax failure
    let error = decode_discovery_document(body.as_bytes()).unwrap_err();
    assert!(matches!(error, DecodeError::Syntax(_)), "body {body:?}");
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

#[tokio::test]
async fn discovery_and_jwks_byte_depth_limits_return_auth_error() {
    let too_large = "a".repeat(MAX_DOCUMENT_BYTES + 1);

    let jwks = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &too_large)],
    );
    let error = jwks.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::JwksParse { .. }));
    assert!(
        error
            .to_string()
            .contains(&format!("{MAX_DOCUMENT_BYTES}-byte limit"))
    );
    let exception = exception_for(&error);
    assert_eq!(exception.code, "401");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert!(
        exception
            .message
            .starts_with("Authentication Error, Error parsing response:")
    );

    let too_deep = nested(MAX_DOCUMENT_DEPTH + 1);
    let deep = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &too_deep)],
    );
    let error = deep.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::JwksParse { .. }));
    assert!(
        error
            .to_string()
            .contains(&format!("nests deeper than {MAX_DOCUMENT_DEPTH} levels"))
    );

    // exactly at the byte bound, the document parses and fails later for the honest reason
    let mut body = String::from(r#"{"keys": null}"#);
    body.push_str(&" ".repeat(MAX_DOCUMENT_BYTES - body.len()));
    let exact = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &body)],
    );
    let error = exact.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::NoMatchingKey { .. }));

    // a discovery document is held to the same byte bound
    let discovery = fixture(
        settings(Some(DISCOVERY_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &too_large)],
    );
    let error = discovery
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::DiscoveryParse { .. }));
    assert_eq!(exception_for(&error).code, "401");
}

#[rstest]
fn replays_the_jwt_keys_corpus(#[files("fuzz/corpus/jwt_keys/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_discovery_document(&bytes));
    drop(userinfo_document(&bytes));
}
