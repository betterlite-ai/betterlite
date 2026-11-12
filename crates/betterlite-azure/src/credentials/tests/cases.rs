use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret as _, SecretString};

use betterlite_config::sections::azure_credentials::DeploymentSettings;

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::headers::{
    ClientAuthParams, HeaderLookup, get_azure_request_auth_headers, process_azure_headers,
    redact_azure_auth_headers,
};
use crate::credentials::selection::{AzureCredentialRequest, AzureTokenError, get_azure_ad_token};
use crate::credentials::source::{CallableTokenProvider, SourceFailure};

use super::support::{FixtureSources, Outcome, map, pairs, params, scripted};

fn run(request: &AzureCredentialRequest<'_>) -> Result<Option<String>, AzureTokenError> {
    let token = get_azure_ad_token(request)?;
    Ok(token.map(|value| value.expose_secret().to_owned()))
}

fn failure(class: &str, message: &str) -> SourceFailure {
    SourceFailure {
        class: class.to_owned(),
        status: None,
        message: message.to_owned(),
    }
}

fn provider(outcome: Outcome) -> (Arc<dyn CallableTokenProvider>, Arc<Mutex<usize>>) {
    let calls = Arc::new(Mutex::new(0));
    (scripted(outcome, &calls), calls)
}

fn counted(calls: &Arc<Mutex<usize>>) -> usize {
    *calls.lock().unwrap()
}

pub(super) fn static_ad_token() {
    explicit_static_token();
    absent_token();
    provider_overrides_static_token();
    empty_provider_token_accepted();
    non_string_provider_token();
    provider_errors();
}

fn explicit_static_token() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_ad_token = Some(SecretString::from("explicit-token"));
    let request = super::support::request(&settings, false, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("explicit-token"));
    assert!(sources.calls().is_empty());
}

fn absent_token() {
    let sources = FixtureSources::new();
    let settings = params();
    let request = super::support::request(&settings, false, None, &sources);
    assert_eq!(run(&request).unwrap(), None);
}

fn provider_overrides_static_token() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_ad_token = Some(SecretString::from("explicit-token"));
    let (caller, calls) = provider(Outcome::Token("provider-token".to_owned()));
    let request = super::support::request(&settings, false, Some(caller), &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("provider-token"));
    assert_eq!(counted(&calls), 1);
}

fn empty_provider_token_accepted() {
    let sources = FixtureSources::new();
    let settings = params();
    let (caller, calls) = provider(Outcome::Token(String::new()));
    let request = super::support::request(&settings, false, Some(caller), &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some(""));
    assert_eq!(counted(&calls), 1);
}

fn non_string_provider_token() {
    let sources = FixtureSources::new();
    let settings = params();
    let (caller, _calls) = provider(Outcome::NonText("NoneType".to_owned()));
    let request = super::support::request(&settings, false, Some(caller), &sources);
    let error = run(&request).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Azure AD token must be a string, got NoneType"
    );
    assert_eq!(error.class(), "TypeError");
}

fn provider_errors() {
    let cases = [
        ("TypeError", "provider type", "TypeError", "provider type"),
        (
            "ClientAuthenticationError",
            "boom",
            "RuntimeError",
            "Failed to get Azure AD token: boom",
        ),
    ];
    for (raised, message, class, text) in cases {
        let sources = FixtureSources::new();
        let settings = params();
        let (caller, _calls) = provider(Outcome::Raise {
            class: raised.to_owned(),
            message: message.to_owned(),
        });
        let request = super::support::request(&settings, false, Some(caller), &sources);
        let error = run(&request).unwrap_err();
        assert_eq!(error.class(), class);
        assert_eq!(error.to_string(), text);
    }
}

pub(super) fn client_secret_precedence() {
    client_secret_from_explicit_values();
    static_token_does_not_suppress_client_secret();
    existing_provider_suppresses_sources();
    client_secret_precedes_username_password();
}

fn client_secret_params() -> DeploymentSettings {
    let mut settings = params();
    settings.tenant_id = Some("tenant".to_owned());
    settings.client_id = Some("client".to_owned());
    settings.client_secret = Some(SecretString::from("secret"));
    settings
}

fn username_params() -> DeploymentSettings {
    let mut settings = params();
    settings.client_id = Some("client".to_owned());
    settings.azure_username = Some("user".to_owned());
    settings.azure_password = Some(SecretString::from("password"));
    settings
}

fn single_source_call(settings: &DeploymentSettings, token: &str, call: &str) {
    let sources = FixtureSources::new();
    let request = super::support::request(settings, false, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some(token));
    assert_eq!(
        sources.calls(),
        vec![format!("{call}:{}", settings.azure_scope)]
    );
}

fn client_secret_from_explicit_values() {
    let settings = client_secret_params();
    single_source_call(
        &settings,
        "client-secret-token",
        "client_secret:tenant:client:secret",
    );
}

fn static_token_does_not_suppress_client_secret() {
    let sources = FixtureSources::new();
    let mut settings = client_secret_params();
    settings.azure_ad_token = Some(SecretString::from("static-token"));
    let request = super::support::request(&settings, false, None, &sources);
    assert_eq!(
        run(&request).unwrap().as_deref(),
        Some("client-secret-token")
    );
    assert_eq!(sources.calls().len(), 1);
}

fn existing_provider_suppresses_sources() {
    let mut settings = client_secret_params();
    settings.azure_username = Some("user".to_owned());
    settings.azure_password = Some(SecretString::from("password"));
    let sources = FixtureSources::new();
    let (caller, calls) = provider(Outcome::Token("caller-token".to_owned()));
    let request = super::support::request(&settings, false, Some(caller), &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("caller-token"));
    assert!(sources.calls().is_empty());
    assert_eq!(counted(&calls), 1);
}

fn client_secret_precedes_username_password() {
    let mut settings = client_secret_params();
    settings.azure_username = Some("user".to_owned());
    settings.azure_password = Some(SecretString::from("password"));
    single_source_call(
        &settings,
        "client-secret-token",
        "client_secret:tenant:client:secret",
    );
}

pub(super) fn username_password() {
    username_password_from_explicit_values();
    username_password_needs_client_id();
}

fn username_password_from_explicit_values() {
    let settings = username_params();
    single_source_call(
        &settings,
        "username-token",
        "username_password:client:user:password:None",
    );
}

fn username_password_needs_client_id() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_username = Some("user".to_owned());
    settings.azure_password = Some(SecretString::from("password"));
    let request = super::support::request(&settings, false, None, &sources);
    assert_eq!(run(&request).unwrap(), None);
    assert!(sources.calls().is_empty());
}

pub(super) fn refresh_fallback() {
    refresh_uses_resolved_scope();
    refresh_value_error_keeps_static_token();
    refresh_value_error_falls_back_to_default();
    refresh_other_error_propagates();
    refresh_disabled_does_nothing();
    oidc_runs_alongside_provider();
    oidc_replaces_token();
    refresh_skipped_with_provider();
    refresh_skipped_after_oidc();
}

fn refresh_skipped_with_provider() {
    let sources = FixtureSources::new();
    let settings = params();
    let (caller, _calls) = provider(Outcome::Token("caller-token".to_owned()));
    let request = super::support::request(&settings, true, Some(caller), &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("caller-token"));
    assert!(sources.calls().is_empty());
}

fn refresh_skipped_after_oidc() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_ad_token = Some(SecretString::from("oidc/assertion"));
    settings.client_id = Some("client".to_owned());
    settings.tenant_id = Some("tenant".to_owned());
    let request = super::support::request(&settings, true, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("oidc-token"));
    assert_eq!(sources.calls().len(), 1);
}

fn refresh_uses_resolved_scope() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_scope = "explicit-scope".to_owned();
    let request = super::support::request(&settings, true, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("refresh-token"));
    assert_eq!(
        sources.calls(),
        vec!["sdk_refresh:explicit-scope".to_owned()]
    );
    assert_eq!(sources.provider_calls(), 1);
}

fn refresh_value_error_keeps_static_token() {
    let mut sources = FixtureSources::new();
    sources.sdk_refresh = Err(failure("ValueError", "invalid credential"));
    let mut settings = params();
    settings.azure_scope = "explicit-scope".to_owned();
    settings.azure_ad_token = Some(SecretString::from("static-token"));
    let request = super::support::request(&settings, true, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("static-token"));
    assert_eq!(
        sources.calls(),
        vec!["sdk_refresh:explicit-scope".to_owned()]
    );
}

fn refresh_value_error_falls_back_to_default() {
    let mut sources = FixtureSources::new();
    sources.sdk_refresh = Err(failure("ValueError", "invalid credential"));
    let mut settings = params();
    settings.azure_scope = "explicit-scope".to_owned();
    let request = super::support::request(&settings, true, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("default-token"));
    assert_eq!(
        sources.calls(),
        vec![
            "sdk_refresh:explicit-scope".to_owned(),
            "default_credential:explicit-scope".to_owned()
        ]
    );
}

fn refresh_other_error_propagates() {
    let mut sources = FixtureSources::new();
    sources.sdk_refresh = Err(failure("KeyError", "AZURE_CLIENT_ID"));
    let settings = params();
    let request = super::support::request(&settings, true, None, &sources);
    let error = run(&request).unwrap_err();
    assert_eq!(error.class(), "KeyError");
    assert_eq!(error.to_string(), "AZURE_CLIENT_ID");
}

fn refresh_disabled_does_nothing() {
    let sources = FixtureSources::new();
    let settings = params();
    let request = super::support::request(&settings, false, None, &sources);
    assert_eq!(run(&request).unwrap(), None);
    assert!(sources.calls().is_empty());
}

fn oidc_runs_alongside_provider() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_ad_token = Some(SecretString::from("oidc/assertion"));
    settings.client_id = Some("client".to_owned());
    settings.tenant_id = Some("tenant".to_owned());
    let (caller, _calls) = provider(Outcome::Token("caller-token".to_owned()));
    let request = super::support::request(&settings, false, Some(caller), &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("caller-token"));
    assert_eq!(
        sources.calls(),
        vec![format!(
            "oidc:oidc/assertion:client:tenant:{}",
            settings.azure_scope
        )]
    );
}

fn oidc_replaces_token() {
    let sources = FixtureSources::new();
    let mut settings = params();
    settings.azure_ad_token = Some(SecretString::from("oidc/assertion"));
    settings.client_id = Some("client".to_owned());
    settings.tenant_id = Some("tenant".to_owned());
    let request = super::support::request(&settings, false, None, &sources);
    assert_eq!(run(&request).unwrap().as_deref(), Some("oidc-token"));
    assert_eq!(sources.calls().len(), 1);
}

pub(super) fn refresh_provider_cache() {
    cache_reuses_provider_per_scope();
    cache_evicts_beyond_capacity();
}

fn cache_reuses_provider_per_scope() {
    let sources = FixtureSources::new();
    let mut cache = RefreshProviderCache::new(&sources);
    let first = cache.provider("scope-a").unwrap();
    let again = cache.provider("scope-a").unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    let other = cache.provider("scope-b").unwrap();
    assert!(!Arc::ptr_eq(&first, &other));
    assert_eq!(
        sources.calls(),
        vec![
            "sdk_refresh:scope-a".to_owned(),
            "sdk_refresh:scope-b".to_owned()
        ]
    );
}

fn cache_evicts_beyond_capacity() {
    let sources = FixtureSources::new();
    let mut cache = RefreshProviderCache::new(&sources);
    let first = cache.provider("scope-0").unwrap();
    for index in 1_u32..128_u32 {
        drop(cache.provider(&format!("scope-{index}")).unwrap());
    }
    assert_eq!(sources.calls().len(), 128);
    drop(cache.provider("scope-128").unwrap());
    assert_eq!(sources.calls().len(), 129);
    let recreated = cache.provider("scope-0").unwrap();
    assert!(!Arc::ptr_eq(&first, &recreated));
    assert_eq!(sources.calls().len(), 130);
    drop(cache.provider("scope-127").unwrap());
    assert_eq!(sources.calls().len(), 130);
}

pub(super) fn header_case_insensitive_preservation() {
    auth_headers_preserved();
    token_adds_authorization();
    invalid_provider_output_falls_through();
    api_key_fallback();
    provider_failure_propagates();
}

fn auth_headers_preserved() {
    let (caller, calls) = provider(Outcome::Token("provider-token".to_owned()));
    let client = ClientAuthParams {
        azure_ad_token: Some(SecretString::from("token")),
        azure_ad_token_provider: Some(caller),
        api_key: Some("key-1".to_owned()),
    };
    let api_key = pairs(&[("Api-Key", "existing"), ("X-Other", "value")]);
    assert_eq!(
        get_azure_request_auth_headers(&api_key, &client).unwrap(),
        api_key
    );
    let authorization = pairs(&[("AUTHORIZATION", "Bearer existing"), ("X-Other", "value")]);
    assert_eq!(
        get_azure_request_auth_headers(&authorization, &client).unwrap(),
        authorization
    );
    assert_eq!(counted(&calls), 0);
}

fn token_adds_authorization() {
    let plain = pairs(&[("X-Other", "value")]);
    let (caller, calls) = provider(Outcome::Token("provider-token".to_owned()));
    let from_token = ClientAuthParams {
        azure_ad_token: Some(SecretString::from("token")),
        azure_ad_token_provider: None,
        api_key: None,
    };
    let from_provider = ClientAuthParams {
        azure_ad_token: Some(SecretString::from("")),
        azure_ad_token_provider: Some(caller),
        api_key: Some("key-1".to_owned()),
    };
    for (client, bearer) in [(from_token, "token"), (from_provider, "provider-token")] {
        assert_eq!(
            get_azure_request_auth_headers(&plain, &client).unwrap(),
            pairs(&[
                ("X-Other", "value"),
                ("Authorization", &format!("Bearer {bearer}"))
            ])
        );
    }
    assert_eq!(counted(&calls), 1);
}

fn invalid_provider_output_falls_through() {
    let plain = pairs(&[("X-Other", "value")]);
    let (non_text, _first_calls) = provider(Outcome::NonText("NoneType".to_owned()));
    let client = ClientAuthParams {
        azure_ad_token: None,
        azure_ad_token_provider: Some(non_text),
        api_key: Some("key-1".to_owned()),
    };
    assert_eq!(
        get_azure_request_auth_headers(&plain, &client).unwrap(),
        pairs(&[("X-Other", "value"), ("api-key", "key-1")])
    );
    let (empty, _second_calls) = provider(Outcome::Token(String::new()));
    let empty_client = ClientAuthParams {
        azure_ad_token: None,
        azure_ad_token_provider: Some(empty),
        api_key: Some("key-1".to_owned()),
    };
    assert_eq!(
        get_azure_request_auth_headers(&plain, &empty_client).unwrap(),
        pairs(&[("X-Other", "value"), ("api-key", "key-1")])
    );
}

fn api_key_fallback() {
    let plain = pairs(&[("X-Other", "value")]);
    let client = ClientAuthParams {
        azure_ad_token: None,
        azure_ad_token_provider: None,
        api_key: Some("key-1".to_owned()),
    };
    assert_eq!(
        get_azure_request_auth_headers(&plain, &client).unwrap(),
        pairs(&[("X-Other", "value"), ("api-key", "key-1")])
    );
    let empty = ClientAuthParams {
        azure_ad_token: None,
        azure_ad_token_provider: None,
        api_key: Some(String::new()),
    };
    assert_eq!(
        get_azure_request_auth_headers(&plain, &empty).unwrap(),
        plain
    );
}

fn provider_failure_propagates() {
    let plain = pairs(&[("X-Other", "value")]);
    let (caller, _calls) = provider(Outcome::Raise {
        class: "TypeError".to_owned(),
        message: "provider type".to_owned(),
    });
    let client = ClientAuthParams {
        azure_ad_token: None,
        azure_ad_token_provider: Some(caller),
        api_key: Some("key-1".to_owned()),
    };
    let error = get_azure_request_auth_headers(&plain, &client).unwrap_err();
    assert_eq!(error.class, "TypeError");
}

pub(super) fn redacted_auth_headers() {
    let headers = pairs(&[
        ("Api-Key", "key-value"),
        ("AUTHORIZATION", "Bearer token"),
        ("X-Other", "value"),
    ]);
    assert_eq!(
        redact_azure_auth_headers(&headers),
        pairs(&[
            ("Api-Key", "***REDACTED***"),
            ("AUTHORIZATION", "***REDACTED***"),
            ("X-Other", "value"),
        ])
    );
}

pub(super) fn azure_response_headers() {
    response_header_prefix_and_ratelimit_copy();
    response_header_lookup_mode();
}

fn response_header_prefix_and_ratelimit_copy() {
    let canonical = pairs(&[
        ("x-ratelimit-limit-requests", "12"),
        ("x-ratelimit-remaining-requests", "11"),
        ("x-ratelimit-limit-tokens", "10"),
        ("x-ratelimit-remaining-tokens", "9"),
        ("Content-Type", "application/json"),
    ]);
    assert_eq!(
        process_azure_headers(&canonical, HeaderLookup::Exact),
        map(&[
            ("llm_provider-x-ratelimit-limit-requests", "12"),
            ("llm_provider-x-ratelimit-remaining-requests", "11"),
            ("llm_provider-x-ratelimit-limit-tokens", "10"),
            ("llm_provider-x-ratelimit-remaining-tokens", "9"),
            ("llm_provider-Content-Type", "application/json"),
            ("x-ratelimit-limit-requests", "12"),
            ("x-ratelimit-remaining-requests", "11"),
            ("x-ratelimit-limit-tokens", "10"),
            ("x-ratelimit-remaining-tokens", "9"),
        ])
    );
    let unrelated = pairs(&[("X-Other", "value")]);
    assert_eq!(
        process_azure_headers(&unrelated, HeaderLookup::CaseInsensitive),
        map(&[("llm_provider-X-Other", "value")])
    );
}

fn response_header_lookup_mode() {
    let mixed_case = pairs(&[("X-Ratelimit-Limit-Requests", "12")]);
    assert_eq!(
        process_azure_headers(&mixed_case, HeaderLookup::Exact),
        map(&[("llm_provider-X-Ratelimit-Limit-Requests", "12")])
    );
    assert_eq!(
        process_azure_headers(&mixed_case, HeaderLookup::CaseInsensitive),
        map(&[
            ("llm_provider-X-Ratelimit-Limit-Requests", "12"),
            ("x-ratelimit-limit-requests", "12"),
        ])
    );
}

pub(super) fn identity_isolation() {
    configured_identities_stay_separate();
    header_identities_stay_separate();
    cache_scopes_stay_separate();
}

fn configured_identities_stay_separate() {
    let mut first = params();
    first.tenant_id = Some("tenant-a".to_owned());
    first.client_id = Some("client-a".to_owned());
    first.client_secret = Some(SecretString::from("secret-a"));
    let mut second = first.clone();
    second.tenant_id = Some("tenant-b".to_owned());
    second.client_id = Some("client-b".to_owned());
    second.client_secret = Some(SecretString::from("secret-b"));
    let mut sources_a = FixtureSources::new();
    sources_a.client_secret = Ok(Outcome::Token("token-a".to_owned()));
    let mut sources_b = FixtureSources::new();
    sources_b.client_secret = Ok(Outcome::Token("token-b".to_owned()));
    let request_a = super::support::request(&first, false, None, &sources_a);
    let request_b = super::support::request(&second, false, None, &sources_b);
    assert_eq!(run(&request_a).unwrap().as_deref(), Some("token-a"));
    assert_eq!(run(&request_b).unwrap().as_deref(), Some("token-b"));
    assert!(sources_a.calls()[0].contains("client-a"));
    assert!(sources_b.calls()[0].contains("client-b"));
    assert!(!sources_a.calls()[0].contains("client-b"));
    assert!(!sources_b.calls()[0].contains("client-a"));
}

fn header_identities_stay_separate() {
    let headers = pairs(&[("X-Other", "value")]);
    let client_a = ClientAuthParams {
        azure_ad_token: Some(SecretString::from("token-a")),
        azure_ad_token_provider: None,
        api_key: Some("key-a".to_owned()),
    };
    let client_b = ClientAuthParams {
        azure_ad_token: None,
        azure_ad_token_provider: None,
        api_key: Some("key-b".to_owned()),
    };
    assert_eq!(
        get_azure_request_auth_headers(&headers, &client_a).unwrap(),
        pairs(&[("X-Other", "value"), ("Authorization", "Bearer token-a")])
    );
    assert_eq!(
        get_azure_request_auth_headers(&headers, &client_b).unwrap(),
        pairs(&[("X-Other", "value"), ("api-key", "key-b")])
    );
}

fn cache_scopes_stay_separate() {
    let sources = FixtureSources::new();
    let mut cache = RefreshProviderCache::new(&sources);
    let first = cache.provider("scope-a").unwrap();
    let second = cache.provider("scope-b").unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
}

pub(super) fn secret_safe_failure() {
    let mut sources = FixtureSources::new();
    sources.client_secret = Err(failure("ValueError", "invalid credential values"));
    let mut settings = params();
    settings.tenant_id = Some("tenant".to_owned());
    settings.client_id = Some("client".to_owned());
    settings.client_secret = Some(SecretString::from("fixture-client-secret"));
    settings.azure_password = Some(SecretString::from("fixture-password"));
    settings.azure_ad_token = Some(SecretString::from("fixture-ad-token"));
    let request = super::support::request(&settings, false, None, &sources);
    let error = run(&request).unwrap_err();
    assert_eq!(error.class(), "ValueError");
    for text in [format!("{error:?}"), error.to_string()] {
        assert!(!text.contains("fixture-client-secret"));
        assert!(!text.contains("fixture-password"));
        assert!(!text.contains("fixture-ad-token"));
    }
    let debug = format!("{settings:?}");
    assert!(!debug.contains("fixture-client-secret"));
    assert!(!debug.contains("fixture-password"));
    assert!(!debug.contains("fixture-ad-token"));
}
