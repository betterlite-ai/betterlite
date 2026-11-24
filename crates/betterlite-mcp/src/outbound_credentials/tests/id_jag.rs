use std::sync::Arc;

use crate::credential_contracts::errors::CredError;
use crate::credential_contracts::server_spec::ServerSpec;

use super::super::resolver::{Collaborators, UpstreamCredentialProvider};
use super::super::token_cache::ExchangedTokenCache;
use super::support::{
    FakeAssertions, FakeClock, FakeEndpoint, RecordedAuth, bearer, failure, instant, param,
    provider, shared_clock, shared_endpoint, spec_from, subject,
};

const MISSING_CALLER: &str = "ID-JAG requires an identified caller; this request carries neither an identity token nor a resolved LiteLLM user.";
const NO_ASSERTION: &str = "ID-JAG requires an IdP identity assertion for this user and none is stored. Sign in through LiteLLM SSO so the gateway captures one.";
const EXPIRED_ASSERTION: &str = "The stored IdP identity assertion for this user has expired. Sign in through LiteLLM SSO again to capture a current one.";
const STORE_OUTAGE: &str =
    "The IdP identity assertion store is unreachable, so ID-JAG cannot resolve a subject.";
const FIXTURE_NOW: i64 = 1_700_000_000;
const ASSERTION_EXPIRY: i64 = 1_700_003_600;

fn id_jag_spec_with(
    org_endpoint: &str,
    resource_endpoint: &str,
    client_secret: &str,
    scopes: &[&str],
) -> ServerSpec {
    let scopes_json = serde_json::to_string(scopes).expect("scopes fixture");
    let encoded = format!(
        r#"{{"server_id":"srv-1","name":"ID JAG","transport":"http","url":"https://mcp.example.com/mcp","auth_type":"oauth2_id_jag","token_exchange_endpoint":"{org_endpoint}","id_jag_resource_token_endpoint":"{resource_endpoint}","client_id":"client-1","client_secret":"{client_secret}","scopes":{scopes_json}}}"#
    );
    spec_from(&encoded)
}

fn standard_spec() -> ServerSpec {
    id_jag_spec_with(
        "https://idp.example.com/org/token",
        "https://upstream.example.com/token",
        "secret-1",
        &["read"],
    )
}

fn stored_assertion_resolver(endpoint: Arc<FakeEndpoint>) -> UpstreamCredentialProvider {
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
        .with_token_endpoint(endpoint);
    provider(collaborators)
}

#[tokio::test]
async fn id_jag_slot_key_principal_variance() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["a1", "a2", "b1", "b2", "c1", "c2"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let server = standard_spec();
    let alice = subject("org-a", "user-1", None);
    let first = resolver.resolve_credentials(&alice, &server).await.unwrap();
    let tenant_peer = subject("org-b", "user-1", None);
    let second = resolver
        .resolve_credentials(&tenant_peer, &server)
        .await
        .unwrap();
    assert_eq!(bearer(&first), "a2");
    assert_eq!(bearer(&second), "b2");
    let again = resolver.resolve_credentials(&alice, &server).await.unwrap();
    assert_eq!(bearer(&again), "a2");
    assert_eq!(endpoint.calls().len(), 4);
    let user_peer = subject("org-a", "user-2", None);
    let third = resolver
        .resolve_credentials(&user_peer, &server)
        .await
        .unwrap();
    assert_eq!(bearer(&third), "c2");
    assert_eq!(endpoint.calls().len(), 6);
}

#[tokio::test]
async fn id_jag_slot_key_inbound_token_variance() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["a1", "a2", "b1", "b2", "c1", "c2"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let server = standard_spec();
    let first = resolver
        .resolve_credentials(&subject("org-1", "user-1", Some("one")), &server)
        .await
        .unwrap();
    let second = resolver
        .resolve_credentials(&subject("org-1", "user-1", Some("two")), &server)
        .await
        .unwrap();
    assert_eq!(bearer(&first), "a2");
    assert_eq!(bearer(&second), "b2");
    let again = resolver
        .resolve_credentials(&subject("org-1", "user-1", Some("one")), &server)
        .await
        .unwrap();
    assert_eq!(bearer(&again), "a2");
    assert_eq!(endpoint.calls().len(), 4);
    let stored = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &server)
        .await
        .unwrap();
    assert_eq!(bearer(&stored), "c2");
    assert_eq!(endpoint.calls().len(), 6);
}

#[tokio::test]
async fn id_jag_fingerprint_config_variance() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["a1", "a2", "b1", "b2", "c1", "c2"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let narrow = id_jag_spec_with(
        "https://idp.example.com/org/token",
        "https://upstream.example.com/token",
        "secret-1",
        &["read"],
    );
    let wide = id_jag_spec_with(
        "https://idp.example.com/org/token",
        "https://upstream.example.com/token",
        "secret-1",
        &["read", "write"],
    );
    let caller = subject("org-1", "user-1", None);
    let first = resolver
        .resolve_credentials(&caller, &narrow)
        .await
        .unwrap();
    let second = resolver.resolve_credentials(&caller, &wide).await.unwrap();
    assert_eq!(bearer(&first), "a2");
    assert_eq!(bearer(&second), "b2");
    let again = resolver
        .resolve_credentials(&caller, &narrow)
        .await
        .unwrap();
    assert_eq!(bearer(&again), "c2");
    assert_eq!(endpoint.calls().len(), 6);
}

#[tokio::test]
async fn id_jag_fingerprint_includes_endpoints() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["a1", "a2", "b1", "b2", "c1", "c2"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let first_server = standard_spec();
    let other_server = id_jag_spec_with(
        "https://idp.example.com/other/token",
        "https://upstream.example.com/token",
        "secret-1",
        &["read"],
    );
    let caller = subject("org-1", "user-1", None);
    let first = resolver
        .resolve_credentials(&caller, &first_server)
        .await
        .unwrap();
    let second = resolver
        .resolve_credentials(&caller, &other_server)
        .await
        .unwrap();
    assert_eq!(bearer(&first), "a2");
    assert_eq!(bearer(&second), "b2");
    let again = resolver
        .resolve_credentials(&caller, &first_server)
        .await
        .unwrap();
    assert_eq!(bearer(&again), "c2");
    let calls = endpoint.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[0].endpoint, "https://idp.example.com/org/token");
    assert_eq!(calls[2].endpoint, "https://idp.example.com/other/token");
    assert_eq!(calls[4].endpoint, "https://idp.example.com/org/token");
}

#[tokio::test]
async fn id_jag_fingerprint_includes_client_auth() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["a1", "a2", "b1", "b2", "c1", "c2"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let first_server = standard_spec();
    let other_server = id_jag_spec_with(
        "https://idp.example.com/org/token",
        "https://upstream.example.com/token",
        "secret-2",
        &["read"],
    );
    let caller = subject("org-1", "user-1", None);
    let _first = resolver
        .resolve_credentials(&caller, &first_server)
        .await
        .unwrap();
    let _second = resolver
        .resolve_credentials(&caller, &other_server)
        .await
        .unwrap();
    let _third = resolver
        .resolve_credentials(&caller, &first_server)
        .await
        .unwrap();
    let calls = endpoint.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(
        calls[0].client_auth,
        RecordedAuth::client_secret("secret-1")
    );
    assert_eq!(
        calls[2].client_auth,
        RecordedAuth::client_secret("secret-2")
    );
    assert_eq!(
        calls[4].client_auth,
        RecordedAuth::client_secret("secret-1")
    );
}

#[tokio::test]
async fn id_jag_subject_token_from_the_store() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["jag", "bearer"]));
    let clock = Arc::new(FakeClock::at(FIXTURE_NOW));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing(
            "stored-id-token",
            Some(instant(ASSERTION_EXPIRY)),
        )))
        .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint)))
        .with_wall_clock(shared_clock(clock));
    let resolver = provider(collaborators);
    let auth = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &standard_spec())
        .await
        .unwrap();
    assert_eq!(bearer(&auth), "bearer");
    assert_eq!(
        param(&endpoint.calls()[0], "subject_token"),
        Some("stored-id-token")
    );
}

#[tokio::test]
async fn id_jag_inbound_token_wins_over_the_store() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["jag", "bearer"]));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing("stored-id-token", None)))
        .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint)));
    let resolver = provider(collaborators);
    let auth = resolver
        .resolve_credentials(
            &subject("org-1", "user-1", Some("caller-id-token")),
            &standard_spec(),
        )
        .await
        .unwrap();
    assert_eq!(bearer(&auth), "bearer");
    assert_eq!(
        param(&endpoint.calls()[0], "subject_token"),
        Some("caller-id-token")
    );
}

#[tokio::test]
async fn id_jag_omits_absent_optional_parameters() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["jag", "bearer"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let server = id_jag_spec_with(
        "https://idp.example.com/org/token",
        "https://upstream.example.com/token",
        "secret-1",
        &[],
    );
    let _credential = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &server)
        .await
        .unwrap();
    let calls = endpoint.calls();
    let first = &calls[0];
    assert_eq!(param(first, "audience"), None);
    assert_eq!(param(first, "resource"), None);
    assert_eq!(param(first, "scope"), None);
    assert_eq!(
        param(first, "subject_token_type"),
        Some("urn:ietf:params:oauth:token-type:id_token")
    );
}

#[tokio::test]
async fn id_jag_drops_empty_optional_parameters() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["jag", "bearer"]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let server = spec_from(
        r#"{"server_id":"srv-1","name":"ID JAG","transport":"http","url":"https://mcp.example.com/mcp","auth_type":"oauth2_id_jag","token_exchange_endpoint":"https://idp.example.com/org/token","id_jag_resource_token_endpoint":"https://upstream.example.com/token","client_id":"client-1","client_secret":"secret-1","audience":"","id_jag_resource":"","scopes":[]}"#,
    );
    let _credential = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &server)
        .await
        .unwrap();
    let calls = endpoint.calls();
    let first = &calls[0];
    assert_eq!(param(first, "audience"), None);
    assert_eq!(param(first, "resource"), None);
}

#[tokio::test]
async fn id_jag_reuses_an_injected_token_cache() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["jag", "bearer"]));
    let cache = Arc::new(ExchangedTokenCache::default());
    let server = standard_spec();
    let alice = subject("org-1", "user-1", None);
    let first = provider(
        Collaborators::default()
            .with_exchanged_tokens(Arc::clone(&cache))
            .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
            .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint))),
    );
    let _one = first.resolve_credentials(&alice, &server).await.unwrap();
    let second = provider(
        Collaborators::default()
            .with_exchanged_tokens(cache)
            .with_sso_assertion_store(Arc::new(FakeAssertions::storing("id-token", None)))
            .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint))),
    );
    let _two = second.resolve_credentials(&alice, &server).await.unwrap();
    assert_eq!(endpoint.calls().len(), 2);
}

#[tokio::test]
async fn id_jag_requires_an_identified_caller() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&[]));
    let resolver = stored_assertion_resolver(Arc::clone(&endpoint));
    let error = failure(
        resolver
            .resolve_credentials(&subject("org-1", "", None), &standard_spec())
            .await,
    );
    assert_eq!(
        error,
        CredError::PreconditionRequired(MISSING_CALLER.to_owned())
    );
    assert!(endpoint.calls().is_empty());
}

#[tokio::test]
async fn id_jag_requires_a_stored_assertion() {
    let collaborators =
        Collaborators::default().with_sso_assertion_store(Arc::new(FakeAssertions::missing()));
    let error = failure(
        provider(collaborators)
            .resolve_credentials(&subject("org-1", "user-1", None), &standard_spec())
            .await,
    );
    assert_eq!(
        error,
        CredError::PreconditionRequired(NO_ASSERTION.to_owned())
    );
}

#[tokio::test]
async fn id_jag_rejects_an_expired_assertion() {
    let endpoint = Arc::new(FakeEndpoint::tokens(&["jag", "bearer"]));
    let clock = Arc::new(FakeClock::at(FIXTURE_NOW));
    let collaborators = Collaborators::default()
        .with_sso_assertion_store(Arc::new(FakeAssertions::storing(
            "id-token",
            Some(instant(ASSERTION_EXPIRY)),
        )))
        .with_token_endpoint(shared_endpoint(Arc::clone(&endpoint)))
        .with_wall_clock(shared_clock(Arc::clone(&clock)));
    let resolver = provider(collaborators);
    let fresh = resolver
        .resolve_credentials(&subject("org-1", "user-1", None), &standard_spec())
        .await
        .unwrap();
    assert_eq!(bearer(&fresh), "bearer");
    clock.advance_to(ASSERTION_EXPIRY);
    let error = failure(
        resolver
            .resolve_credentials(&subject("org-1", "user-1", None), &standard_spec())
            .await,
    );
    assert_eq!(
        error,
        CredError::PreconditionRequired(EXPIRED_ASSERTION.to_owned())
    );
    assert_eq!(endpoint.calls().len(), 2);
}

#[tokio::test]
async fn id_jag_maps_a_store_outage_to_503() {
    let collaborators =
        Collaborators::default().with_sso_assertion_store(Arc::new(FakeAssertions::unavailable()));
    let error = failure(
        provider(collaborators)
            .resolve_credentials(&subject("org-1", "user-1", None), &standard_spec())
            .await,
    );
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(STORE_OUTAGE.to_owned())
    );
}
