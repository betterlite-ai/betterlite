use std::error::Error as _;

use secrecy::{ExposeSecret, SecretString};

use crate::resource_urls::error::ResourceUrlError;
use crate::resource_urls::{
    AzureResource, UrlRequest, complete_url, validate_environment, video_validate_environment,
};

use super::cases::VIDEO_URL_CASES;
use super::support::{auth, header_map, settings};
use super::urls::expect_urls;

fn container_headers() {
    let credentials = settings(None);
    let positional = SecretString::from("k1");
    let module_key = SecretString::from("g");
    let plain = validate_environment(auth(&[], None, &credentials), Some(&positional)).unwrap();
    assert_eq!(plain, header_map(&[("api-key", "k1")]));
    // header names compare exactly, so another casing neither blocks the key nor gets replaced by it
    let cased = auth(&[("API-Key", "old")], None, &credentials);
    assert_eq!(
        validate_environment(cased, Some(&positional)).unwrap(),
        header_map(&[("API-Key", "old"), ("api-key", "k1")])
    );
    let kept = auth(&[("api-key", "keep")], None, &credentials);
    assert_eq!(
        validate_environment(kept, Some(&positional)).unwrap(),
        header_map(&[("api-key", "keep")])
    );
    let empty = SecretString::from("");
    let fallback = auth(&[], Some(&module_key), &credentials);
    assert_eq!(
        validate_environment(fallback, Some(&empty)).unwrap(),
        header_map(&[("api-key", "g")])
    );
    let preferred = auth(&[], Some(&module_key), &credentials);
    assert_eq!(
        validate_environment(preferred, Some(&positional)).unwrap(),
        header_map(&[("api-key", "k1")])
    );
    let token = settings(Some("tok"));
    assert_eq!(
        validate_environment(auth(&[], None, &token), None).unwrap(),
        header_map(&[("Authorization", "Bearer tok")])
    );
}

fn video_headers() {
    let credentials = settings(None);
    let positional = SecretString::from("pos");
    let mut absent = None;
    let created = auth(&[], None, &credentials);
    assert_eq!(
        video_validate_environment(created, Some(&positional), &mut absent).unwrap(),
        header_map(&[("api-key", "pos")])
    );
    assert_eq!(
        absent.as_ref().map(ExposeSecret::expose_secret),
        Some("pos")
    );
    let mut own = Some(SecretString::from("own"));
    let owned = auth(&[], None, &credentials);
    assert_eq!(
        video_validate_environment(owned, Some(&positional), &mut own).unwrap(),
        header_map(&[("api-key", "own")])
    );
    assert_eq!(own.as_ref().map(ExposeSecret::expose_secret), Some("own"));
    let mut blank = Some(SecretString::from(""));
    let filled = auth(&[], None, &credentials);
    assert_eq!(
        video_validate_environment(filled, Some(&positional), &mut blank).unwrap(),
        header_map(&[("api-key", "pos")])
    );
    assert_eq!(blank.as_ref().map(ExposeSecret::expose_secret), Some("pos"));
    // an empty positional key fills nothing, so the token is the only credential left
    let token = settings(Some("tok"));
    let mut untouched = None;
    let empty = SecretString::from("");
    assert_eq!(
        video_validate_environment(auth(&[], None, &token), Some(&empty), &mut untouched).unwrap(),
        header_map(&[("Authorization", "Bearer tok")])
    );
    assert!(untouched.is_none());
}

pub(super) fn video_auth_and_url() {
    expect_urls(AzureResource::Video, &VIDEO_URL_CASES);
    let request = UrlRequest {
        api_base: None,
        fallback_api_base: Some("https://m.example"),
        api_version: Some("2025-01-01"),
    };
    assert_eq!(
        complete_url(AzureResource::Video, &request).unwrap(),
        "https://m.example/openai/v1/videos?api-version=2025-01-01"
    );
    container_headers();
    video_headers();
}

pub(super) fn vector_store_headers() {
    let credentials = settings(None);
    let module_key = SecretString::from("g");
    let own = SecretString::from("own");
    assert_eq!(
        validate_environment(auth(&[], Some(&module_key), &credentials), None).unwrap(),
        header_map(&[("api-key", "g")])
    );
    // no OpenAI-Beta header comes from the Azure override
    assert_eq!(
        validate_environment(auth(&[], Some(&module_key), &credentials), Some(&own)).unwrap(),
        header_map(&[("api-key", "own")])
    );
    let token = settings(Some("tok2"));
    assert_eq!(
        validate_environment(auth(&[], None, &token), None).unwrap(),
        header_map(&[("Authorization", "Bearer tok2")])
    );
    assert!(
        validate_environment(auth(&[], None, &credentials), None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn azure_resources_credential_identity_isolation() {
    let first_credentials = settings(Some("first-token"));
    let second_credentials = settings(Some("second-token"));
    let first_key = SecretString::from("first-identity-key");
    let second_key = SecretString::from("second-identity-key");
    let first = validate_environment(auth(&[], None, &first_credentials), Some(&first_key));
    let second = validate_environment(auth(&[], None, &second_credentials), Some(&second_key));
    assert_eq!(
        first.unwrap(),
        header_map(&[("api-key", "first-identity-key")])
    );
    assert_eq!(
        second.unwrap(),
        header_map(&[("api-key", "second-identity-key")])
    );
    assert_eq!(
        validate_environment(auth(&[], None, &first_credentials), None).unwrap(),
        header_map(&[("Authorization", "Bearer first-token")])
    );
    assert_eq!(
        validate_environment(auth(&[], None, &second_credentials), None).unwrap(),
        header_map(&[("Authorization", "Bearer second-token")])
    );
    let mut first_params = None;
    let mut second_params = None;
    let first_auth = auth(&[], None, &first_credentials);
    let second_auth = auth(&[], None, &second_credentials);
    video_validate_environment(first_auth, Some(&first_key), &mut first_params).unwrap();
    video_validate_environment(second_auth, Some(&second_key), &mut second_params).unwrap();
    assert_eq!(
        first_params.as_ref().map(ExposeSecret::expose_secret),
        Some("first-identity-key")
    );
    assert_eq!(
        second_params.as_ref().map(ExposeSecret::expose_secret),
        Some("second-identity-key")
    );
}

#[test]
fn azure_resources_secret_safe_failure() {
    let mut credentials = settings(Some("fixture-token"));
    credentials.tenant_id = Some("fixture-tenant".to_owned());
    credentials.client_id = Some("fixture-client".to_owned());
    credentials.client_secret = Some(SecretString::from("fixture-client-secret"));
    let debug = format!("{credentials:?}");
    assert!(!debug.contains("fixture-token"));
    assert!(!debug.contains("fixture-client-secret"));
    let failure = validate_environment(auth(&[], None, &credentials), None).unwrap_err();
    assert!(matches!(failure, ResourceUrlError::Token(_)));
    for text in [format!("{failure}"), format!("{failure:?}")] {
        assert!(!text.contains("fixture-client-secret"));
        assert!(!text.contains("fixture-token"));
    }
    let embedded = Some("https://user:fixture-api-key@[:::1]/openai/responses");
    for resource in [
        AzureResource::Container,
        AzureResource::Video,
        AzureResource::VectorStore,
    ] {
        let request = UrlRequest {
            api_base: embedded,
            fallback_api_base: None,
            api_version: None,
        };
        let error = complete_url(resource, &request).unwrap_err();
        let source = error.source().unwrap().to_string();
        for text in [format!("{error}"), format!("{error:?}"), source] {
            assert!(!text.contains("fixture-api-key"), "{resource:?}: {text}");
        }
    }
}
