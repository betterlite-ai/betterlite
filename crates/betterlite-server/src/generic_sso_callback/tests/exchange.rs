use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sso_mappings::{INCLUDE_CLIENT_ID_ENV, SSO_HEADERS_ENV};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::ExposeSecret as _;

use crate::generic_sso_callback::failure::GenericSsoError;
use crate::sso_pkce::{PKCE_CACHE_MISS_PARAM, PKCE_VERIFIER_CACHE_PREFIX};

use super::support::{
    CLIENT_ID, FakeCache, FakeRows, Fixture, REDIRECT_URI, Replies, TokenReply, VERIFIER, call,
    mapping, pkce, team_handler, text, token,
};

fn cached_key() -> String {
    format!("{PKCE_VERIFIER_CACHE_PREFIX}state-1")
}

#[tokio::test]
async fn a_pkce_callback_exchanges_with_the_cached_verifier() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::holding_verifier();
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap();
    assert_eq!(result.identity.id.as_deref(), Some("user-1"));
    assert_eq!(result.identity.email.as_deref(), Some("user@example.test"));
    assert_eq!(result.identity.display_name.as_deref(), Some("subject-1"));
    assert!(result.access_token_payload.is_none());
    assert!(result.sso_assertion.is_none());
    let seen = fixture.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/token");
    assert_eq!(seen[0].form("code"), "abc");
    assert_eq!(seen[0].form("code_verifier"), VERIFIER);
    assert_eq!(seen[0].form("redirect_uri"), REDIRECT_URI);
    assert_eq!(seen[0].form("client_id"), "");
    let basic = format!(
        "Basic {}",
        STANDARD.encode(format!("{CLIENT_ID}:generic-secret"))
    );
    assert_eq!(seen[0].header("authorization"), Some(basic.as_str()));
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].path, "/userinfo");
    assert_eq!(
        seen[1].header("authorization"),
        Some("Bearer access-opaque")
    );
    assert_eq!(cache.deletes(), vec![cached_key()]);
}

#[tokio::test]
async fn an_include_client_id_exchange_puts_the_client_in_the_form() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let sources = mapping(&fixture.base, &[(INCLUDE_CLIENT_ID_ENV, "true")]);
    fixture.sources = sources;
    let cache = FakeCache::holding_verifier();
    call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap();
    let seen = fixture.requests();
    assert_eq!(seen[0].form("client_id"), CLIENT_ID);
    assert_eq!(seen[0].form("client_secret"), "generic-secret");
    assert_eq!(seen[0].header("authorization"), None);
    assert_eq!(cache.deletes(), vec![cached_key()]);
}

#[tokio::test]
async fn a_permissive_cache_miss_uses_the_non_pkce_exchange() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let cache = FakeCache::empty();
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap();
    assert_eq!(result.identity.id.as_deref(), Some("user-1"));
    let seen = fixture.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].path, "/token");
    assert_eq!(seen[0].form("code"), "abc");
    assert_eq!(seen[0].form("code_verifier"), "");
    assert_eq!(seen[0].form("redirect_uri"), REDIRECT_URI);
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn a_conversion_failure_keeps_the_verifier() {
    let mut replies = Replies::new();
    let groups = JsonValue::Array(vec![text("admins")]);
    replies
        .userinfo
        .insert("preferred_username".to_owned(), groups);
    let mut fixture = Fixture::start(replies).await;
    fixture.pkce = pkce(true);
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
    assert!(matches!(&error, GenericSsoError::Claims(_)));
    assert_eq!(
        (error.response().status, error.response().error_type),
        (500, ProxyErrorTypes::InternalServerError)
    );
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn an_access_token_fills_in_the_missing_teams() {
    let mut replies = Replies::new();
    replies.token = TokenReply {
        access_token: token(r#"{"teams":["team-a"]}"#),
        id_token: None,
        refresh_token: None,
    };
    let mut fixture = Fixture::start(replies).await;
    fixture.sso_jwt_handler = Some(team_handler("teams"));
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    assert_eq!(result.identity.team_ids, vec!["team-a".to_owned()]);
}

#[tokio::test]
async fn an_access_token_augmentation_failure_keeps_the_verifier() {
    let mut replies = Replies::new();
    replies.token = TokenReply {
        access_token: token(r#"{"teams":[1]}"#),
        id_token: None,
        refresh_token: None,
    };
    let mut fixture = Fixture::start(replies).await;
    fixture.pkce = pkce(true);
    fixture.sso_jwt_handler = Some(team_handler("teams"));
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
    assert!(matches!(&error, GenericSsoError::Claims(_)));
    assert_eq!(error.response().status, 500);
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn the_configured_headers_reach_the_non_pkce_provider() {
    let mut fixture = Fixture::start(Replies::new()).await;
    let sources = mapping(
        &fixture.base,
        &[(SSO_HEADERS_ENV, "X-Api-Key=header-secret")],
    );
    fixture.sources = sources;
    call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    let seen = fixture.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].header("x-api-key"), Some("header-secret"));
    assert_eq!(seen[1].header("x-api-key"), Some("header-secret"));
}

#[tokio::test]
async fn a_header_the_pkce_exchange_cannot_send_answers_its_typed_failure() {
    let mut fixture = Fixture::start(Replies::new()).await;
    fixture.pkce = pkce(true);
    let sources = mapping(&fixture.base, &[(SSO_HEADERS_ENV, "X-Api-Key=line\nbreak")]);
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
    assert!(matches!(&error, GenericSsoError::TokenExchange(_)));
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
            Some("token_exchange"),
            "401"
        )
    );
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn a_strict_cache_miss_answers_the_pkce_cache_error() {
    let mut fixture = Fixture::start(Replies::new()).await;
    let mut settings = pkce(true);
    settings.strict_cache_miss = true;
    fixture.pkce = settings;
    let cache = FakeCache::empty();
    let error = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, GenericSsoError::Pkce(_)));
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
            Some(PKCE_CACHE_MISS_PARAM),
            "401"
        )
    );
    assert!(fixture.requests().is_empty());
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn an_opaque_access_token_leaves_no_payload() {
    let fixture = Fixture::start(Replies::new()).await;
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    assert!(result.access_token_payload.is_none());
    assert!(result.identity.team_ids.is_empty());
    assert!(result.identity.user_role.is_none());
}

#[tokio::test]
async fn an_undecodable_id_token_is_not_retained() {
    let mut replies = Replies::new();
    replies.token = TokenReply {
        access_token: "access-opaque".to_owned(),
        id_token: Some("not-a-jwt".to_owned()),
        refresh_token: Some("refresh-1".to_owned()),
    };
    let fixture = Fixture::start(replies).await;
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    assert!(result.sso_assertion.is_none());
}

#[tokio::test]
async fn a_decodable_id_token_is_retained_with_its_issuer() {
    let mut replies = Replies::new();
    replies.token = TokenReply {
        access_token: "access-opaque".to_owned(),
        id_token: Some(token(r#"{"iss":"https://issuer.test","exp":2000000000}"#)),
        refresh_token: Some("refresh-1".to_owned()),
    };
    let fixture = Fixture::start(replies).await;
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    let assertion = result.sso_assertion.unwrap();
    assert_eq!(assertion.issuer.as_deref(), Some("https://issuer.test"));
    assert_eq!(
        assertion.id_token.expose_secret(),
        token(r#"{"iss":"https://issuer.test","exp":2000000000}"#)
    );
    assert_eq!(
        assertion.refresh_token.unwrap().expose_secret(),
        "refresh-1"
    );
}
