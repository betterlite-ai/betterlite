use betterlite_config::sso_mappings::{
    CLIENT_SECRET_ENV, ROLE_MAPPINGS_ROLES_ENV, SSO_HEADERS_ENV, TOKEN_ENDPOINT_ENV,
};
use betterlite_error::proxy_error_types::ProxyErrorTypes;

use crate::generic_sso_callback::failure::GenericSsoError;

use super::support::{FakeCache, FakeRows, Fixture, Replies, call, mapping, pkce, sources};

fn assert_guard(error: &GenericSsoError, status: u16, param: &str) {
    let response = error.response();
    assert_eq!(response.status, status);
    assert_eq!(response.error_type, ProxyErrorTypes::AuthError);
    assert_eq!(response.param.as_deref(), Some(param));
    assert_eq!(response.code, status.to_string());
}

#[tokio::test]
async fn missing_settings_fail_before_the_row_is_read() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.sources = sources(&[]);
    let rows = FakeRows::absent();
    let cache = FakeCache::empty();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &rows,
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, GenericSsoError::MissingVariable(_)));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type),
        (500, ProxyErrorTypes::AuthError)
    );
    assert_eq!(response.param.as_deref(), Some(CLIENT_SECRET_ENV));
    assert_eq!(rows.reads(), 0);
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn a_role_mapping_failure_precedes_the_header_failure() {
    let mut fixture = Fixture::start(Replies::new()).await;
    let sources = mapping(
        &fixture.base,
        &[
            (ROLE_MAPPINGS_ROLES_ENV, "[1, 2"),
            (SSO_HEADERS_ENV, "no-equals-sign"),
        ],
    );
    fixture.sources = sources;
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, GenericSsoError::RoleMapping(_)));
    assert_eq!(
        (error.response().status, error.response().error_type,),
        (500, ProxyErrorTypes::InternalServerError)
    );
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn a_missing_state_cookie_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        None,
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 400, "state");
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn an_empty_state_cookie_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some(""),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 400, "state");
    assert!(cache.deletes().is_empty());
}

// LiteLLM's compare_digest 500s on non-ASCII text; the byte compare matches it and keeps the 400
#[tokio::test]
async fn a_non_ascii_state_pair_matches_bytewise() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc&state=caf%C3%A9"),
        Some("café"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap();
    assert_eq!(result.identity.id.as_deref(), Some("user-1"));
    assert_eq!(cache.deletes().len(), 1);
}

#[tokio::test]
async fn a_state_that_differs_from_the_cookie_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-2"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 400, "state");
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn a_callback_without_code_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 400, "code");
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn an_empty_code_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("code=&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 400, "code");
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn an_empty_client_id_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    fixture.client_id = String::new();
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 401, "GENERIC_CLIENT_ID");
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn an_empty_token_endpoint_is_refused() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let sources = mapping(&fixture.base, &[(TOKEN_ENDPOINT_ENV, "")]);
    fixture.sources = sources;
    let cache = FakeCache::holding_verifier();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert_guard(&error, 401, "GENERIC_TOKEN_ENDPOINT");
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn the_last_state_value_in_the_callback_wins() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc&state=stale&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap();
    assert_eq!(result.identity.id.as_deref(), Some("user-1"));
    assert_eq!(cache.deletes().len(), 1);
}
