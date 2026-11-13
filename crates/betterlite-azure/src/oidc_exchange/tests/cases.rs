use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::fixture::{
    ASSERTION, ASSERTION_ENV, ASSERTION_REF, CLIENT, Fixture, FixtureResolver, Recorded, Reply,
    TENANT, default_input, form_map, form_value, input,
};
use crate::oidc_exchange::{
    AzureOidcExchange, CLIENT_ASSERTION_TYPE, DEFAULT_SCOPE, OidcExchangeError, OidcExchangeInput,
};

const TOKEN_BODY: &str = r#"{"access_token":"tok-value","expires_in":3600}"#;
const DEFAULT_URL: &str = "https://login.microsoftonline.com/tenant/oauth2/v2.0/token";
const ENV_URL: &str = "https://login.microsoftonline.com/env-tenant/oauth2/v2.0/token";
const EMPTY_URL: &str = "https://login.microsoftonline.com//oauth2/v2.0/token";
const EXPLICIT_URL: &str = "https://login.microsoftonline.com/explicit-tenant/oauth2/v2.0/token";
const OTHER_TENANT_URL: &str = "https://login.microsoftonline.com/other-tenant/oauth2/v2.0/token";

#[rstest]
#[case::oidc_form_grant(oidc_form_grant)]
#[case::authority_tenant_path(authority_tenant_path)]
#[case::missing_client_tenant(missing_client_tenant)]
#[case::unresolved_oidc_token(unresolved_oidc_token)]
#[case::missing_access_token(missing_access_token)]
#[case::missing_expires_in(missing_expires_in)]
#[case::cached_unexpired_token(cached_unexpired_token)]
#[case::expired_token_refresh(expired_token_refresh)]
#[case::oidc_http_failure(oidc_http_failure)]
fn oidc_exchange_cases(#[case] scenario: fn()) {
    scenario();
}

fn oidc_form_grant() {
    source_form();
    scope_is_retained();
}

fn source_form() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let token = fixture
        .exchange()
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(token.expose_secret(), "tok-value");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, DEFAULT_URL);
    let form = form_map(&requests[0]);
    assert_eq!(form.len(), 5);
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT));
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(form.get("scope").map(String::as_str), Some(DEFAULT_SCOPE));
    assert_eq!(
        form.get("client_assertion_type").map(String::as_str),
        Some(CLIENT_ASSERTION_TYPE)
    );
    assert_eq!(
        form.get("client_assertion").map(String::as_str),
        Some(ASSERTION)
    );
    assert!(!form.contains_key("client_secret"));
}

fn scope_is_retained() {
    let explicit_scope = "https://management.azure.com/.default";
    let explicit = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let explicit_request = recorded_request(
        &explicit,
        &input(
            ASSERTION_REF,
            Some(CLIENT),
            Some(TENANT),
            Some(explicit_scope),
        ),
    );
    assert_eq!(
        form_value(&explicit_request, "scope").as_deref(),
        Some(explicit_scope)
    );

    let empty = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let empty_request = recorded_request(
        &empty,
        &input(ASSERTION_REF, Some(CLIENT), Some(TENANT), Some("")),
    );
    assert_eq!(form_value(&empty_request, "scope").as_deref(), Some(""));
}

fn authority_tenant_path() {
    authority_host_forms();
    client_tenant_precedence();
}

fn authority_host_forms() {
    let default = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let default_request = recorded_request(&default, &default_input());
    assert_eq!(default_request.url, DEFAULT_URL);

    let trailing = Fixture::new(vec![Reply::ok(TOKEN_BODY)])
        .with_environment(&[("AZURE_AUTHORITY_HOST", "https://login.contoso.test/")]);
    let trailing_request = recorded_request(&trailing, &default_input());
    assert_eq!(
        trailing_request.url,
        "https://login.contoso.test//tenant/oauth2/v2.0/token"
    );

    let empty =
        Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(&[("AZURE_AUTHORITY_HOST", "")]);
    let empty_request = recorded_request(&empty, &default_input());
    assert_eq!(empty_request.url, "/tenant/oauth2/v2.0/token");
}

fn client_tenant_precedence() {
    let environment = [
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ];
    let empty_env = [("AZURE_CLIENT_ID", ""), ("AZURE_TENANT_ID", "")];
    let env_request = request_for(&environment, &input(ASSERTION_REF, None, None, None));
    assert_eq!(env_request.url, ENV_URL);
    assert_eq!(
        form_value(&env_request, "client_id").as_deref(),
        Some("env-client")
    );
    let empty_request = request_for(&empty_env, &input(ASSERTION_REF, None, None, None));
    assert_eq!(empty_request.url, EMPTY_URL);
    assert_eq!(form_value(&empty_request, "client_id").as_deref(), Some(""));
    let explicit_request = request_for(
        &environment,
        &input(
            ASSERTION_REF,
            Some("explicit-client"),
            Some("explicit-tenant"),
            None,
        ),
    );
    assert_eq!(explicit_request.url, EXPLICIT_URL);
    assert_eq!(
        form_value(&explicit_request, "client_id").as_deref(),
        Some("explicit-client")
    );
    let fallback_request = request_for(
        &environment,
        &input(ASSERTION_REF, Some(""), Some(""), None),
    );
    assert_eq!(fallback_request.url, ENV_URL);
    assert_eq!(
        form_value(&fallback_request, "client_id").as_deref(),
        Some("env-client")
    );
}

fn missing_client_tenant() {
    let fixture = Fixture::new(Vec::new());
    let missing_both = fixture.error_of(&input(ASSERTION_REF, None, None, None));
    assert!(matches!(
        &missing_both,
        OidcExchangeError::MissingClientTenant
    ));
    assert_eq!(missing_both.status_code(), Some(422));
    let missing_tenant = fixture.error_of(&input(ASSERTION_REF, Some(CLIENT), None, None));
    assert!(matches!(
        &missing_tenant,
        OidcExchangeError::MissingClientTenant
    ));
    let missing_client = fixture.error_of(&input(ASSERTION_REF, None, Some(TENANT), None));
    assert!(matches!(
        &missing_client,
        OidcExchangeError::MissingClientTenant
    ));
    // an unresolvable assertion still reports the ids, so this check runs before secret resolution
    let unresolved = fixture.error_of(&input("os.environ/MISSING", None, None, None));
    assert!(matches!(
        &unresolved,
        OidcExchangeError::MissingClientTenant
    ));
    assert!(fixture.requests().is_empty());
}

fn unresolved_oidc_token() {
    let fixture = Fixture::new(Vec::new())
        .with_environment(&[("AZURE_CLIENT_ID", CLIENT), ("AZURE_TENANT_ID", TENANT)]);
    let missing_reference = fixture.error_of(&input("os.environ/MISSING", None, None, None));
    assert!(matches!(
        &missing_reference,
        OidcExchangeError::UnresolvedOidcToken
    ));
    assert_eq!(missing_reference.status_code(), Some(401));
    let plain_value = fixture.error_of(&input("plain-token", None, None, None));
    assert!(matches!(
        &plain_value,
        OidcExchangeError::UnresolvedOidcToken
    ));
    let empty_reference = fixture.error_of(&input("", None, None, None));
    assert!(matches!(
        &empty_reference,
        OidcExchangeError::UnresolvedOidcToken
    ));
    assert!(fixture.requests().is_empty());
    empty_resolved_token_is_sent();
}

fn empty_resolved_token_is_sent() {
    let fixture =
        Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(&[(ASSERTION_ENV, "")]);
    let token = fixture
        .exchange()
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(token.expose_secret(), "tok-value");
    assert_eq!(
        form_value(&fixture.requests()[0], "client_assertion").as_deref(),
        Some("")
    );
}

fn missing_access_token() {
    let missing = token_error(r#"{"expires_in":3600}"#);
    assert!(matches!(&missing, OidcExchangeError::AccessTokenMissing));
    assert_eq!(missing.status_code(), Some(422));
    let absent = token_error(r#"{"access_token":null,"expires_in":3600}"#);
    assert!(matches!(&absent, OidcExchangeError::AccessTokenMissing));
    let both = token_error("{}");
    assert!(matches!(&both, OidcExchangeError::AccessTokenMissing));
    // only null or a missing field is absent, so an empty token is sent and cached
    cached_token_is_reused(r#"{"access_token":"","expires_in":3600}"#, "");
}

fn missing_expires_in() {
    let missing = token_error(r#"{"access_token":"tok"}"#);
    assert!(matches!(&missing, OidcExchangeError::ExpiresInMissing));
    assert_eq!(missing.status_code(), Some(422));
    let absent = token_error(r#"{"access_token":"tok","expires_in":null}"#);
    assert!(matches!(&absent, OidcExchangeError::ExpiresInMissing));
    // a zero lifetime is used as the cache TTL, and an expiry equal to the clock still reads
    cached_token_is_reused(r#"{"access_token":"zero","expires_in":0}"#, "zero");
}

fn cached_unexpired_token() {
    cached_token_is_reused(TOKEN_BODY, "tok-value");
    distinct_identities_refetch();
}

fn cached_token_is_reused(body: &str, expected: &str) {
    let fixture = Fixture::new(vec![Reply::ok(body)]);
    let exchange = fixture.exchange();
    let resolver = fixture.resolver();
    let first = next_token(&exchange, &resolver, &default_input());
    let second = next_token(&exchange, &resolver, &default_input());
    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(fixture.requests().len(), 1);
}

fn distinct_identities_refetch() {
    client_and_tenant_identity_refetch();
    assertion_identity_refetch();
}

fn client_and_tenant_identity_refetch() {
    let fixture = Fixture::new(vec![
        token_reply("one"),
        token_reply("two"),
        token_reply("three"),
    ]);
    let exchange = fixture.exchange();
    let resolver = fixture.resolver();
    let base = next_token(&exchange, &resolver, &default_input());
    assert_eq!(base, "one");
    let other_client = next_token(
        &exchange,
        &resolver,
        &input(ASSERTION_REF, Some("other-client"), Some(TENANT), None),
    );
    assert_eq!(other_client, "two");
    let other_tenant = next_token(
        &exchange,
        &resolver,
        &input(ASSERTION_REF, Some(CLIENT), Some("other-tenant"), None),
    );
    assert_eq!(other_tenant, "three");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        form_value(&requests[1], "client_id").as_deref(),
        Some("other-client")
    );
    assert_eq!(requests[2].url, OTHER_TENANT_URL);
}

fn assertion_identity_refetch() {
    let fixture = Fixture::new(vec![token_reply("one"), token_reply("two")])
        .with_environment(&[("OTHER_ASSERTION", "other-assertion")]);
    let exchange = fixture.exchange();
    let resolver = fixture.resolver();
    let base = next_token(&exchange, &resolver, &default_input());
    assert_eq!(base, "one");
    let other = next_token(
        &exchange,
        &resolver,
        &input(
            "os.environ/OTHER_ASSERTION",
            Some(CLIENT),
            Some(TENANT),
            None,
        ),
    );
    assert_eq!(other, "two");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        form_value(&requests[1], "client_assertion").as_deref(),
        Some("other-assertion")
    );
}

fn expired_token_refresh() {
    let fixture = Fixture::new(vec![
        Reply::ok(r#"{"access_token":"first","expires_in":3600}"#),
        Reply::ok(r#"{"access_token":"second","expires_in":3600}"#),
    ]);
    let exchange = fixture.exchange();
    let first = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    fixture.clock.advance(3600.0);
    let at_expiry = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(at_expiry.expose_secret(), first.expose_secret());
    assert_eq!(fixture.requests().len(), 1);
    fixture.clock.advance(1.0);
    let refreshed = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(refreshed.expose_secret(), "second");
    assert_eq!(fixture.requests().len(), 2);
}

fn oidc_http_failure() {
    let rejected = Fixture::new(vec![Reply::status(400, r#"{"error":"invalid_client"}"#)]);
    let error = rejected.error_of(&default_input());
    assert_eq!(error.status_code(), Some(400));
    assert!(error.to_string().contains("invalid_client"));

    // any status other than 200 fails, another 2xx included
    let created = Fixture::new(vec![Reply::status(201, TOKEN_BODY)]);
    let created_error = created.error_of(&default_input());
    assert_eq!(created_error.status_code(), Some(201));

    let server = Fixture::new(vec![Reply::status(500, "unexpected")]);
    let server_error = server.error_of(&default_input());
    assert_eq!(server_error.status_code(), Some(500));
    assert_eq!(server_error.to_string(), "unexpected");

    let malformed = Fixture::new(vec![Reply::ok("not json")]);
    let malformed_error = malformed.error_of(&default_input());
    assert!(matches!(
        &malformed_error,
        OidcExchangeError::TokenJson { .. }
    ));
    assert_eq!(malformed_error.status_code(), None);

    let sequence = Fixture::new(vec![Reply::ok("[]")]);
    let sequence_error = sequence.error_of(&default_input());
    assert!(matches!(
        &sequence_error,
        OidcExchangeError::TokenJson { .. }
    ));
}

fn recorded_request(fixture: &Fixture, request: &OidcExchangeInput) -> Recorded {
    fixture
        .exchange()
        .token(request, &fixture.resolver())
        .unwrap();
    fixture.requests().remove(0)
}

fn request_for(environment: &[(&str, &str)], request: &OidcExchangeInput) -> Recorded {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(environment);
    recorded_request(&fixture, request)
}

fn next_token(
    exchange: &AzureOidcExchange,
    resolver: &FixtureResolver,
    request: &OidcExchangeInput,
) -> String {
    exchange
        .token(request, resolver)
        .unwrap()
        .expose_secret()
        .to_owned()
}

fn token_error(body: &str) -> OidcExchangeError {
    Fixture::new(vec![Reply::ok(body)]).error_of(&default_input())
}

fn token_reply(access_token: &str) -> Reply {
    Reply::ok(&format!(
        r#"{{"access_token":"{access_token}","expires_in":3600}}"#
    ))
}
