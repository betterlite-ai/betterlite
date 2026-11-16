use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp_user_credentials::types::{CredentialError, OAuthCredentialPayload};
use crate::oauth_token_resolution::ports::{
    AccessTokenRequest, CachedUserToken, PrefetchedCredentials,
};
use crate::oauth_token_resolution::{
    TokenResolutionError, refresh, resolve, resolve_user_access_token,
    resolve_valid_user_oauth_token,
};
use crate::stored_cipher::DbCipher;

use super::fakes::{
    BindingCall, CacheCall, FakeBinding, FakeCache, FakePost, FakeServer, FakeTtl, Fixture,
};
use super::support::{
    FakeTable, TableMode, block_on, expires_at, exposed, now, oauth_row, payload, secret,
    text_response, warnings_during,
};

const REFRESH_URL: &str = "https://auth.example/token";

fn credential_text(access: &str, refresh: Option<&str>, expires: Option<&str>) -> String {
    let refresh_field = refresh.map_or_else(String::new, |token| {
        format!(r#","refresh_token":"{token}""#)
    });
    let expires_field = expires.map_or_else(String::new, |deadline| {
        format!(r#","expires_at":"{deadline}""#)
    });
    format!(r#"{{"type":"oauth2","access_token":"{access}"{refresh_field}{expires_field}}}"#)
}

fn credential(
    access: &str,
    refresh: Option<&str>,
    expires: Option<&str>,
) -> OAuthCredentialPayload {
    payload(&credential_text(access, refresh, expires))
}

fn live_credential() -> OAuthCredentialPayload {
    credential("db-token", Some("ref-1"), Some(&expires_at(300)))
}

fn row_for(cipher: &DbCipher, cred: &OAuthCredentialPayload) -> JsonData {
    oauth_row(
        cipher,
        "u1",
        "s1",
        &serde_json::to_string(cred).expect("the test credential serializes"),
    )
}

fn access_request<'req>(
    server: &'req FakeServer,
    user_id: Option<&'req str>,
    prefetched: Option<&'req PrefetchedCredentials>,
) -> AccessTokenRequest<'req, FakeServer> {
    AccessTokenRequest {
        server,
        user_id,
        prefetched,
        now: now(),
    }
}

fn resolve_valid(
    fixture: &Fixture,
    server: &FakeServer,
    cred: Option<&OAuthCredentialPayload>,
) -> Option<OAuthCredentialPayload> {
    block_on(resolve_valid_user_oauth_token(
        &fixture.services(),
        server,
        "u1",
        cred,
        now(),
    ))
    .expect("the resolution reports no error")
}

fn resolve_access(
    fixture: &Fixture,
    cache: &FakeCache,
    ttl: &FakeTtl,
    request: &AccessTokenRequest<'_, FakeServer>,
) -> Option<String> {
    block_on(resolve_user_access_token(
        &fixture.services(),
        cache,
        ttl,
        request,
    ))
    .map(|token| token.expose_secret().to_owned())
}

#[test]
fn resolve_valid_returns_none_without_a_credential() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").enforcing();
    assert!(resolve_valid(&fixture, &server, None).is_none());
    assert!(fixture.binding.calls().is_empty());
    assert!(fixture.table.reads().is_empty());
}

#[test]
fn resolve_valid_returns_a_live_credential_without_reading_the_store() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1");
    let live = live_credential();
    let resolved =
        resolve_valid(&fixture, &server, Some(&live)).expect("the live credential returns");
    assert_eq!(exposed(resolved.access_token.as_ref()), Some("db-token"));
    assert_eq!(resolved.expires_at, live.expires_at);
    assert!(fixture.table.reads().is_empty());
    assert!(fixture.builder.calls().is_empty());
    assert!(fixture.post.calls().is_empty());
}

#[test]
fn resolve_valid_returns_none_when_the_binding_proof_disagrees() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default().disagreeing();
    let server = FakeServer::new("s1").enforcing();
    let live = live_credential();
    assert!(resolve_valid(&fixture, &server, Some(&live)).is_none());
    assert!(fixture.post.calls().is_empty());
    assert!(matches!(
        fixture.binding.calls().as_slice(),
        [BindingCall::Proof { .. }]
    ));
}

#[test]
fn resolve_valid_checks_the_binding_before_returning_a_live_token() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default();
    let server = FakeServer::new("s1").enforcing();
    let mut live = live_credential();
    live.identity_binding_proof = Some("stored-proof".to_owned());
    assert!(resolve_valid(&fixture, &server, Some(&live)).is_some());
    assert_eq!(
        fixture.binding.calls(),
        vec![BindingCall::Proof {
            server_id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            stored_proof: Some("stored-proof".to_owned()),
        }]
    );
}

#[test]
fn resolve_valid_returns_none_for_a_grant_without_an_access_token() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").enforcing();
    let cred = payload(r#"{"type":"oauth2","refresh_token":"ref-1"}"#);
    assert!(resolve_valid(&fixture, &server, Some(&cred)).is_none());
    assert!(fixture.binding.calls().is_empty());
    assert!(fixture.builder.calls().is_empty());
}

#[test]
fn resolve_valid_returns_none_for_an_expired_token_without_a_refresh_token() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let cred = credential("old-token", None, Some(&expires_at(-10)));
    assert!(resolve_valid(&fixture, &server, Some(&cred)).is_none());
    assert!(fixture.builder.calls().is_empty());
}

#[rstest]
#[case::inside_the_buffer(30, true)]
#[case::one_second_inside(59, true)]
#[case::at_the_deadline(60, false)]
#[case::one_second_past(61, false)]
#[case::long_lived(300, false)]
fn resolve_valid_refreshes_only_inside_the_expiry_buffer(
    #[case] seconds: i64,
    #[case] refreshed: bool,
) {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let cred = credential("old-token", Some("ref-1"), Some(&expires_at(seconds)));
    let resolved = resolve_valid(&fixture, &server, Some(&cred)).expect("a credential returns");
    let expected = if refreshed { "new-token" } else { "old-token" };
    assert_eq!(exposed(resolved.access_token.as_ref()), Some(expected));
}

#[test]
fn resolve_valid_refreshes_inside_the_caller_supplied_expiry_buffer() {
    let mut fixture = Fixture::new();
    fixture.buffer = 120;
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let cred = credential("old-token", Some("ref-1"), Some(&expires_at(90)));
    let resolved = resolve_valid(&fixture, &server, Some(&cred)).expect("a credential returns");
    assert_eq!(exposed(resolved.access_token.as_ref()), Some("new-token"));
    assert_eq!(fixture.post.calls().len(), 1);
}

#[test]
fn resolve_valid_keeps_a_token_outside_the_caller_supplied_expiry_buffer() {
    let mut fixture = Fixture::new();
    fixture.buffer = 120;
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let cred = credential("old-token", Some("ref-1"), Some(&expires_at(121)));
    let resolved = resolve_valid(&fixture, &server, Some(&cred)).expect("a credential returns");
    assert_eq!(exposed(resolved.access_token.as_ref()), Some("old-token"));
    assert!(fixture.post.calls().is_empty());
}

#[test]
fn resolve_valid_returns_none_when_the_refresh_has_no_token_url() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1");
    let cred = credential("old-token", Some("ref-1"), Some(&expires_at(30)));
    assert!(resolve_valid(&fixture, &server, Some(&cred)).is_none());
    assert!(fixture.post.calls().is_empty());
}

#[test]
fn resolve_valid_returns_none_when_the_refreshed_credential_has_no_access_token() {
    let mut fixture = Fixture::new();
    let row = oauth_row(&fixture.cipher, "u1", "s1", r#"{"type":"oauth2"}"#);
    fixture.table = FakeTable::new(Vec::new()).reading(row);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let cred = credential("old-token", Some("ref-1"), Some(&expires_at(30)));
    assert!(resolve_valid(&fixture, &server, Some(&cred)).is_none());
    assert_eq!(fixture.post.calls().len(), 1);
}

#[test]
fn resolve_valid_reports_a_failing_store() {
    let mut fixture = Fixture::new();
    fixture.table = FakeTable::new(Vec::new()).with_mode(TableMode::FailWrites);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let cred = credential("old-token", Some("ref-1"), Some(&expires_at(30)));
    let error = block_on(resolve_valid_user_oauth_token(
        &fixture.services(),
        &server,
        "u1",
        Some(&cred),
        now(),
    ))
    .err()
    .expect("the store fails");
    assert!(matches!(
        error,
        TokenResolutionError::Credential(CredentialError::Table(McpTableError::MissingServerId))
    ));
}

#[test]
fn usable_drops_a_credential_without_a_usable_access_token() {
    assert!(refresh::usable(None).is_none());
    assert!(refresh::usable(Some(payload(r#"{"type":"oauth2"}"#))).is_none());
    assert!(refresh::usable(Some(payload(r#"{"type":"oauth2","access_token":""}"#))).is_none());
    let kept = refresh::usable(Some(credential("tok", None, None))).expect("the token is usable");
    assert_eq!(exposed(kept.access_token.as_ref()), Some("tok"));
}

#[test]
fn cached_token_carries_the_access_token_and_proof() {
    let cred =
        payload(r#"{"type":"oauth2","access_token":"tok","identity_binding_proof":"proof"}"#);
    let token = resolve::token(&cred).expect("the credential has a token");
    assert_eq!(exposed(Some(&token.access_token)), Some("tok"));
    assert_eq!(token.identity_binding_proof.as_deref(), Some("proof"));
}

#[test]
fn cached_token_is_absent_without_an_access_token() {
    assert!(resolve::token(&payload(r#"{"type":"oauth2"}"#)).is_none());
}

#[test]
fn resolve_access_returns_none_without_a_user_id() {
    let fixture = Fixture::new();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    assert!(resolve_access(&fixture, &cache, &ttl, &access_request(&server, None, None)).is_none());
    assert!(cache.calls().is_empty());
    assert!(fixture.table.reads().is_empty());
}

#[test]
fn resolve_access_returns_none_for_an_empty_user_id() {
    let fixture = Fixture::new();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some(""), None);
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(cache.calls().is_empty());
}

#[test]
fn resolve_access_returns_none_for_an_empty_server_id() {
    let fixture = Fixture::new();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("");
    let request = access_request(&server, Some("u1"), None);
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(cache.calls().is_empty());
}

#[test]
fn resolve_access_uses_the_plain_cache_for_a_server_without_binding() {
    let fixture = Fixture::new();
    let cache = FakeCache::holding("cached-token");
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("cached-token")
    );
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::Get { user_id, server_id }]
            if user_id == "u1" && server_id == "s1"
    ));
    assert!(fixture.table.reads().is_empty());
}

#[test]
fn resolve_access_uses_a_bound_token_whose_proof_matches() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default();
    let cache = FakeCache::holding_bound(CachedUserToken {
        access_token: secret("bound-token"),
        identity_binding_proof: Some("proof".to_owned()),
    });
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").enforcing();
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("bound-token")
    );
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::GetToken { .. }]
    ));
    assert_eq!(
        fixture.binding.calls(),
        vec![BindingCall::Proof {
            server_id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            stored_proof: Some("proof".to_owned()),
        }]
    );
    assert!(fixture.table.reads().is_empty());
}

#[test]
fn resolve_access_deletes_a_bound_token_whose_proof_disagrees() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default().answering_proofs(vec![false, true]);
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &live_credential())]);
    let cache = FakeCache::holding_bound(CachedUserToken {
        access_token: secret("stale-bound"),
        identity_binding_proof: Some("old-proof".to_owned()),
    });
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").enforcing();
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("db-token")
    );
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::GetToken { .. }, CacheCall::Delete { .. }, CacheCall::Set { access_token, .. }]
            if access_token == "db-token"
    ));
}

#[test]
fn resolve_access_reads_the_database_when_no_bound_token_is_cached() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default();
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &live_credential())]);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").enforcing();
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("db-token")
    );
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::GetToken { .. }, CacheCall::Set { .. }]
    ));
}

#[test]
fn resolve_access_bypasses_the_cache_with_prefetched_credentials() {
    let fixture = Fixture::new();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let prefetched = PrefetchedCredentials::from([("s1".to_owned(), live_credential())]);
    let request = access_request(&server, Some("u1"), Some(&prefetched));
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("db-token")
    );
    assert!(cache.calls().is_empty());
    assert!(fixture.table.reads().is_empty());
}

#[test]
fn resolve_access_returns_none_when_the_prefetched_map_has_no_entry() {
    let fixture = Fixture::new();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let prefetched = PrefetchedCredentials::new();
    let request = access_request(&server, Some("u1"), Some(&prefetched));
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(cache.calls().is_empty());
}

#[test]
fn resolve_access_returns_none_without_a_stored_row() {
    let fixture = Fixture::new();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(matches!(cache.calls().as_slice(), [CacheCall::Get { .. }]));
}

#[rstest]
#[case::absent(r#"{"type":"oauth2"}"#)]
#[case::empty(r#"{"type":"oauth2","access_token":""}"#)]
fn resolve_access_returns_none_for_a_stored_credential_without_a_token(#[case] stored: &str) {
    let mut fixture = Fixture::new();
    let row = oauth_row(&fixture.cipher, "u1", "s1", stored);
    fixture.table = FakeTable::new(vec![row]);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(matches!(cache.calls().as_slice(), [CacheCall::Get { .. }]));
}

#[test]
fn resolve_access_clears_the_cache_when_the_refresh_fails() {
    let mut fixture = Fixture::new();
    let expired = credential("old-token", Some("ref-1"), Some(&expires_at(-10)));
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &expired)]);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::Get { .. }, CacheCall::Delete { .. }]
    ));
}

#[test]
fn resolve_access_clears_the_cache_when_an_expired_token_cannot_refresh() {
    let mut fixture = Fixture::new();
    let expired = credential("old-token", None, Some(&expires_at(-10)));
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &expired)]);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let request = access_request(&server, Some("u1"), None);
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::Get { .. }, CacheCall::Delete { .. }]
    ));
    assert!(fixture.builder.calls().is_empty());
}

#[test]
fn resolve_access_clears_the_cache_even_when_the_credentials_were_prefetched() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default().disagreeing();
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").enforcing();
    let prefetched = PrefetchedCredentials::from([("s1".to_owned(), live_credential())]);
    let request = access_request(&server, Some("u1"), Some(&prefetched));
    assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::Delete { .. }]
    ));
}

#[test]
fn resolve_access_warns_and_returns_none_when_the_table_read_fails() {
    let mut fixture = Fixture::new();
    fixture.table = FakeTable::in_mode(TableMode::FailReads);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    let warnings = warnings_during(|| {
        assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("resolve_user_oauth_access_token: failed for user=u1 server=s1"));
}

#[test]
fn resolve_access_warns_and_returns_none_when_the_store_fails() {
    let mut fixture = Fixture::new();
    let expired = credential("old-token", Some("ref-1"), Some(&expires_at(-10)));
    let table = vec![row_for(&fixture.cipher, &expired)];
    fixture.table = FakeTable::new(table).with_mode(TableMode::FailWrites);
    fixture.post = FakePost::returning(text_response(&[("access_token", "new-token")]));
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let request = access_request(&server, Some("u1"), None);
    let warnings = warnings_during(|| {
        assert!(resolve_access(&fixture, &cache, &ttl, &request).is_none());
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("resolve_user_oauth_access_token: failed for user=u1 server=s1"));
}

#[test]
fn resolve_access_warms_the_cache_with_the_remaining_lifetime() {
    let mut fixture = Fixture::new();
    let mut live = live_credential();
    live.identity_binding_proof = Some("proof-7".to_owned());
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &live)]);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(55);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("db-token")
    );
    assert_eq!(ttl.seen(), vec![("s1".to_owned(), Some(300))]);
    assert!(matches!(
        cache.calls().as_slice(),
        [CacheCall::Get { .. }, CacheCall::Set { access_token, identity_binding_proof, ttl_seconds, .. }]
            if access_token == "db-token"
                && identity_binding_proof.as_deref() == Some("proof-7")
                && *ttl_seconds == 55
    ));
}

#[test]
fn resolve_access_warms_without_a_lifetime_for_a_token_without_expiry() {
    let mut fixture = Fixture::new();
    let forever = credential("db-token", Some("ref-1"), None);
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &forever)]);
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(4200);
    let server = FakeServer::new("s1");
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("db-token")
    );
    assert_eq!(ttl.seen(), vec![("s1".to_owned(), None)]);
}

#[test]
fn resolve_access_warms_from_the_refreshed_credential() {
    let mut fixture = Fixture::new();
    let expired = credential("old-token", Some("ref-1"), Some(&expires_at(30)));
    fixture.table = FakeTable::new(vec![row_for(&fixture.cipher, &expired)]);
    fixture.post = FakePost::returning(text_response(&[
        ("access_token", "new-token"),
        ("expires_in", "3600"),
    ]));
    let cache = FakeCache::empty();
    let ttl = FakeTtl::returning(60);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let request = access_request(&server, Some("u1"), None);
    assert_eq!(
        resolve_access(&fixture, &cache, &ttl, &request).as_deref(),
        Some("new-token")
    );
    assert_eq!(ttl.seen(), vec![("s1".to_owned(), Some(3600))]);
}
