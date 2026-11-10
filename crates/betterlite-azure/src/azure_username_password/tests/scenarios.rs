use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::fixture::{
    CLIENT, Fixture, HOST, NOW, PASSWORD, Reply, USERNAME, form_map, input, input_for,
    instance_metadata, openid, scope_set, token, token_url, well_known_url,
};
use crate::azure_client_secret::ClientSecretError;
use crate::azure_client_secret::ResolvedCredentials;
use crate::azure_client_secret::token::token_request as client_credentials_request;
use crate::azure_client_secret::transport::HttpMethod;
use crate::azure_username_password::provider::{AccessToken, DEPRECATION_WARNING};
use crate::azure_username_password::token::PasswordTokenError;
use crate::azure_username_password::transport::HttpRequest;
use crate::azure_username_password::{DEFAULT_SCOPE, UsernamePasswordError, UsernamePasswordInput};

use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::logs::{Capture, CapturedEvent, FieldValue};

const CLAIMS: &str = r#""id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","tid":"tenant","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9""#;

pub(super) fn token_body() -> String {
    format!(r#"{{"access_token":"tok-1","expires_in":3600,{CLAIMS}}}"#)
}

pub(super) fn second_token_body() -> String {
    format!(r#"{{"access_token":"tok-2","expires_in":3600,{CLAIMS}}}"#)
}

fn granted(
    replies: Vec<Reply>,
    provider_input: &UsernamePasswordInput,
    host: Option<&str>,
) -> Fixture {
    let fixture = Fixture::new(replies);
    fixture
        .provider_on(provider_input, host)
        .unwrap()
        .token()
        .unwrap();
    fixture
}

fn realm_request(request: &HttpRequest, username: &str) {
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(
        request.url,
        format!("https://{HOST}/common/userrealm/{username}?api-version=1.0")
    );
    assert_eq!(
        request.headers,
        vec![("Accept".to_owned(), "application/json".to_owned())]
    );
    assert!(request.form.is_empty());
}

pub(super) fn password_grant_form() {
    let username = "user name+tag@example.com";
    let password = "p@ss word+\u{fc}n\u{ef}code";
    let fixture = granted(
        vec![openid(HOST), token(&token_body())],
        &input_for(username, password, None),
        None,
    );
    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url, well_known_url(HOST));
    realm_request(&requests[1], username);
    assert_eq!(requests[2].method, HttpMethod::Post);
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, token_url(HOST));
    assert_eq!(
        posts[0].headers,
        vec![("Accept".to_owned(), "application/json".to_owned())]
    );
    password_form_fields(&posts[0], username, password);
    assert_eq!(
        scope_set(&posts[0]),
        vec![
            DEFAULT_SCOPE.to_owned(),
            "offline_access".to_owned(),
            "openid".to_owned(),
            "profile".to_owned(),
        ]
    );
}

fn password_form_fields(request: &HttpRequest, username: &str, password: &str) {
    let form = form_map(request);
    assert_eq!(form.len(), 6);
    assert_eq!(form.get("grant_type").map(String::as_str), Some("password"));
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT));
    assert_eq!(form.get("username").map(String::as_str), Some(username));
    assert_eq!(form.get("password").map(String::as_str), Some(password));
    assert_eq!(form.get("client_info").map(String::as_str), Some("1"));
    assert!(!form.contains_key("client_secret"));
    assert!(!form.contains_key("tenant_id"));
}

pub(super) fn default_tenant() {
    default_host();
    custom_host();
    decorated_scope(
        None,
        &[DEFAULT_SCOPE, "offline_access", "openid", "profile"],
    );
    decorated_scope(Some(""), &["offline_access", "openid", "profile"]);
    decorated_scope(
        Some("https://management.azure.com/.default"),
        &[
            "https://management.azure.com/.default",
            "offline_access",
            "openid",
            "profile",
        ],
    );
}

fn default_host() {
    let fixture = granted(
        vec![openid(HOST), token(&token_body())],
        &input_for(USERNAME, PASSWORD, None),
        None,
    );
    let requests = fixture.transport.requests();
    assert_eq!(
        requests[0].url,
        "https://login.microsoftonline.com/organizations/v2.0/.well-known/openid-configuration"
    );
    assert_eq!(
        fixture.transport.posts()[0].url,
        "https://login.microsoftonline.com/organizations/oauth2/v2.0/token"
    );
}

fn custom_host() {
    let discovery =
        "https://login.contoso.test/organizations/v2.0/.well-known/openid-configuration";
    for host in ["https://login.contoso.test", "login.contoso.test"] {
        let fixture = granted(
            vec![
                instance_metadata(discovery),
                openid("login.contoso.test"),
                token(&token_body()),
            ],
            &input_for(USERNAME, PASSWORD, None),
            Some(host),
        );
        let requests = fixture.transport.requests();
        assert_eq!(
            requests[0].url,
            "https://login.microsoftonline.com/common/discovery/instance"
        );
        assert_eq!(requests[1].url, discovery);
        assert_eq!(
            requests[2].url,
            "https://login.contoso.test/common/userrealm/user@example.com?api-version=1.0"
        );
        assert_eq!(
            fixture.transport.posts()[0].url,
            "https://login.contoso.test/organizations/oauth2/v2.0/token"
        );
    }
}

fn decorated_scope(scope: Option<&str>, expected: &[&str]) {
    let fixture = granted(
        vec![openid(HOST), token(&token_body())],
        &input_for(USERNAME, PASSWORD, scope),
        None,
    );
    assert_eq!(scope_set(&fixture.transport.posts()[0]), expected);
}

fn token_error(status: u16, body: &str) -> String {
    let fixture = Fixture::new(vec![
        openid(HOST),
        Reply::status(status, "/oauth2/v2.0/token", body),
    ]);
    let error = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap_err();
    assert_eq!(fixture.transport.posts().len(), 1);
    error.to_string()
}

pub(super) fn invalid_grant() {
    descriptions();
    identity_token_failures();
    server_failures();
}

fn descriptions() {
    assert_eq!(
        token_error(
            400,
            r#"{"error":"invalid_grant","error_description":"AADSTS50126: bad password"}"#
        ),
        "Authentication failed: AADSTS50126: bad password"
    );
    assert_eq!(
        token_error(400, r#"{"error":"invalid_grant"}"#),
        "Authentication failed: invalid_grant"
    );
    assert_eq!(
        token_error(400, r#"{"error":"invalid_grant","error_description":""}"#),
        "Authentication failed: invalid_grant"
    );
}

fn identity_token_failures() {
    assert_eq!(
        token_error(200, r#"{"access_token":"tok","expires_in":3600}"#),
        "Failed to build AuthenticationRecord from unexpected identity token"
    );
    let missing_aud = r#"{"access_token":"tok","expires_in":3600,"id_token_claims":{"iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}"#;
    assert_eq!(
        token_error(200, missing_aud),
        "Failed to build AuthenticationRecord from unexpected identity token"
    );
    let malformed_client_info = r#"{"access_token":"tok","expires_in":3600,"id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"not-base64!!"}"#;
    assert_eq!(
        token_error(200, malformed_client_info),
        "Failed to build AuthenticationRecord from unexpected identity token"
    );
}

fn server_failures() {
    assert_eq!(
        token_error(500, "unexpected"),
        "Authentication failed: HTTP Error: 500"
    );
    let fixture = Fixture::new(vec![openid(HOST), Reply::failure("/oauth2/v2.0/token")]);
    let error = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap_err();
    assert_eq!(error.to_string(), "Authentication failed: connection reset");
    assert_eq!(fixture.transport.posts().len(), 1);
}

pub(super) fn token_response() {
    lazy();
    first_call();
    reuse_window();
    hard_expiry();
    renewal_sends_the_password_again();
    info_fields();
    record_claims();
    incomplete_results();
}

fn lazy() {
    let fixture = Fixture::new(Vec::new());
    let _provider = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap();
    assert!(fixture.transport.requests().is_empty());
}

fn first_call() {
    let fixture = Fixture::new(vec![openid(HOST), token(&token_body())]);
    let provider = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap();
    let access = provider.token().unwrap();
    assert_eq!(access.token.expose_secret(), "tok-1");
    assert_eq!(access.expires_on, NOW + 3600);
    assert_eq!(access.token_type, "Bearer");
    assert_eq!(access.refresh_on, None);
    assert_eq!(fixture.transport.posts().len(), 1);
    assert_eq!(access.record.authority, HOST);
    assert_eq!(access.record.client_id, CLIENT);
    assert_eq!(access.record.home_account_id, "uid.utid");
    assert_eq!(access.record.tenant_id, "tenant");
    assert_eq!(access.record.username, USERNAME);
}

fn reuse_window() {
    let cached = Fixture::new(vec![openid(HOST), token(&token_body())]);
    let provider = cached
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap();
    provider.token().unwrap();
    cached.clock.set(NOW + 3300);
    assert_eq!(provider.token().unwrap().token.expose_secret(), "tok-1");
    assert_eq!(cached.transport.posts().len(), 1);
    let renewed = Fixture::new(vec![
        openid(HOST),
        token(&token_body()),
        token(&second_token_body()),
    ]);
    let renewed_provider = renewed
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap();
    renewed_provider.token().unwrap();
    renewed.clock.set(NOW + 3301);
    assert_eq!(
        renewed_provider.token().unwrap().token.expose_secret(),
        "tok-2"
    );
    assert_eq!(renewed.transport.posts().len(), 2);
    assert_eq!(renewed.transport.requests().len(), 5);
}

fn hard_expiry() {
    let fixture = Fixture::new(vec![
        openid(HOST),
        token(&token_body()),
        token(&second_token_body()),
    ]);
    let provider = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap();
    provider.token().unwrap();
    fixture.clock.set(NOW + 3600);
    assert_eq!(provider.token().unwrap().token.expose_secret(), "tok-2");
    assert_eq!(fixture.transport.posts().len(), 2);
    assert_eq!(fixture.transport.requests().len(), 5);
}

fn renewal_sends_the_password_again() {
    let with_refresh = token_body().replace(
        r#""expires_in":3600"#,
        r#""expires_in":3600,"refresh_token":"rt-1""#,
    );
    let fixture = Fixture::new(vec![
        openid(HOST),
        token(&with_refresh),
        token(&second_token_body()),
    ]);
    let provider = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap();
    provider.token().unwrap();
    fixture.clock.set(NOW + 3600);
    provider.token().unwrap();
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 2);
    for post in posts {
        let form = form_map(&post);
        assert_eq!(form.get("grant_type").map(String::as_str), Some("password"));
        assert_eq!(form.get("password").map(String::as_str), Some(PASSWORD));
        assert!(!form.contains_key("refresh_token"));
    }
}

fn access_for(body: &str) -> AccessToken {
    let fixture = Fixture::new(vec![openid(HOST), token(body)]);
    fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap()
}

fn info_fields() {
    assert_eq!(
        access_for(r#"{"access_token":"tok","expires_in":7200,"token_type":"Custom","id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}"#).token_type,
        "Custom"
    );
    assert_eq!(
        access_for(r#"{"access_token":"tok","expires_in":3600,"refresh_in":120,"refresh_on":"1700009999","id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}"#).refresh_on,
        Some(NOW + 120)
    );
    assert_eq!(
        access_for(r#"{"access_token":"tok","expires_in":"1800","id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}"#).expires_on,
        NOW + 1800
    );
    assert_eq!(
        access_for(r#"{"access_token":"tok","expires_in":-5,"id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}"#).expires_on,
        NOW - 5
    );
}

fn record_claims() {
    sub_fallback();
    upn_fallback();
}

fn sub_fallback() {
    let no_client_info = r#"{"access_token":"tok","expires_in":3600,"id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"}}"#;
    let fixture = Fixture::new(vec![openid(HOST), token(no_client_info)]);
    let access = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap();
    assert_eq!(access.record.home_account_id, "subject");
    assert_eq!(access.record.tenant_id, "tenant/v2.0");
}

fn upn_fallback() {
    let upn = r#"{"access_token":"tok","expires_in":3600,"id_token_claims":{"aud":"client","iss":"https://x/t","sub":"subject","upn":"u@x"}}"#;
    let fixture = Fixture::new(vec![openid(HOST), token(upn)]);
    let access = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap();
    assert_eq!(access.record.authority, "x");
    assert_eq!(access.record.tenant_id, "t");
    assert_eq!(access.record.home_account_id, "subject");
    assert_eq!(access.record.username, "u@x");
}

fn incomplete_results() {
    let bare = Fixture::new(vec![openid(HOST), token(r#"{"access_token":"tok"}"#)]);
    let bare_error = bare
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap_err();
    assert!(matches!(
        bare_error,
        UsernamePasswordError::Token(PasswordTokenError::Record(_))
    ));
    let claims_without_expiry = r#"{"access_token":"tok","id_token_claims":{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","preferred_username":"user@example.com"},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}"#;
    let fixture = Fixture::new(vec![openid(HOST), token(claims_without_expiry)]);
    let expiry_error = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .unwrap_err();
    assert!(matches!(
        expiry_error,
        UsernamePasswordError::Token(PasswordTokenError::Shared(
            ClientSecretError::TokenExpiresInMissing
        ))
    ));
}

pub(super) fn password_scope_decoration() {
    let fixture = granted(
        vec![openid(HOST), token(&token_body())],
        &input_for(USERNAME, PASSWORD, None),
        None,
    );
    assert_eq!(
        scope_set(&fixture.transport.posts()[0]),
        vec![
            DEFAULT_SCOPE.to_owned(),
            "offline_access".to_owned(),
            "openid".to_owned(),
            "profile".to_owned(),
        ]
    );
    for reserved in ["openid", "profile", "offline_access"] {
        let reserved_fixture = Fixture::new(vec![openid(HOST)]);
        let error = reserved_fixture
            .provider(&input_for(USERNAME, PASSWORD, Some(reserved)))
            .unwrap()
            .token()
            .unwrap_err();
        assert!(matches!(error, UsernamePasswordError::ReservedScope));
        assert_eq!(
            error.to_string(),
            "Authentication failed: You cannot use any scope value that is reserved."
        );
        assert!(reserved_fixture.transport.posts().is_empty());
        assert_eq!(reserved_fixture.transport.requests().len(), 1);
    }
    client_credentials_untouched();
}

fn construction_events(provider_input: &UsernamePasswordInput) -> Vec<CapturedEvent> {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let fixture = Fixture::new(Vec::new());
    let built = with_default(subscriber, || fixture.provider(provider_input));
    built.unwrap();
    capture.events()
}

pub(super) fn username_debug_warning() {
    warning_and_debug_lines();
    empty_strings_still_set();
}

fn warning_and_debug_lines() {
    let events = construction_events(&input_for(
        "alice@example.com",
        "hunter2-fixture-value",
        None,
    ));
    assert_warning(&events);
    assert_debug_lines(&events);
    for event in &events {
        assert!(!event.message.contains("alice@example.com"));
        assert!(!event.message.contains("hunter2-fixture-value"));
    }
}

fn assert_warning(events: &[CapturedEvent]) {
    let warnings: Vec<&CapturedEvent> = events
        .iter()
        .filter(|event| event.level == tracing::Level::WARN)
        .collect();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].message, DEPRECATION_WARNING);
    assert_eq!(
        warnings[0].fields.get("category"),
        Some(&FieldValue::Text("DeprecationWarning".to_owned()))
    );
}

fn assert_debug_lines(events: &[CapturedEvent]) {
    let debug: Vec<&CapturedEvent> = events
        .iter()
        .filter(|event| event.level == tracing::Level::DEBUG)
        .collect();
    assert_eq!(debug.len(), 2);
    assert_eq!(
        debug[0].message,
        "client_id=client, azure_username=[set=true], azure_password=[set=true]"
    );
    assert_eq!(
        debug[0].fields.get("client_id"),
        Some(&FieldValue::Text(CLIENT.to_owned()))
    );
    assert_eq!(
        debug[0].fields.get("username_set"),
        Some(&FieldValue::Bool(true))
    );
    assert_eq!(
        debug[0].fields.get("password_set"),
        Some(&FieldValue::Bool(true))
    );
    assert!(debug[1].message.starts_with("token_provider "));
}

fn empty_strings_still_set() {
    let events = construction_events(&input_for("", "", None));
    let flags: Vec<&CapturedEvent> = events
        .iter()
        .filter(|event| event.message.starts_with("client_id="))
        .collect();
    assert_eq!(flags.len(), 1);
    assert_eq!(
        flags[0].fields.get("username_set"),
        Some(&FieldValue::Bool(true))
    );
    assert_eq!(
        flags[0].fields.get("password_set"),
        Some(&FieldValue::Bool(true))
    );
}

pub(super) fn username_constructor_failure() {
    invalid_authorities();
    empty_client_id();
    empty_authority_host();
}

fn invalid_authorities() {
    for authority in ["http://insecure", "ftp://login.example.com"] {
        let fixture = Fixture::new(Vec::new());
        let error = fixture
            .provider_on(&input_for(USERNAME, PASSWORD, None), Some(authority))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "'{authority}' is an invalid authority. The value must be a TLS protected (https) URL."
            )
        );
        assert!(fixture.transport.requests().is_empty());
    }
}

fn empty_client_id() {
    let fixture = Fixture::new(vec![openid(HOST), token(&token_body())]);
    let provider = fixture
        .provider(&input("", USERNAME, PASSWORD, None))
        .unwrap();
    provider.token().unwrap();
    let post = fixture.transport.posts().remove(0);
    assert!(!form_map(&post).contains_key("client_id"));
}

fn empty_authority_host() {
    let fixture = Fixture::new(Vec::new());
    let provider = fixture
        .provider_on(&input_for(USERNAME, PASSWORD, None), Some(""))
        .unwrap();
    let error = provider.token().unwrap_err();
    assert!(matches!(error, UsernamePasswordError::Authority(_)));
    assert!(fixture.transport.requests().is_empty());
}

fn client_credentials_untouched() {
    let credentials = ResolvedCredentials {
        tenant_id: "tenant".to_owned(),
        client_id: CLIENT.to_owned(),
        client_secret: SecretString::from(PASSWORD),
        scope: DEFAULT_SCOPE.to_owned(),
    };
    let request = client_credentials_request("https://login.example.com/token", &credentials);
    let form: BTreeMap<String, String> = request.form.iter().cloned().collect();
    assert_eq!(form.get("scope").map(String::as_str), Some(DEFAULT_SCOPE));
}
