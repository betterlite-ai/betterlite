use betterlite_config::sso_mappings::SSO_HEADERS_ENV;
use betterlite_error::proxy_error_types::ProxyErrorTypes;

use crate::generic_sso_callback::failure::{
    GENERIC_CLIENT_USE_PKCE_PARAM, GenericSsoError, GenericSsoErrorResponse,
    classify_generic_sso_error, pkce_hint_message, pkce_hint_required, pkce_hint_response,
};

use super::support::{FakeCache, FakeRows, Fixture, Replies, call, capture_logs, mapping, pkce};

fn source_response() -> GenericSsoErrorResponse {
    GenericSsoErrorResponse {
        status: 401,
        error_type: ProxyErrorTypes::AuthError,
        param: Some("token_exchange".to_owned()),
        code: "401".to_owned(),
    }
}

#[test]
fn the_hint_needs_the_written_word_or_the_lowercased_phrase() {
    assert!(pkce_hint_required("provider requires PKCE", false));
    assert!(pkce_hint_required("missing Code Verifier value", false));
    assert!(pkce_hint_required("missing code verifier value", false));
    assert!(!pkce_hint_required("the provider said pkce", false));
    assert!(!pkce_hint_required("provider requires PKCE", true));
}

#[test]
fn the_classification_keeps_an_unrelated_error() {
    let source = source_response();
    let classified = classify_generic_sso_error("access denied", source.clone(), false);
    assert_eq!(classified, source);
}

#[test]
fn the_classification_replaces_a_pkce_error_when_pkce_is_off() {
    let classified = classify_generic_sso_error("provider requires PKCE", source_response(), false);
    assert_eq!(classified, pkce_hint_response());
    assert_eq!(
        (
            classified.status,
            classified.error_type,
            classified.param.as_deref(),
        ),
        (
            401,
            ProxyErrorTypes::AuthError,
            Some(GENERIC_CLIENT_USE_PKCE_PARAM)
        )
    );
}

#[test]
fn the_hint_message_names_okta_only_for_an_okta_endpoint() {
    let okta = pkce_hint_message(
        "original error",
        "https://example.okta.test/authorize",
        "https://example.test/token",
    );
    assert!(okta.contains("Okta requires PKCE"));
    assert!(okta.contains("For Docker: Add -e GENERIC_CLIENT_USE_PKCE=true"));
    assert!(okta.ends_with("Original error: original error"));
    let okta_token_endpoint = pkce_hint_message(
        "original error",
        "https://example.test/authorize",
        "https://OKTA.example.test/token",
    );
    assert!(okta_token_endpoint.contains("Okta requires PKCE"));
    let generic = pkce_hint_message(
        "original error",
        "https://example.test/authorize",
        "https://example.test/token",
    );
    assert!(generic.contains("Your OAuth provider requires PKCE"));
    assert!(!generic.contains("For Docker"));
}

#[tokio::test]
async fn a_pkce_provider_error_becomes_the_configuration_hint() {
    let fixture = Fixture::start(Replies::new()).await;
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&error=PKCE_required&error_description=send+challenge"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    let response = error.response();
    assert_eq!(
        (
            response.status,
            response.error_type,
            response.param.as_deref(),
            response.code.as_str(),
        ),
        (
            401,
            ProxyErrorTypes::AuthError,
            Some(GENERIC_CLIENT_USE_PKCE_PARAM),
            "401"
        )
    );
    assert!(error.to_string().contains("Original error:"));
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn a_provider_error_with_the_phrase_code_verifier_is_a_hint() {
    let fixture = Fixture::start(Replies::new()).await;
    let error = call(
        &fixture,
        &fixture
            .callback_url("code=abc&error=invalid_grant&error_description=Code+Verifier+required"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.response().param.as_deref(),
        Some(GENERIC_CLIENT_USE_PKCE_PARAM)
    );
}

#[tokio::test]
async fn a_lowercase_pkce_word_is_not_matched_by_itself() {
    let fixture = Fixture::start(Replies::new()).await;
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&error=pkce_required"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, GenericSsoError::OAuth(_)));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type, response.param),
        (401, ProxyErrorTypes::AuthError, None)
    );
    assert!(error.to_string().contains("pkce_required"));
}

#[tokio::test]
async fn an_unrelated_provider_error_is_preserved() {
    let fixture = Fixture::start(Replies::new()).await;
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&error=access_denied&error_description=user+refused"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type, response.param),
        (401, ProxyErrorTypes::AuthError, None)
    );
    assert!(error.to_string().contains("access_denied"));
    assert!(error.to_string().contains("user refused"));
}

#[tokio::test]
async fn a_pkce_error_keeps_its_provider_message_when_pkce_is_enabled() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&error=PKCE_required"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type, response.param),
        (401, ProxyErrorTypes::AuthError, None)
    );
    assert!(error.to_string().contains("PKCE_required"));
}

#[tokio::test]
async fn the_failure_log_omits_the_header_values() {
    let mut replies = Replies::new();
    replies.token_status = 500;
    let mut fixture = Fixture::start(replies).await;
    let sources = mapping(
        &fixture.base,
        &[(SSO_HEADERS_ENV, "X-Api-Key=header-secret,X-Tenant=tenant-1")],
    );
    fixture.sources = sources;
    let (logs, _guard) = capture_logs(tracing::Level::TRACE);
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.response().status,
        401,
        "the token endpoint failure keeps its own response"
    );
    let captured = logs.contents();
    assert!(captured.contains("SSO authentication failed"));
    assert!(!captured.contains("header-secret"));
    assert!(!captured.contains("X-Api-Key"));
    assert!(!error.to_string().contains("header-secret"));
}
