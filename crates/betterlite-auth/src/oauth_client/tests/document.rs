use std::fs;
use std::path::{Path, PathBuf};

use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::oauth_client::document::{
    MAX_OAUTH_DOCUMENT_BYTES, OAuthDiscoveryDocument, OAuthDocumentError, OAuthDocumentKind,
    OAuthProviderDocument, OAuthTokenResponse, OAuthUserInfo, decode_oauth_provider_document,
};

#[test]
fn decodes_a_google_discovery_document_with_its_other_keys() {
    let document: OAuthDiscoveryDocument = decode_oauth_provider_document(
        br#"{"authorization_endpoint":"https://accounts.google.com/o/oauth2/v2/auth",
             "token_endpoint":"https://oauth2.googleapis.com/token",
             "userinfo_endpoint":"https://openidconnect.googleapis.com/v1/userinfo",
             "issuer":"https://accounts.google.com","jwks_uri":"https://www.googleapis.com/oauth2/v3/certs"}"#,
    )
    .unwrap();
    assert_eq!(
        document,
        OAuthDiscoveryDocument {
            authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth".to_owned(),
            token_endpoint: "https://oauth2.googleapis.com/token".to_owned(),
            userinfo_endpoint: "https://openidconnect.googleapis.com/v1/userinfo".to_owned(),
        }
    );
}

#[test]
fn a_discovery_document_without_an_endpoint_is_refused() {
    let error = decode_oauth_provider_document::<OAuthDiscoveryDocument>(
        br#"{"authorization_endpoint":"https://p/auth","token_endpoint":"https://p/token"}"#,
    )
    .unwrap_err();
    assert!(matches!(error, OAuthDocumentError::Document(_)));
}

#[test]
fn a_token_response_keeps_access_refresh_and_id_tokens_apart() {
    let tokens: OAuthTokenResponse = decode_oauth_provider_document(
        br#"{"access_token":"at","refresh_token":"rt","id_token":"it","token_type":"Bearer","expires_in":3600}"#,
    )
    .unwrap();
    assert_eq!(tokens.access_token.expose_secret(), "at");
    assert_eq!(
        tokens
            .refresh_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("rt")
    );
    assert_eq!(
        tokens.id_token.as_ref().map(|token| token.expose_secret()),
        Some("it")
    );
}

#[test]
fn a_token_response_without_tokens_has_none() {
    let tokens: OAuthTokenResponse =
        decode_oauth_provider_document(br#"{"access_token":"at"}"#).unwrap();
    assert!(tokens.refresh_token.is_none());
    assert!(tokens.id_token.is_none());
}

#[rstest]
#[case::null(br#"{"access_token":null}"#)]
#[case::missing(br#"{"refresh_token":"rt"}"#)]
#[case::number(br#"{"access_token":7}"#)]
fn a_token_response_without_an_access_token_string_is_refused(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_oauth_provider_document::<OAuthTokenResponse>(bytes),
        Err(OAuthDocumentError::Document(_))
    ));
}

#[test]
fn a_userinfo_document_keeps_every_provider_field() {
    let userinfo: OAuthUserInfo = decode_oauth_provider_document(
        br#"{"sub":"1","email_verified":true,"picture":null,"groups":["a","b"]}"#,
    )
    .unwrap();
    assert_eq!(userinfo.fields.len(), 4);
    assert!(matches!(
        userinfo.fields.get("email_verified"),
        Some(JsonValue::Bool(true))
    ));
}

#[test]
fn a_userinfo_document_must_be_an_object() {
    assert!(matches!(
        decode_oauth_provider_document::<OAuthUserInfo>(b"[\"sub\"]"),
        Err(OAuthDocumentError::Document(_))
    ));
}

#[test]
fn a_document_with_bytes_after_its_json_value_is_refused() {
    let error = decode_oauth_provider_document::<OAuthDiscoveryDocument>(
        br#"{"authorization_endpoint":"a","token_endpoint":"t","userinfo_endpoint":"u"} {}"#,
    )
    .unwrap_err();
    assert!(matches!(error, OAuthDocumentError::Trailing(_)));
}

#[test]
fn bytes_that_are_not_json_are_refused() {
    let error = decode_oauth_provider_document::<OAuthUserInfo>(b"not json").unwrap_err();
    assert!(matches!(error, OAuthDocumentError::Document(_)));
}

#[test]
fn a_document_over_the_transport_limit_is_refused() {
    let bytes = vec![b' '; MAX_OAUTH_DOCUMENT_BYTES + 1];
    let error = decode_oauth_provider_document::<OAuthDiscoveryDocument>(&bytes).unwrap_err();
    assert!(matches!(error, OAuthDocumentError::TooLarge { .. }));
}

#[test]
fn the_document_kind_tag_selects_the_document() {
    assert!(matches!(
        OAuthDocumentKind::try_from(0).unwrap().decode(b"{}"),
        Err(OAuthDocumentError::Document(_))
    ));
    assert!(matches!(
        OAuthDocumentKind::try_from(1)
            .unwrap()
            .decode(br#"{"access_token":"at"}"#),
        Ok(OAuthProviderDocument::Token(_))
    ));
    assert!(matches!(
        OAuthDocumentKind::try_from(2).unwrap().decode(b"{}"),
        Ok(OAuthProviderDocument::Userinfo(_))
    ));
    assert!(matches!(
        OAuthDocumentKind::try_from(9),
        Err(OAuthDocumentError::UnknownKind { tag: 9 })
    ));
}

#[rstest]
fn replays_the_oauth_provider_document_corpus(
    #[files("fuzz/corpus/oauth_provider_document/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    if let Ok(kind) = OAuthDocumentKind::try_from(tag) {
        drop(kind.decode(payload));
    }
}

#[test]
fn the_oauth_document_corpus_covers_every_kind() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/oauth_provider_document");
    let mut tags: Vec<u8> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap()[0])
        .collect();
    tags.sort_unstable();
    tags.dedup();
    // 0, 1 and 2 are the three document kinds the fuzz target decodes; 9 is a tag it must skip
    assert_eq!(tags, [0, 1, 2, 9]);
}
