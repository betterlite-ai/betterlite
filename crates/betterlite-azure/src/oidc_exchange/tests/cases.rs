use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::fixture::{
    ASSERTION, ASSERTION_ENV, ASSERTION_REF, AUTHORITY, CLIENT, Fixture, Reply, TENANT,
    default_input, form_map, form_value, input,
};
use crate::oidc_exchange::{CLIENT_ASSERTION_TYPE, DEFAULT_SCOPE, OidcExchangeError};

const TOKEN_BODY: &str = r#"{"access_token":"tok-value","expires_in":3600}"#;

#[rstest]
#[case::case_1_oidc_form_grant(oidc_form_grant)]
#[case::case_2_authority_tenant_path(authority_tenant_path)]
#[case::case_3_missing_client_tenant(missing_client_tenant)]
#[case::case_4_unresolved_oidc_token(unresolved_oidc_token)]
#[case::case_5_missing_access_token(missing_access_token)]
#[case::case_6_missing_expires_in(missing_expires_in)]
#[case::case_7_cached_unexpired_token(cached_unexpired_token)]
#[case::case_8_expired_token_refresh(expired_token_refresh)]
#[case::case_9_oidc_http_failure(oidc_http_failure)]
fn oidc_exchange_cases(#[case] scenario: fn()) {
    scenario();
}

fn oidc_form_grant() {
    default_scope_grant();
    explicit_scope_grant();
    empty_scope_grant();
}

fn default_scope_grant() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let token = fixture
        .exchange()
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(token.expose_secret(), "tok-value");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        format!("{AUTHORITY}/{TENANT}/oauth2/v2.0/token")
    );
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

fn explicit_scope_grant() {
    let scope = "https://management.azure.com/.default";
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    fixture
        .exchange()
        .token(
            &input(ASSERTION_REF, Some(CLIENT), Some(TENANT), Some(scope)),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(
        form_value(&fixture.requests()[0], "scope").as_deref(),
        Some(scope)
    );
}

fn empty_scope_grant() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    fixture
        .exchange()
        .token(
            &input(ASSERTION_REF, Some(CLIENT), Some(TENANT), Some("")),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(
        form_value(&fixture.requests()[0], "scope").as_deref(),
        Some("")
    );
}

fn authority_tenant_path() {
    default_authority();
    trailing_slash_authority();
    environment_identity();
    empty_environment_identity();
    explicit_beats_environment();
    empty_explicit_falls_back();
    empty_authority_environment();
}

fn default_authority() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    fixture
        .exchange()
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(
        fixture.requests()[0].url,
        "https://login.microsoftonline.com/tenant/oauth2/v2.0/token"
    );
}

fn trailing_slash_authority() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)])
        .with_environment(&[("AZURE_AUTHORITY_HOST", "https://login.contoso.test/")]);
    fixture
        .exchange()
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(
        fixture.requests()[0].url,
        "https://login.contoso.test//tenant/oauth2/v2.0/token"
    );
}

fn environment_identity() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    fixture
        .exchange()
        .token(&input(ASSERTION_REF, None, None, None), &fixture.resolver())
        .unwrap();
    let request = &fixture.requests()[0];
    assert_eq!(
        request.url,
        "https://login.microsoftonline.com/env-tenant/oauth2/v2.0/token"
    );
    assert_eq!(
        form_value(request, "client_id").as_deref(),
        Some("env-client")
    );
}

fn empty_environment_identity() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)])
        .with_environment(&[("AZURE_CLIENT_ID", ""), ("AZURE_TENANT_ID", "")]);
    fixture
        .exchange()
        .token(&input(ASSERTION_REF, None, None, None), &fixture.resolver())
        .unwrap();
    let request = &fixture.requests()[0];
    assert_eq!(
        request.url,
        "https://login.microsoftonline.com//oauth2/v2.0/token"
    );
    assert_eq!(form_value(request, "client_id").as_deref(), Some(""));
}

fn explicit_beats_environment() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    fixture
        .exchange()
        .token(
            &input(
                ASSERTION_REF,
                Some("explicit-client"),
                Some("explicit-tenant"),
                None,
            ),
            &fixture.resolver(),
        )
        .unwrap();
    let request = &fixture.requests()[0];
    assert_eq!(
        request.url,
        "https://login.microsoftonline.com/explicit-tenant/oauth2/v2.0/token"
    );
    assert_eq!(
        form_value(request, "client_id").as_deref(),
        Some("explicit-client")
    );
}

fn empty_explicit_falls_back() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    fixture
        .exchange()
        .token(
            &input(ASSERTION_REF, Some(""), Some(""), None),
            &fixture.resolver(),
        )
        .unwrap();
    let request = &fixture.requests()[0];
    assert_eq!(
        request.url,
        "https://login.microsoftonline.com/env-tenant/oauth2/v2.0/token"
    );
    assert_eq!(
        form_value(request, "client_id").as_deref(),
        Some("env-client")
    );
}

fn empty_authority_environment() {
    let fixture =
        Fixture::new(vec![Reply::ok(TOKEN_BODY)]).with_environment(&[("AZURE_AUTHORITY_HOST", "")]);
    fixture
        .exchange()
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(
        fixture.requests()[0].url,
        "/tenant/oauth2/v2.0/token".to_owned()
    );
}

fn missing_client_tenant() {
    missing_both_ids();
    missing_tenant_id();
    ids_checked_before_secret_resolution();
}

fn missing_both_ids() {
    let fixture = Fixture::new(Vec::new());
    let error = fixture.error_of(&input(ASSERTION_REF, None, None, None));
    assert!(matches!(&error, OidcExchangeError::MissingClientTenant));
    assert_eq!(error.status_code(), Some(422));
    assert!(fixture.requests().is_empty());
}

fn missing_tenant_id() {
    let fixture = Fixture::new(Vec::new());
    let error = fixture.error_of(&input(ASSERTION_REF, Some(CLIENT), None, None));
    assert!(matches!(&error, OidcExchangeError::MissingClientTenant));
}

fn ids_checked_before_secret_resolution() {
    let fixture = Fixture::new(Vec::new());
    let unresolved = fixture.error_of(&input("os.environ/MISSING", None, None, None));
    assert!(matches!(
        &unresolved,
        OidcExchangeError::MissingClientTenant
    ));
}

fn unresolved_oidc_token() {
    unresolved_reference();
    unresolved_plain_value();
    empty_resolved_token_is_sent();
}

fn unresolved_reference() {
    let fixture = Fixture::new(Vec::new())
        .with_environment(&[("AZURE_CLIENT_ID", CLIENT), ("AZURE_TENANT_ID", TENANT)]);
    let error = fixture.error_of(&input("os.environ/MISSING", None, None, None));
    assert!(matches!(&error, OidcExchangeError::UnresolvedOidcToken));
    assert_eq!(error.status_code(), Some(401));
    assert!(fixture.requests().is_empty());
}

fn unresolved_plain_value() {
    let fixture = Fixture::new(Vec::new())
        .with_environment(&[("AZURE_CLIENT_ID", CLIENT), ("AZURE_TENANT_ID", TENANT)]);
    let error = fixture.error_of(&input("plain-token", None, None, None));
    assert!(matches!(&error, OidcExchangeError::UnresolvedOidcToken));
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
    missing_access_token_field();
    null_access_token_field();
    access_token_checked_before_expires_in();
}

fn missing_access_token_field() {
    let fixture = Fixture::new(vec![Reply::ok(r#"{"expires_in":3600}"#)]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::AccessTokenMissing));
    assert_eq!(error.status_code(), Some(422));
    assert_eq!(fixture.requests().len(), 1);
}

fn null_access_token_field() {
    let fixture = Fixture::new(vec![Reply::ok(
        r#"{"access_token":null,"expires_in":3600}"#,
    )]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::AccessTokenMissing));
}

fn access_token_checked_before_expires_in() {
    let fixture = Fixture::new(vec![Reply::ok("{}")]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::AccessTokenMissing));
}

fn missing_expires_in() {
    missing_expires_in_field();
    null_expires_in_field();
    zero_expires_in_is_kept();
}

fn missing_expires_in_field() {
    let fixture = Fixture::new(vec![Reply::ok(r#"{"access_token":"tok"}"#)]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::ExpiresInMissing));
    assert_eq!(error.status_code(), Some(422));
}

fn null_expires_in_field() {
    let fixture = Fixture::new(vec![Reply::ok(
        r#"{"access_token":"tok","expires_in":null}"#,
    )]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::ExpiresInMissing));
}

fn zero_expires_in_is_kept() {
    // a zero lifetime is used as the cache TTL, and an expiry equal to the clock still reads
    let zero = Fixture::new(vec![Reply::ok(r#"{"access_token":"zero","expires_in":0}"#)]);
    let exchange = zero.exchange();
    let first = exchange.token(&default_input(), &zero.resolver()).unwrap();
    let second = exchange.token(&default_input(), &zero.resolver()).unwrap();
    assert_eq!(first.expose_secret(), "zero");
    assert_eq!(second.expose_secret(), "zero");
    assert_eq!(zero.requests().len(), 1);
}

fn cached_unexpired_token() {
    same_identity_is_cached();
    client_identity_is_separate();
    tenant_identity_is_separate();
    assertion_identity_is_separate();
}

fn same_identity_is_cached() {
    let fixture = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let exchange = fixture.exchange();
    let first = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    let second = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    assert_eq!(first.expose_secret(), "tok-value");
    assert_eq!(second.expose_secret(), "tok-value");
    assert_eq!(fixture.requests().len(), 1);
}

fn client_identity_is_separate() {
    let fixture = Fixture::new(vec![
        Reply::ok(r#"{"access_token":"one","expires_in":3600}"#),
        Reply::ok(r#"{"access_token":"two","expires_in":3600}"#),
    ]);
    let exchange = fixture.exchange();
    let first = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    let second = exchange
        .token(
            &input(ASSERTION_REF, Some("other-client"), Some(TENANT), None),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(first.expose_secret(), "one");
    assert_eq!(second.expose_secret(), "two");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        form_value(&requests[1], "client_id").as_deref(),
        Some("other-client")
    );
}

fn tenant_identity_is_separate() {
    let fixture = Fixture::new(vec![
        Reply::ok(r#"{"access_token":"one","expires_in":3600}"#),
        Reply::ok(r#"{"access_token":"two","expires_in":3600}"#),
    ]);
    let exchange = fixture.exchange();
    let first = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    let second = exchange
        .token(
            &input(ASSERTION_REF, Some(CLIENT), Some("other-tenant"), None),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(first.expose_secret(), "one");
    assert_eq!(second.expose_secret(), "two");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].url,
        "https://login.microsoftonline.com/other-tenant/oauth2/v2.0/token"
    );
}

fn assertion_identity_is_separate() {
    let fixture = Fixture::new(vec![
        Reply::ok(r#"{"access_token":"one","expires_in":3600}"#),
        Reply::ok(r#"{"access_token":"two","expires_in":3600}"#),
    ])
    .with_environment(&[("OTHER_ASSERTION", "other-assertion")]);
    let exchange = fixture.exchange();
    let first = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap();
    let second = exchange
        .token(
            &input(
                "os.environ/OTHER_ASSERTION",
                Some(CLIENT),
                Some(TENANT),
                None,
            ),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(first.expose_secret(), "one");
    assert_eq!(second.expose_secret(), "two");
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
    rejected_status_preserves_text();
    other_success_status_fails();
    server_status_fails();
    invalid_json_propagates();
}

fn rejected_status_preserves_text() {
    let fixture = Fixture::new(vec![Reply::status(400, r#"{"error":"invalid_client"}"#)]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::TokenStatus { .. }));
    assert_eq!(error.status_code(), Some(400));
    assert!(error.to_string().contains("invalid_client"));
}

fn other_success_status_fails() {
    let fixture = Fixture::new(vec![Reply::status(201, TOKEN_BODY)]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::TokenStatus { .. }));
    assert_eq!(error.status_code(), Some(201));
}

fn server_status_fails() {
    let fixture = Fixture::new(vec![Reply::status(500, "unexpected")]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::TokenStatus { .. }));
    assert_eq!(error.status_code(), Some(500));
    assert_eq!(error.to_string(), "unexpected");
}

fn invalid_json_propagates() {
    let fixture = Fixture::new(vec![Reply::ok("not json")]);
    let error = fixture.error_of(&default_input());
    assert!(matches!(&error, OidcExchangeError::TokenJson { .. }));
    assert_eq!(error.status_code(), None);

    let sequence = Fixture::new(vec![Reply::ok("[]")]);
    let not_an_object = sequence.error_of(&default_input());
    assert!(matches!(
        &not_an_object,
        OidcExchangeError::TokenJson { .. }
    ));
}
